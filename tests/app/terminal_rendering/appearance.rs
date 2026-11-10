use super::*;

#[test]
fn terminal_theme_recolors_open_tabs_without_changing_light_sidebars() {
    use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
    use slint::{ComponentHandle, Model, Rgb8Pixel};

    struct Backend(Rc<MinimalSoftwareWindow>);
    impl slint::platform::Platform for Backend {
        fn create_window_adapter(
            &self,
        ) -> Result<Rc<dyn slint::platform::WindowAdapter>, slint::PlatformError> {
            Ok(self.0.clone())
        }
    }

    std::thread::spawn(|| {
        let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
        slint::platform::set_platform(Box::new(Backend(window.clone()))).unwrap();
        let ui = AppWindow::new().unwrap();
        let (width, height) = (1280, 760);
        ui.set_ui_font_family(resolve_ui_font_family());
        ui.set_connection_state("root@192.0.2.10".into());
        ui.set_resource_title("Server resources".into());
        ui.set_conn_state(1);
        ui.set_cpu_percent(0.07);
        ui.set_mem_percent(0.26);
        ui.set_mem_detail("0.5G/1.9G".into());
        ui.set_swap_detail("0G/2.0G".into());
        ui.set_app_version(env!("CARGO_PKG_VERSION").into());
        ui.set_color_themes(ModelRc::new(VecModel::from(vec![
            crate::ui::ColorThemeItem {
                id: "default".into(),
                name_en: "Default".into(),
                name_zh: "默认".into(),
                swatch: slint::Color::from_rgb_u8(0, 113, 227),
            },
        ])));
        ui.set_dock_panels(ModelRc::new(VecModel::from(vec![
            crate::ui::PanelGeomInfo {
                kind: "sidebar".into(),
                edge: "left".into(),
                x: 0.0,
                y: 0.0,
                w: 280.0,
                h: height as f32,
            },
        ])));
        ui.set_dock_central_x(288.0);
        ui.set_dock_central_y(0.0);
        ui.set_dock_central_w(width as f32 - 288.0);
        ui.set_dock_central_h(height as f32);
        let tab = TabInfo {
            id: "live".into(),
            title: "Demo SSH".into(),
            kind: "terminal".into(),
            connected: true,
            ..Default::default()
        };
        ui.set_tabs(ModelRc::new(VecModel::from(vec![tab.clone()])));
        ui.set_panes(ModelRc::new(VecModel::from(vec![PaneInfo {
            id: 0,
            x: 0.0,
            y: 0.0,
            w: width as f32 - 288.0,
            h: height as f32,
            active_id: "live".into(),
            focused: true,
            reserve_right: 240.0,
            tabs: ModelRc::new(VecModel::from(vec![tab])),
        }])));
        ui.set_active_tab_id("live".into());
        ui.set_terminals(ModelRc::new(VecModel::from(
            ["live", "history", "alt"]
                .map(|id| TerminalState {
                    id: id.into(),
                    sftp_collapsed: true,
                    status: "Connected root@192.0.2.10:22 (proxy socks5://127.0.0.1:10808)".into(),
                    ..Default::default()
                })
                .to_vec(),
        )));
        let bufs = TermBuffers::default();
        let mut live = make_buf(
            24,
            100,
            &[],
            &[
                "root@demo:~# uname -a",
                "Linux demo 6.8.0 x86_64 GNU/Linux",
                "root@demo:~# printf 'Hello MeatShell\\n'",
                "\x1b[32mHello MeatShell\x1b[0m",
                "root@demo:~# ",
            ],
            0,
        );
        live.output_highlight = OutputHighlightPreset::Off;
        let mut history = make_buf(3, 40, &[], &["live"], 2);
        let mut old_screen = vt100::Parser::new(3, 40, 0);
        old_screen.process(b"plain scrollback\r\nolder line");
        history.history = (0..2)
            .map(|r| build_row(old_screen.screen(), r, 40).into())
            .collect();
        let mut alt = make_buf(3, 40, &[], &[], 0);
        alt.parser.process(b"\x1b[?1049hplain alternate screen");
        for (id, buffer) in [("live", live), ("history", history), ("alt", alt)] {
            bufs.lock()
                .unwrap()
                .insert(id.into(), Arc::new(Mutex::new(buffer)));
        }
        let check_colors = |dark: bool| {
            for row in ui.get_terminals().iter() {
                assert_eq!(
                    term_buf(&bufs, row.id.as_str())
                        .unwrap()
                        .lock()
                        .unwrap()
                        .is_dark,
                    dark
                );
                let plain = row.spans.iter().find(|s| s.col == 0 && s.row == 0).unwrap();
                if row.id == "history" {
                    assert_eq!(plain.text.trim_end(), "plain scrollback");
                }
                assert_eq!(
                    plain.fg.as_argb_encoded(),
                    if dark { 0xffd4d4d4 } else { 0xff2d2d2f }
                );
            }
            assert_eq!(
                ui.get_term_cursor_color().as_argb_encoded(),
                if dark { 0xffd4d4d4 } else { 0xff2d2d2f }
            );
        };
        ui.set_terminal_theme("follow".into());
        apply_dark_mode(&ui, &bufs, false);
        check_colors(false);
        ui.set_terminal_theme("dark".into());
        refresh_terminal_colors(&ui, &bufs);
        assert!(!ui.get_dark_mode());
        assert!(ui.get_terminal_dark_mode());
        check_colors(true);
        // UI toggles and wallpaper-induced light/dark changes honour the override.
        apply_dark_mode(&ui, &bufs, true);
        apply_dark_mode(&ui, &bufs, false);
        check_colors(true);

        ui.show().unwrap();
        window.set_size(slint::PhysicalSize::new(width, height));
        let render = || {
            for _ in 0..5 {
                std::thread::sleep(std::time::Duration::from_millis(60));
                slint::platform::update_timers_and_animations();
            }
            let mut pixels = vec![Rgb8Pixel::default(); (width * height) as usize];
            window.request_redraw();
            window.draw_if_needed(|r| {
                r.render(&mut pixels, width as usize);
            });
            pixels
        };
        let pixels = render();
        let sidebar = pixels[(500 * width + 100) as usize];
        let terminal = pixels[(500 * width + 600) as usize];
        assert!(sidebar.r > 230 && sidebar.g > 230 && sidebar.b > 230);
        assert!(terminal.r < 35 && terminal.g < 35 && terminal.b < 35);
        if let Ok(path) = std::env::var("MEATSHELL_TERMINAL_THEME_SCREENSHOT") {
            let save = |path: &str, pixels: Vec<Rgb8Pixel>| {
                let bytes: Vec<u8> = pixels.iter().flat_map(|p| [p.r, p.g, p.b]).collect();
                image::save_buffer(path, &bytes, width, height, image::ColorType::Rgb8).unwrap();
            };
            save(&path, pixels);
            ui.set_interface_start_page("wallpaper".into());
            ui.set_interface_open(true);
            save(&format!("{path}.settings.png"), render());
            ui.set_interface_open(false);
        }
        ui.set_terminal_theme("light".into());
        apply_dark_mode(&ui, &bufs, true);
        assert!(!ui.get_terminal_dark_mode());
        check_colors(false);
        ui.set_terminal_theme("follow".into());
        refresh_terminal_colors(&ui, &bufs);
        check_colors(true);
        apply_dark_mode(&ui, &bufs, false);
        check_colors(false);
    })
    .join()
    .unwrap();
}
