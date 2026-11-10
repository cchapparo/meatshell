use super::*;
use ElementState::{Pressed, Released};

fn press(tracker: &mut AltTapTracker, key: KeyCode) -> bool {
    tracker.key(PhysicalKey::Code(key), Pressed, false, false)
}
fn release(tracker: &mut AltTapTracker, key: KeyCode) -> bool {
    tracker.key(PhysicalKey::Code(key), Released, false, false)
}

#[test]
fn alt_tap_opens_only_on_release_once_including_after_repeat() {
    for alt in [KeyCode::AltLeft, KeyCode::AltRight] {
        let mut t = AltTapTracker::default();
        assert!(!release(&mut t, alt));
        assert!(!press(&mut t, alt));
        assert!(!t.key(PhysicalKey::Code(alt), Pressed, true, false));
        assert!(release(&mut t, alt));
        assert!(!release(&mut t, alt));
    }
}

#[test]
fn alt_tap_preserves_chords_in_either_press_and_release_order() {
    for key in [
        KeyCode::KeyB,
        KeyCode::Enter,
        KeyCode::Tab,
        KeyCode::ControlLeft,
        KeyCode::ShiftLeft,
        KeyCode::SuperLeft,
        KeyCode::AltRight,
    ] {
        for alt_first in [true, false] {
            for alt_released_first in [true, false] {
                let mut t = AltTapTracker::default();
                for k in if alt_first {
                    [KeyCode::AltLeft, key]
                } else {
                    [key, KeyCode::AltLeft]
                } {
                    assert!(!press(&mut t, k));
                }
                for k in if alt_released_first {
                    [KeyCode::AltLeft, key]
                } else {
                    [key, KeyCode::AltLeft]
                } {
                    assert!(!release(&mut t, k));
                }
                assert!(!press(&mut t, KeyCode::AltLeft));
                assert!(release(&mut t, KeyCode::AltLeft));
            }
        }
    }
}

#[test]
fn alt_tap_ignores_altgr_mouse_composition_and_focus_loss() {
    let mut t = AltTapTracker::default();
    t.modifiers(ModifiersState::CONTROL | ModifiersState::ALT);
    press(&mut t, KeyCode::AltRight);
    assert!(!release(&mut t, KeyCode::AltRight));
    t.reset();
    assert!(!t.key(PhysicalKey::Code(KeyCode::AltRight), Pressed, false, true));
    assert!(!t.key(PhysicalKey::Code(KeyCode::AltRight), Released, false, true));
    press(&mut t, KeyCode::AltLeft);
    assert!(!t.key(PhysicalKey::Code(KeyCode::KeyB), Pressed, true, false));
    assert!(!release(&mut t, KeyCode::AltLeft));
    t.reset();
    for cancel in 0..5 {
        press(&mut t, KeyCode::AltLeft);
        match cancel {
            0 => t.mouse(MouseButton::Left, Pressed),
            1 => t.cancel(), // wheel / IME composition
            2 => t.modifiers(ModifiersState::SHIFT),
            3 => t.reset(), // Alt+Tab / focus loss / synthetic event
            _ => t.modifiers(ModifiersState::SUPER),
        }
        assert!(!release(&mut t, KeyCode::AltLeft));
        t.reset();
    }
    t.mouse(MouseButton::Right, Pressed);
    press(&mut t, KeyCode::AltLeft);
    assert!(!release(&mut t, KeyCode::AltLeft));
}

#[test]
fn command_popup_filters_recalls_runs_and_restores_terminal_focus() {
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
        wire(&ui);
        ui.set_ui_font_family(resolve_ui_font_family());
        ui.set_dark_mode(false);
        ui.set_terminal_theme("dark".into());
        ui.set_cmd_bar_hidden(true);
        let tab = TabInfo {
            id: "ssh".into(),
            title: "Demo SSH".into(),
            kind: "terminal".into(),
            connected: true,
            ..Default::default()
        };
        ui.set_tabs(ModelRc::new(VecModel::from(vec![tab.clone()])));
        ui.set_terminals(ModelRc::new(VecModel::from(vec![TerminalState {
            id: "ssh".into(),
            sftp_collapsed: true,
            status: "Connected to demo SSH".into(),
            ..Default::default()
        }])));
        ui.set_active_tab_id("ssh".into());
        ui.set_dock_central_w(1200.0);
        ui.set_dock_central_h(760.0);
        ui.set_panes(ModelRc::new(VecModel::from(vec![PaneInfo {
            id: 0,
            x: 0.0,
            y: 0.0,
            w: 1200.0,
            h: 760.0,
            active_id: "ssh".into(),
            tabs: ModelRc::new(VecModel::from(vec![tab])),
            focused: true,
            ..Default::default()
        }])));
        let history: Vec<SharedString> = [
            "docker ps",
            "docker logs -f web",
            "systemctl status nginx",
            "printf 'one\\ntwo'\necho done",
        ]
        .map(Into::into)
        .to_vec();
        ui.set_command_history(ModelRc::new(VecModel::from(history.clone())));
        ui.set_quick_commands(ModelRc::new(VecModel::from(vec![QuickCmd {
            name: "查看容器".into(),
            command: "docker ps -a".into(),
            group: "Docker".into(),
            orig_index: 7,
            ..Default::default()
        }])));
        let ran = Rc::new(RefCell::new(Vec::new()));
        let calls = ran.clone();
        ui.on_run_command(move |tab, command, all| calls.borrow_mut().push((tab, command, all)));
        let copied = Rc::new(RefCell::new(Vec::new()));
        let calls = copied.clone();
        ui.on_copy_text(move |command| calls.borrow_mut().push(command));
        let deleted = Rc::new(Cell::new(-1));
        let calls = deleted.clone();
        ui.on_delete_quick_command(move |i| calls.set(i));
        let typed = Rc::new(RefCell::new(Vec::new()));
        let calls = typed.clone();
        ui.on_send_key(move |tab, text, _, _, _| calls.borrow_mut().push((tab, text)));
        let cleared = Rc::new(Cell::new(0));
        let calls = cleared.clone();
        let weak = ui.as_weak();
        ui.on_clear_command_history(move || {
            calls.set(calls.get() + 1);
            weak.upgrade()
                .unwrap()
                .set_command_history(ModelRc::new(VecModel::default()));
        });
        ui.show().unwrap();
        window.set_size(slint::PhysicalSize::new(1200, 760));
        let settle = || {
            for _ in 0..4 {
                slint::platform::update_timers_and_animations();
                let mut pixels = vec![slint::Rgb8Pixel::default(); 1200 * 760];
                window.request_redraw();
                window.draw_if_needed(|r| {
                    r.render(&mut pixels, 1200);
                });
            }
        };
        let key = |text: SharedString| {
            window.dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
            window.dispatch_event(WindowEvent::KeyReleased { text });
        };
        let click = |x: f32, y: f32| {
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
        ui.invoke_toggle_command_popup();
        settle();
        assert!(ui.get_command_popup_open());
        assert!(ui.get_cmd_bar_hidden());
        assert_eq!(ui.get_command_popup_rows().row_count(), 4);
        key(Key::UpArrow.into());
        key(Key::Return.into()); // recall the previous command, never execute
        settle();
        assert!(!ui.get_command_popup_open());
        assert!(!ui.get_cmd_bar_hidden());
        assert!(ran.borrow().is_empty());
        assert!(typed.borrow().is_empty());
        // Focus must land in the command input, not the remote terminal.
        key("x".into());
        assert!(typed.borrow().is_empty());
        key(Key::Return.into());
        assert_eq!(ran.borrow().last().unwrap().1, "systemctl status nginxx");
        assert!(!ran.borrow().last().unwrap().2);

        ui.invoke_toggle_command_popup();
        settle();
        key("DOCKER".into());
        settle();
        assert_eq!(ui.get_command_popup_rows().row_count(), 2);
        ui.invoke_command_popup_copy(1);
        assert_eq!(copied.borrow()[0], "docker logs -f web");
        ui.set_command_popup_query("".into());
        ui.invoke_refresh_command_popup();
        settle();
        ui.invoke_command_popup_copy(3);
        assert_eq!(copied.borrow().last().unwrap(), history.last().unwrap());
        if let Ok(path) = std::env::var("MEATSHELL_ALT_POPUP_SCREENSHOT") {
            let demo: Vec<SharedString> = [
                "docker ps -a",
                "docker logs --tail 100 web",
                "docker stats --no-stream",
                "systemctl status nginx",
                "journalctl -u nginx -n 50",
                "ss -lntp",
                "ip addr",
                "df -h",
                "free -h",
                "uname -a",
                "uptime",
                "ls -lah /var/log",
                "systemctl status docker",
            ]
            .map(Into::into)
            .to_vec();
            ui.set_command_history(ModelRc::new(VecModel::from(demo)));
            settle();
            let mut pixels = vec![slint::Rgb8Pixel::default(); 1200 * 760];
            window.request_redraw();
            window.draw_if_needed(|r| {
                r.render(&mut pixels, 1200);
            });
            let bytes: Vec<u8> = pixels.iter().flat_map(|p| [p.r, p.g, p.b]).collect();
            image::save_buffer(path, &bytes, 1200, 760, image::ColorType::Rgb8).unwrap();
            ui.set_command_history(ModelRc::new(VecModel::from(history.clone())));
            settle();
        }
        window.dispatch_event(WindowEvent::KeyPressed {
            text: Key::Control.into(),
        });
        key(Key::Return.into());
        window.dispatch_event(WindowEvent::KeyReleased {
            text: Key::Control.into(),
        });
        settle();
        assert_eq!(
            ran.borrow().last().unwrap().1,
            history.last().unwrap().as_str()
        );
        assert!(!ran.borrow().last().unwrap().2);
        assert!(!ui.get_command_popup_open());
        ui.invoke_toggle_command_popup();
        settle();
        click(374.0, 175.0);
        key("容器".into());
        settle();
        assert!(ui.get_command_popup_quick());
        assert_eq!(ui.get_command_popup_rows().row_count(), 1);
        ui.invoke_command_popup_remove(0);
        assert_eq!(deleted.get(), 7);
        key(Key::Escape.into());
        settle();
        assert!(!ui.get_command_popup_open());
        key("z".into());
        assert_eq!(typed.borrow().last().unwrap().1, "z");
        ui.invoke_toggle_command_popup();
        settle();
        ui.invoke_toggle_command_popup();
        settle();
        assert!(!ui.get_command_popup_open());
        ui.set_interface_open(true);
        ui.invoke_toggle_command_popup();
        assert!(!ui.get_command_popup_open());
        ui.set_interface_open(false);
        ui.invoke_toggle_command_popup();
        settle();
        click(887.0, 547.0);
        settle();
        assert_eq!(cleared.get(), 0);
        click(887.0, 547.0);
        settle();
        assert_eq!(cleared.get(), 1);
        assert_eq!(ui.get_command_popup_rows().row_count(), 0);
        assert!(ui.get_command_popup_open());
        // A tab change dismisses it; a stale callback cannot run on the new tab.
        ui.set_active_tab_id("welcome".into());
        settle();
        assert!(!ui.get_command_popup_open());
        let count = ran.borrow().len();
        ui.invoke_command_popup_activate(0, true);
        assert_eq!(ran.borrow().len(), count);
    })
    .join()
    .unwrap();
}
