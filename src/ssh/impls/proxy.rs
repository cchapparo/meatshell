//! Outbound proxy support for SSH / SFTP connections (issue #7).
//!
//! Establishes the TCP stream to the target host **through a proxy**, then the
//! caller hands that stream to `russh::client::connect_stream`.  Both proxy
//! kinds end up as a transparent `TcpStream`:
//!
//! * **SOCKS5** (`socks5://` / `socks5h://`) via `tokio-socks`; after the
//!   handshake we unwrap to the inner `TcpStream`.
//! * **HTTP / HTTPS CONNECT** (`http://` / `https://`): we issue an HTTP
//!   `CONNECT host:port` and reuse the same socket as the tunnel.
//!
//! An enabled global SOCKS5 setting takes precedence for every SSH / SFTP
//! connection. Otherwise use the per-session proxy, then `ALL_PROXY` / `all_proxy`.

use anyhow::{anyhow, bail, Context, Result};
use base64::Engine as _;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use std::sync::RwLock;

use super::structs::{ProxyConfig, ProxyKind};
use crate::config::Secret;

// Shared by shell, SFTP, reconnects and automation, like the global keepalive.
struct ProxySettings {
    global: Option<ProxyConfig>,
    ssh_required: bool,
}

static PROXY_SETTINGS: RwLock<ProxySettings> = RwLock::new(ProxySettings {
    global: None,
    ssh_required: false,
});

pub fn set_global_socks5(proxy: Option<ProxyConfig>, ssh_required: bool) {
    *PROXY_SETTINGS.write().unwrap() = ProxySettings {
        global: proxy,
        ssh_required,
    };
}

/// Validate a host entered separately from its port, including IPv6 literals.
pub fn socks5_config(host: &str, port: u16) -> Result<ProxyConfig> {
    let host = host.trim();
    if host.is_empty() || port == 0 {
        bail!("SOCKS5 requires a host and a port between 1 and 65535");
    }
    let host = match host.parse::<std::net::IpAddr>() {
        Ok(ip) => ip.to_string(),
        Err(_) => match url::Host::parse(host).context("invalid SOCKS5 host")? {
            url::Host::Domain(name) => name,
            url::Host::Ipv4(ip) => ip.to_string(),
            url::Host::Ipv6(ip) => ip.to_string(),
        },
    };
    Ok(ProxyConfig {
        kind: ProxyKind::Socks5,
        host,
        port,
        auth: None,
    })
}

/// Resolve global SOCKS5 first, then the session proxy and environment.
pub fn resolve(session_proxy: &str) -> Option<ProxyConfig> {
    resolve_with_global(
        PROXY_SETTINGS.read().unwrap().global.as_ref(),
        session_proxy,
    )
}

/// Enforce the SSH policy against the same proxy snapshot used for connecting.
pub fn resolve_ssh(session_proxy: &str) -> Result<Option<ProxyConfig>> {
    let settings = PROXY_SETTINGS.read().unwrap();
    let proxy = resolve_with_global(settings.global.as_ref(), session_proxy);
    if settings.ssh_required && proxy.is_none() {
        bail!(crate::i18n::t(
            "已拒绝 SSH 直连：请配置全局代理、会话代理或 ALL_PROXY，或关闭“拒绝未使用代理的 SSH 连接”",
            "Direct SSH connection refused. Configure a global, session or ALL_PROXY proxy, or disable the proxy requirement",
        ));
    }
    Ok(proxy)
}

fn resolve_with_global(global: Option<&ProxyConfig>, session_proxy: &str) -> Option<ProxyConfig> {
    if let Some(proxy) = global {
        return Some(proxy.clone());
    }
    let s = session_proxy.trim();
    if !s.is_empty() {
        return parse(s);
    }
    for var in ["ALL_PROXY", "all_proxy"] {
        if let Ok(v) = std::env::var(var) {
            if !v.trim().is_empty() {
                return parse(v.trim());
            }
        }
    }
    None
}

/// Parse a proxy URL: `scheme://[user:pass@]host:port`.
fn parse(url: &str) -> Option<ProxyConfig> {
    let (scheme, rest) = url.split_once("://").unwrap_or(("socks5", url));
    let kind = match scheme.to_ascii_lowercase().as_str() {
        "socks5" | "socks5h" | "socks" => ProxyKind::Socks5,
        "http" | "https" => ProxyKind::Http,
        _ => return None,
    };
    // Optional userinfo before '@'.
    let (auth, hostport) = match rest.rsplit_once('@') {
        Some((userinfo, hp)) => {
            let (u, p) = userinfo.split_once(':').unwrap_or((userinfo, ""));
            (Some((u.to_string(), Secret::new(p))), hp)
        }
        None => (None, rest),
    };
    let hostport = hostport.trim_end_matches('/');
    let (host, port) = hostport.rsplit_once(':')?;
    let port: u16 = port.parse().ok()?;
    if host.is_empty() {
        return None;
    }
    Some(ProxyConfig {
        kind,
        host: host.to_string(),
        port,
        auth,
    })
}

/// Human-readable description of where we're connecting (for status messages).
pub fn describe(cfg: &ProxyConfig) -> String {
    let scheme = match cfg.kind {
        ProxyKind::Socks5 => "socks5",
        ProxyKind::Http => "http",
    };
    format!("{}://{}:{}", scheme, cfg.host, cfg.port)
}

/// Open a TCP stream to `target_host:target_port` through the proxy.
pub async fn connect(cfg: &ProxyConfig, target_host: &str, target_port: u16) -> Result<TcpStream> {
    match cfg.kind {
        ProxyKind::Socks5 => connect_socks5(cfg, target_host, target_port).await,
        ProxyKind::Http => connect_http(cfg, target_host, target_port).await,
    }
}

async fn connect_socks5(cfg: &ProxyConfig, host: &str, port: u16) -> Result<TcpStream> {
    use tokio_socks::tcp::Socks5Stream;
    let proxy = (cfg.host.as_str(), cfg.port);
    let target = (host, port);
    let stream = match &cfg.auth {
        Some((u, p)) => Socks5Stream::connect_with_password(proxy, target, u, p.as_str())
            .await
            .context("SOCKS5 proxy connect failed")?,
        None => Socks5Stream::connect(proxy, target)
            .await
            .context("SOCKS5 proxy connect failed")?,
    };
    // After the handshake the underlying socket is a transparent tunnel.
    Ok(stream.into_inner())
}

async fn connect_http(cfg: &ProxyConfig, host: &str, port: u16) -> Result<TcpStream> {
    let mut s = TcpStream::connect((cfg.host.as_str(), cfg.port))
        .await
        .with_context(|| format!("connect to HTTP proxy {}:{} failed", cfg.host, cfg.port))?;

    let mut req = format!("CONNECT {host}:{port} HTTP/1.1\r\nHost: {host}:{port}\r\n");
    if let Some((u, p)) = &cfg.auth {
        let token = base64::engine::general_purpose::STANDARD.encode(format!("{u}:{}", p.as_str()));
        req.push_str(&format!("Proxy-Authorization: Basic {token}\r\n"));
    }
    req.push_str("Proxy-Connection: keep-alive\r\n\r\n");
    s.write_all(req.as_bytes())
        .await
        .context("write CONNECT to proxy")?;

    // Read response headers up to the blank line, bounded.
    let mut buf = Vec::with_capacity(256);
    let mut byte = [0u8; 1];
    loop {
        let n = s.read(&mut byte).await.context("read proxy response")?;
        if n == 0 {
            bail!("proxy closed the connection during CONNECT");
        }
        buf.push(byte[0]);
        if buf.ends_with(b"\r\n\r\n") {
            break;
        }
        if buf.len() > 8192 {
            bail!("proxy CONNECT response too large");
        }
    }
    let head = String::from_utf8_lossy(&buf);
    let status_line = head.lines().next().unwrap_or("");
    // Expect "HTTP/1.x 200 ...".
    let ok = status_line
        .split_whitespace()
        .nth(1)
        .map(|c| c == "200")
        .unwrap_or(false);
    if !ok {
        return Err(anyhow!("proxy CONNECT rejected: {}", status_line.trim()));
    }
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn global_socks5_overrides_session_proxy() {
        let global = socks5_config("127.0.0.1", 1080).unwrap();
        for session in [
            "",
            "http://per-session.invalid:3128",
            "invalid://proxy:9999",
        ] {
            let resolved = resolve_with_global(Some(&global), session).unwrap();
            assert_eq!(resolved.kind, ProxyKind::Socks5);
            assert_eq!(resolved.host, "127.0.0.1");
            assert_eq!(resolved.port, 1080);
        }
        let resolved = resolve_with_global(None, "http://per-session.invalid:3128").unwrap();
        assert_eq!(resolved.kind, ProxyKind::Http);
        assert_eq!(resolved.host, "per-session.invalid");
    }

    #[test]
    fn global_socks5_validates_host_and_port() {
        for (input, expected) in [
            (" 127.0.0.1 ", "127.0.0.1"),
            ("proxy.example", "proxy.example"),
            ("::1", "::1"),
            ("[::1]", "::1"),
        ] {
            assert_eq!(socks5_config(input, 1080).unwrap().host, expected);
        }
        for host in [
            "",
            "   ",
            "socks5://127.0.0.1",
            "127.0.0.1:1080",
            "bad host",
            "user@proxy",
        ] {
            assert!(socks5_config(host, 1080).is_err(), "{host}");
        }
        assert!(socks5_config("127.0.0.1", 0).is_err());
    }

    #[tokio::test]
    async fn socks5_sends_target_hostname_to_proxy() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy = socks5_config("127.0.0.1", listener.local_addr().unwrap().port()).unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut greeting = [0; 3];
            stream.read_exact(&mut greeting).await.unwrap();
            assert_eq!(greeting, [5, 1, 0]);
            stream.write_all(&[5, 0]).await.unwrap();
            let mut request = [0; 5];
            stream.read_exact(&mut request).await.unwrap();
            assert_eq!(&request[..4], &[5, 1, 0, 3]);
            let mut hostname = vec![0; request[4] as usize];
            stream.read_exact(&mut hostname).await.unwrap();
            assert_eq!(hostname, b"server.example.invalid");
            let mut port = [0; 2];
            stream.read_exact(&mut port).await.unwrap();
            assert_eq!(u16::from_be_bytes(port), 2222);
            stream
                .write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 22])
                .await
                .unwrap();
            stream.write_all(b"SSH-2.0-fixture\r\n").await.unwrap();
        });
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            let mut stream = connect(&proxy, "server.example.invalid", 2222)
                .await
                .unwrap();
            let mut banner = [0; 17];
            stream.read_exact(&mut banner).await.unwrap();
            assert_eq!(&banner, b"SSH-2.0-fixture\r\n");
            server.await.unwrap();
        })
        .await
        .unwrap();
    }
}
