use super::*;

#[test]
fn bottom_panel_search_keeps_raw_commands_indices_and_group_headers() {
    let commands: ModelRc<QuickCmd> = ModelRc::new(VecModel::from(vec![
        QuickCmd {
            name: "first".into(),
            command: "echo first".into(),
            group: "Ops".into(),
            group_header: "Ops".into(),
            collapsed: true,
            orig_index: 4,
            ..Default::default()
        },
        QuickCmd {
            name: "容器状态".into(),
            command: "docker ps\necho done".into(),
            group: "Ops".into(),
            collapsed: true,
            orig_index: 9,
            ..Default::default()
        },
        QuickCmd {
            name: "last".into(),
            command: "pwd".into(),
            group: "Docker".into(),
            group_header: "Docker".into(),
            orig_index: 2,
            ..Default::default()
        },
    ]));
    let rows = bottom_command_model(&commands, "  DOCKER ");
    assert_eq!(rows.row_count(), 2);
    let row = rows.row_data(0).unwrap();
    assert_eq!(row.orig_index, 9);
    assert_eq!(row.command, "docker ps\necho done");
    assert_eq!(row.group_header, "Ops");
    assert!(!row.collapsed);
    assert!(commands.row_data(1).unwrap().collapsed);
    assert_eq!(rows.row_data(1).unwrap().group_header, "Docker");
    assert_eq!(bottom_command_model(&commands, "容器").row_count(), 1);
    assert_eq!(bottom_command_model(&commands, "unmatched").row_count(), 0);
    assert!(
        bottom_command_model(&commands, "")
            .row_data(0)
            .unwrap()
            .collapsed
    );
}

#[test]
fn bottom_panel_controller_restores_selection_and_never_broadcasts() {
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
    std::thread::spawn(|| {
        let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
        slint::platform::set_platform(Box::new(Backend(window.clone()))).unwrap();
        let ui = AppWindow::new().unwrap();
        let path =
            std::env::temp_dir().join(format!("meatshell-bottom-{}.json", uuid::Uuid::new_v4()));
        let store = Rc::new(RefCell::new(crate::config::fixture_store(
            path.clone(),
            vec![],
        )));
        wire_bottom_commands(&ui, store.clone());
        let layout_store = store.clone();
        ui.on_dock_layout_changed(move || {
            let _ = layout_store.borrow().bottom_panel_tab();
        });
        ui.set_quick_panel_open(true);
        ui.set_ui_font_family(resolve_ui_font_family());
        ui.set_dark_mode(false);
        ui.set_terminal_theme("dark".into());
        ui.set_conn_state(1);
        ui.set_cmd_bar_hidden(true);
        let tabs: Vec<TabInfo> = ["ssh", "other"]
            .map(|id| TabInfo {
                id: id.into(),
                title: "Demo SSH".into(),
                kind: "terminal".into(),
                connected: true,
                ..Default::default()
            })
            .to_vec();
        ui.set_tabs(ModelRc::new(VecModel::from(tabs.clone())));
        ui.set_terminals(ModelRc::new(VecModel::from(
            tabs.iter()
                .map(|t| TerminalState {
                    id: t.id.clone(),
                    sftp_available: true,
                    sftp_panel_height: 310.0,
                    sftp_panel_width: 380.0,
                    status: "Connected root@demo:22 (proxy socks5://127.0.0.1:10808)".into(),
                    ..Default::default()
                })
                .collect::<Vec<_>>(),
        )));
        ui.set_active_tab_id("ssh".into());
        ui.set_dock_central_w(1200.0);
        ui.set_dock_central_h(760.0);
        ui.set_panes(ModelRc::new(VecModel::from(vec![PaneInfo {
            id: 0,
            w: 1200.0,
            h: 760.0,
            active_id: "ssh".into(),
            tabs: ModelRc::new(VecModel::from(tabs.clone())),
            focused: true,
            ..Default::default()
        }])));
        let commands: Vec<QuickCmd> = [
            (
                "流媒体解锁检查",
                "bash <(curl -L -s https://example.com/check.sh)",
                "default",
                4,
            ),
            ("软件源更新", "apt update\napt upgrade", "default", 9),
            ("查看容器", "docker ps -a", "Docker", 2),
        ]
        .map(|(name, command, group, i)| QuickCmd {
            name: name.into(),
            command: command.into(),
            command_preview: command_preview(command).into(),
            group: group.into(),
            group_header: if i == 9 { "".into() } else { group.into() },
            orig_index: i,
            ..Default::default()
        })
        .to_vec();
        ui.set_quick_commands(ModelRc::new(VecModel::from(commands)));
        ui.invoke_refresh_bottom_commands();
        let copied = Rc::new(RefCell::new(Vec::new()));
        let calls = copied.clone();
        ui.on_copy_text(move |command| calls.borrow_mut().push(command));
        let edited = Rc::new(Cell::new(-1));
        let calls = edited.clone();
        ui.on_edit_quick_command(move |i| calls.set(i));
        let removed = Rc::new(Cell::new(-1));
        let calls = removed.clone();
        ui.on_delete_quick_command(move |i| calls.set(i));
        let ran = Rc::new(RefCell::new(Vec::new()));
        let calls = ran.clone();
        ui.on_run_command(move |tab, command, all| calls.borrow_mut().push((tab, command, all)));
        ui.show().unwrap();
        window.set_size(slint::PhysicalSize::new(1200, 800));
        let settle = || {
            let mut pixels = vec![slint::Rgb8Pixel::default(); 1200 * 800];
            for _ in 0..4 {
                slint::platform::update_timers_and_animations();
                window.request_redraw();
                window.draw_if_needed(|r| {
                    r.render(&mut pixels, 1200);
                });
            }
            pixels
        };
        let click = |x, y| {
            let position = slint::LogicalPosition::new(x, y);
            window.dispatch_event(WindowEvent::PointerMoved { position });
            window.dispatch_event(WindowEvent::PointerPressed {
                position,
                button: slint::platform::PointerEventButton::Left,
            });
            window.dispatch_event(WindowEvent::PointerReleased {
                position,
                button: slint::platform::PointerEventButton::Left,
            });
        };
        settle();
        click(120.0, 505.0);
        settle();
        assert_eq!(ui.get_bottom_panel_tab(), "tunnels");
        click(65.0, 505.0);
        settle();
        assert_eq!(ui.get_bottom_panel_tab(), "files");
        click(200.0, 505.0);
        settle();
        assert_eq!(ui.get_bottom_panel_tab(), "quick-commands");
        assert!(!ui.get_quick_panel_open());
        assert_eq!(store.borrow().bottom_panel_tab(), "quick-commands");
        // Exercise row actions through the real panel, including original indices.
        click(1092.0, 615.0);
        assert_eq!(
            ran.borrow().last().unwrap().1,
            "bash <(curl -L -s https://example.com/check.sh)"
        );
        click(1120.0, 615.0);
        assert_eq!(
            copied.borrow().last().unwrap(),
            "bash <(curl -L -s https://example.com/check.sh)"
        );
        click(1148.0, 615.0);
        assert_eq!(edited.get(), 4);
        click(1176.0, 652.0);
        assert_eq!(removed.get(), 9);
        ran.borrow_mut().clear();
        let saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(path.clone()).unwrap()).unwrap();
        assert_eq!(saved["bottom_panel_tab"], "quick-commands");
        // A second terminal or a new view uses the same window preference.
        ui.set_active_tab_id("other".into());
        settle();
        assert_eq!(ui.get_bottom_panel_tab(), "quick-commands");
        ui.invoke_execute_bottom_command("ssh".into(), "wrong target".into());
        assert!(ran.borrow().is_empty());
        ui.set_active_tab_id("ssh".into());
        click(100.0, 652.0);
        settle();
        assert!(!ui.get_cmd_bar_hidden());
        assert!(ran.borrow().is_empty());
        // Recalled raw multiline text lands in the input and Return sends it.
        window.dispatch_event(WindowEvent::KeyPressed {
            text: Key::Return.into(),
        });
        window.dispatch_event(WindowEvent::KeyReleased {
            text: Key::Return.into(),
        });
        assert_eq!(ran.borrow().last().unwrap().1, "apt update\napt upgrade");
        ui.invoke_execute_bottom_command("ssh".into(), "docker ps -a".into());
        assert_eq!(ran.borrow().last().unwrap().0, "ssh");
        assert!(!ran.borrow().last().unwrap().2);
        click(100.0, 545.0);
        window.dispatch_event(WindowEvent::KeyPressed {
            text: "UPGRADE".into(),
        });
        window.dispatch_event(WindowEvent::KeyReleased {
            text: "UPGRADE".into(),
        });
        settle();
        assert_eq!(ui.get_bottom_quick_commands().row_count(), 1);
        assert_eq!(
            ui.get_bottom_quick_commands()
                .row_data(0)
                .unwrap()
                .orig_index,
            9
        );
        settle();
        click(1148.0, 615.0);
        assert_eq!(edited.get(), 9);
        click(1176.0, 615.0);
        assert_eq!(removed.get(), 9);
        ui.set_bottom_quick_query("".into());
        ui.invoke_refresh_bottom_commands();
        let count = ran.borrow().len();
        ui.set_tabs(ModelRc::new(VecModel::from(
            tabs.into_iter()
                .map(|mut t| {
                    t.connected = false;
                    t
                })
                .collect::<Vec<_>>(),
        )));
        ui.set_conn_state(0);
        settle();
        click(1092.0, 615.0);
        assert_eq!(ran.borrow().len(), count);
        ui.invoke_execute_bottom_command("ssh".into(), "offline".into());
        assert_eq!(ran.borrow().len(), count);
        ui.set_tabs(ModelRc::new(VecModel::from(
            ["ssh", "other"]
                .map(|id| TabInfo {
                    id: id.into(),
                    kind: "terminal".into(),
                    title: "Demo SSH".into(),
                    connected: true,
                    ..Default::default()
                })
                .to_vec(),
        )));
        ui.set_conn_state(1);
        let pixels = settle();
        if let Ok(path) = std::env::var("MEATSHELL_BOTTOM_PANEL_SCREENSHOT") {
            let raw: Vec<u8> = pixels.iter().flat_map(|p| [p.r, p.g, p.b]).collect();
            image::save_buffer(path, &raw, 1200, 800, image::ColorType::Rgb8).unwrap();
        }
        ui.invoke_select_bottom_panel("tunnels".into());
        assert_eq!(ui.get_bottom_panel_tab(), "tunnels");
        ui.invoke_select_bottom_panel("files".into());
        assert_eq!(ui.get_bottom_panel_tab(), "files");
        let mut terms: Vec<_> = ui.get_terminals().iter().collect();
        terms[0].sftp_available = false;
        ui.set_terminals(ModelRc::new(VecModel::from(terms)));
        settle();
        assert_eq!(ui.get_bottom_panel_tab(), "files");
        click(1092.0, 615.0);
        assert_eq!(
            ran.borrow().last().unwrap().1,
            "bash <(curl -L -s https://example.com/check.sh)"
        );
        // The unavailable file tabs do not overwrite the saved preference.
        click(120.0, 505.0);
        assert_eq!(ui.get_bottom_panel_tab(), "files");
        // Exercise the same panel in each supported docking direction.
        for dock in ["left", "right", "top", "bottom"] {
            ui.set_sftp_dock(dock.into());
            settle();
            assert_eq!(ui.get_bottom_panel_tab(), "files");
        }
        click(600.0, 758.0);
        settle();
        assert!(ui.get_quick_cmd_manage_open());
        ui.set_quick_cmd_manage_open(false);
        // Concurrent external writes reject the save and roll back both states.
        std::fs::write(path.clone(), b"{}\n").unwrap();
        ui.invoke_select_bottom_panel("quick-commands".into());
        assert_eq!(ui.get_bottom_panel_tab(), "files");
        assert_eq!(store.borrow().bottom_panel_tab(), "files");
        let _ = std::fs::remove_file(path);
    })
    .join()
    .unwrap();
}
