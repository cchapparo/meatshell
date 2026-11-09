//! Atomic JSON imports with an explicit original-key association per FinalShell ID.
use super::*;
use crate::config::{OriginalPrivateKey, SessionImportBatch};
use std::path::PathBuf;

struct PendingKeyImport {
    batch: SessionImportBatch,
    keys: HashMap<String, OriginalPrivateKey>,
    index: usize,
    selected_path: Option<PathBuf>,
}

fn show_key_prompt(window: &AppWindow, pending: &PendingKeyImport) {
    let requests = pending.batch.key_requests();
    let request = &requests[pending.index];
    window.set_finalshell_key_progress(
        format!(
            "{} / {} · {} {}",
            pending.index + 1,
            requests.len(),
            request.connections.len(),
            t("个连接", "connections")
        )
        .into(),
    );
    window.set_finalshell_key_id(request.id.clone().into());
    window.set_finalshell_key_connections(request.connections.join("\n").into());
    window.set_finalshell_key_last(pending.index + 1 == requests.len());
    window.set_finalshell_key_file("".into());
    window.set_finalshell_key_passphrase("".into());
    window.set_finalshell_key_error("".into());
    window.set_finalshell_key_import_open(true);
}

fn close_key_prompt(window: &AppWindow) {
    window.set_finalshell_key_import_open(false);
    window.set_finalshell_key_passphrase("".into());
    window.set_finalshell_key_file("".into());
    window.set_finalshell_key_id("".into());
    window.set_finalshell_key_connections("".into());
    window.set_finalshell_key_error("".into());
}

fn finish_import(
    weak: &slint::Weak<AppWindow>,
    store: &Rc<RefCell<ConfigStore>>,
    model: &Rc<VecModel<SessionInfo>>,
    registry: &Rc<WindowRegistry<slint::Weak<AppWindow>>>,
    batch: &SessionImportBatch,
    keys: &HashMap<String, OriginalPrivateKey>,
) -> Result<()> {
    let (added, skipped) = store.borrow_mut().import_batch(batch, keys)?;
    sync_sessions_for_window(weak, &store.borrow(), model);
    if added > 0 {
        registry.broadcast_config_changed();
    }
    if let Some(window) = weak.upgrade() {
        window.set_ssh_import_hint(
            format!(
                "{} {} / {} {} · {} {}{}",
                t("已导入", "imported"),
                added,
                t("跳过重复", "skipped"),
                skipped,
                batch.file_count(),
                t("个文件", "files"),
                if keys.is_empty() {
                    String::new()
                } else {
                    format!(
                        " · {} {} {}",
                        t("已关联", "associated"),
                        keys.len(),
                        t("把私钥", "private keys")
                    )
                }
            )
            .into(),
        );
    }
    Ok(())
}

pub(super) fn wire_json_import(
    window: &AppWindow,
    store: Rc<RefCell<ConfigStore>>,
    sessions_model: Rc<VecModel<SessionInfo>>,
    registry: Rc<WindowRegistry<slint::Weak<AppWindow>>>,
) {
    let pending: Rc<RefCell<Option<PendingKeyImport>>> = Rc::new(RefCell::new(None));
    {
        let weak = window.as_weak();
        let pending = pending.clone();
        let store = store.clone();
        let model = sessions_model.clone();
        let registry = registry.clone();
        window.on_import_sessions(move || {
            let Some(paths) = DialogOwner::of_weak(&weak)
                .file()
                .add_filter("JSON", &["json"])
                .pick_files()
            else {
                return;
            };
            let Some(window) = weak.upgrade() else {
                return;
            };
            let result = (|| {
                let batch = SessionImportBatch::read(&paths)?;
                if batch.key_requests().is_empty() {
                    finish_import(&weak, &store, &model, &registry, &batch, &HashMap::new())?;
                } else {
                    let draft = PendingKeyImport {
                        batch,
                        keys: HashMap::new(),
                        index: 0,
                        selected_path: None,
                    };
                    show_key_prompt(&window, &draft);
                    *pending.borrow_mut() = Some(draft);
                }
                Ok::<_, anyhow::Error>(())
            })();
            if let Err(error) = result {
                window.set_ssh_import_hint(
                    format!("{}: {error:#}", t("导入失败", "import failed")).into(),
                );
            }
        });
    }
    wire_key_callbacks(window, pending, store, sessions_model, registry);
}

fn wire_key_callbacks(
    window: &AppWindow,
    pending: Rc<RefCell<Option<PendingKeyImport>>>,
    store: Rc<RefCell<ConfigStore>>,
    sessions_model: Rc<VecModel<SessionInfo>>,
    registry: Rc<WindowRegistry<slint::Weak<AppWindow>>>,
) {
    {
        let weak = window.as_weak();
        let pending = pending.clone();
        window.on_finalshell_key_select(move || {
            if pending.borrow().is_none() {
                return;
            }
            let Some(path) = DialogOwner::of_weak(&weak)
                .file()
                .set_title(t("选择原始私钥文件", "Select original private key"))
                .pick_file()
            else {
                return;
            };
            if let (Some(window), Some(draft)) = (weak.upgrade(), pending.borrow_mut().as_mut()) {
                window.set_finalshell_key_file(path.display().to_string().into());
                window.set_finalshell_key_passphrase("".into());
                window.set_finalshell_key_error("".into());
                draft.selected_path = Some(path);
            }
        });
    }
    {
        let weak = window.as_weak();
        let pending = pending.clone();
        window.on_finalshell_key_confirm(move || {
            let Some(window) = weak.upgrade() else {
                return;
            };
            let result = (|| {
                let mut state = pending.borrow_mut();
                let Some(draft) = state.as_mut() else {
                    return Ok(());
                };
                let path = draft
                    .selected_path
                    .as_ref()
                    .context(t("请先选择私钥文件", "select a private key first"))?;
                let key = OriginalPrivateKey::read(
                    path,
                    Secret::new(window.get_finalshell_key_passphrase().to_string()),
                )?;
                let id = draft.batch.key_requests()[draft.index].id.clone();
                draft.keys.insert(id, key);
                if draft.index + 1 < draft.batch.key_requests().len() {
                    draft.index += 1;
                    draft.selected_path = None;
                    show_key_prompt(&window, draft);
                } else {
                    finish_import(
                        &weak,
                        &store,
                        &sessions_model,
                        &registry,
                        &draft.batch,
                        &draft.keys,
                    )?;
                    *state = None;
                    close_key_prompt(&window);
                }
                Ok::<_, anyhow::Error>(())
            })();
            if let Err(error) = result {
                window.set_finalshell_key_error(format!("{error:#}").into());
            }
        });
    }
    {
        let weak = window.as_weak();
        window.on_finalshell_key_cancel(move || {
            *pending.borrow_mut() = None;
            if let Some(window) = weak.upgrade() {
                close_key_prompt(&window);
                window.set_ssh_import_hint(
                    t(
                        "已取消导入，未保存连接",
                        "import cancelled; no connections saved",
                    )
                    .into(),
                );
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
    use slint::platform::{Key, WindowEvent};

    struct Backend(Rc<MinimalSoftwareWindow>);
    impl slint::platform::Platform for Backend {
        fn create_window_adapter(
            &self,
        ) -> Result<Rc<dyn slint::platform::WindowAdapter>, slint::PlatformError> {
            Ok(self.0.clone())
        }
    }

    fn settle(host: &MinimalSoftwareWindow) {
        for _ in 0..2 {
            slint::platform::update_timers_and_animations();
            let mut pixels = vec![slint::Rgb8Pixel::default(); 1000 * 850];
            host.request_redraw();
            host.draw_if_needed(|r| {
                r.render(&mut pixels, 1000);
            });
            if let Some(path) = std::env::var_os("MEATSHELL_KEY_IMPORT_TEST_SCREENSHOT") {
                let mut bytes = b"P6\n1000 850\n255\n".to_vec();
                for pixel in pixels {
                    bytes.extend_from_slice(&[pixel.r, pixel.g, pixel.b]);
                }
                std::fs::write(path, bytes).unwrap();
            }
        }
        slint::platform::update_timers_and_animations();
    }

    #[test]
    fn key_import_dialog_retries_advances_cancels_and_commits_the_complete_batch() {
        std::thread::spawn(|| {
            let host = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
            slint::platform::set_platform(Box::new(Backend(host.clone()))).unwrap();
            let ui = AppWindow::new().unwrap();
            ui.show().unwrap();
            host.set_size(slint::PhysicalSize::new(1000, 850));
            settle(&host);
            let root = std::env::temp_dir().join(format!("ms-key-ui-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir(&root).unwrap();
            let config = root.join("sessions.json");
            let store = Rc::new(RefCell::new(crate::config::fixture_store(config.clone(), vec![])));
            let path = root.join("connections.json");
            std::fs::write(&path, serde_json::json!([
                {"conection_type":100,"authentication_type":2,"secret_key_id":"one","name":"First","host":"one.invalid"},
                {"conection_type":100,"authentication_type":2,"secret_key_id":"two","name":"Second","host":"two.invalid"}
            ]).to_string()).unwrap();
            let key_path = root.join("original-key");
            let key = ssh_key::PrivateKey::random(&mut rand::rngs::OsRng, ssh_key::Algorithm::Ed25519).unwrap();
            std::fs::write(&key_path, key.to_openssh(ssh_key::LineEnding::LF).unwrap().as_str()).unwrap();
            let pending = Rc::new(RefCell::new(None));
            let model = Rc::new(VecModel::default());
            wire_key_callbacks(&ui, pending.clone(), store.clone(), model.clone(), Rc::new(WindowRegistry::default()));
            let start = || {
                let draft = PendingKeyImport {
                    batch: SessionImportBatch::read(&[path.clone()]).unwrap(),
                    keys: HashMap::new(), index: 0, selected_path: None
                };
                show_key_prompt(&ui, &draft);
                *pending.borrow_mut() = Some(draft);
            };
            start();
            ui.invoke_finalshell_key_confirm();
            assert!(!ui.get_finalshell_key_error().is_empty());
            assert!(!config.exists());
            pending.borrow_mut().as_mut().unwrap().selected_path = Some(key_path.clone());
            ui.invoke_finalshell_key_confirm();
            assert_eq!(ui.get_finalshell_key_id(), "two");
            assert!(ui.get_finalshell_key_last());
            assert!(ui.get_finalshell_key_passphrase().is_empty());
            assert!(ui.get_finalshell_key_file().is_empty());
            assert!(!config.exists());
            ui.set_finalshell_key_passphrase("typed-synthetic-secret".into());
            settle(&host);
            for _ in 0..8 {
                host.dispatch_event(WindowEvent::KeyPressed { text: Key::Tab.into() });
                host.dispatch_event(WindowEvent::KeyReleased { text: Key::Tab.into() });
            }
            host.dispatch_event(WindowEvent::KeyPressed { text: Key::Escape.into() });
            host.dispatch_event(WindowEvent::KeyReleased { text: Key::Escape.into() });
            assert!(!ui.get_finalshell_key_import_open());
            assert!(ui.get_finalshell_key_passphrase().is_empty());
            assert!(pending.borrow().is_none());
            assert!(!config.exists());
            assert!(store.borrow().sessions().is_empty());
            start();
            for _ in 0..2 {
                pending.borrow_mut().as_mut().unwrap().selected_path = Some(key_path.clone());
                ui.invoke_finalshell_key_confirm();
            }
            assert!(pending.borrow().is_none());
            assert!(!ui.get_finalshell_key_import_open());
            assert_eq!(store.borrow().sessions().len(), 2);
            assert_eq!(model.iter().filter(|row| !row.builtin).count(), 2);
            assert!(config.exists());
            assert!(!std::fs::read_to_string(config).unwrap().contains("BEGIN OPENSSH PRIVATE KEY"));
            std::fs::remove_dir_all(root).unwrap();
        }).join().unwrap();
    }
}
