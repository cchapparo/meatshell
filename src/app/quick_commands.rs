use super::*;

/// Variable-width named tiles, laid out separately for each panel's viewport.
/// Width is capped so one long name cannot force horizontal scrolling.
fn layout_command_tiles(
    source: &ModelRc<QuickCmd>,
    available_width: f32,
    font_size: f32,
) -> ModelRc<crate::ui::QuickCommandTile> {
    use unicode_width::UnicodeWidthStr;
    let width = if available_width.is_finite() {
        available_width.max(1.0)
    } else {
        1.0
    };
    let font = if font_size.is_finite() {
        font_size.clamp(8.0, 40.0)
    } else {
        13.0
    };
    let mut tiles = Vec::new();
    let (mut x, mut y) = (0.0_f32, 0.0_f32);
    for item in source.iter() {
        if !item.group_header.is_empty() {
            if x > 0.0 {
                y += 36.0;
                x = 0.0;
            }
            tiles.push(crate::ui::QuickCommandTile {
                item: item.clone(),
                x: 0.0,
                y,
                width,
                header: true,
            });
            y += 31.0;
        }
        if item.orig_index < 0 || item.collapsed {
            continue;
        }
        let tile_width = (item.name.as_str().width() as f32 * font * 0.64 + 45.0)
            .clamp(64.0, 300.0)
            .min(width);
        if x > 0.0 && x + tile_width > width {
            x = 0.0;
            y += 36.0;
        }
        tiles.push(crate::ui::QuickCommandTile {
            item,
            x,
            y,
            width: tile_width,
            header: false,
        });
        x += tile_width + 5.0;
    }
    ModelRc::new(VecModel::from(tiles))
}

/// Search this view without mutating the shared model's collapse state or indices.
fn bottom_command_model(source: &ModelRc<QuickCmd>, query: &str) -> ModelRc<QuickCmd> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return source.clone();
    }
    let mut rows = Vec::new();
    let mut previous_group = String::new();
    for mut row in source.iter() {
        if ![&row.name, &row.command, &row.group]
            .iter()
            .any(|text| text.to_lowercase().contains(&query))
        {
            continue;
        }
        row.group_header = if row.group.as_str() != previous_group {
            previous_group = row.group.to_string();
            row.group.clone()
        } else {
            SharedString::default()
        };
        row.collapsed = false;
        rows.push(row);
    }
    ModelRc::new(VecModel::from(rows))
}

pub(super) fn wire_bottom_commands(window: &AppWindow, store: Rc<RefCell<ConfigStore>>) {
    window.on_layout_bottom_commands(move |source, width, font_size| {
        layout_command_tiles(&source, width, font_size)
    });
    let collapsed = Rc::new(RefCell::new(std::collections::HashSet::<String>::new()));
    let collapsed_refresh = collapsed.clone();
    let weak = window.as_weak();
    window.on_refresh_bottom_commands(move || {
        if let Some(w) = weak.upgrade() {
            let query = w.get_bottom_quick_query();
            let rows = bottom_command_model(&w.get_quick_commands(), &query);
            // The tiled view starts expanded, independently of popup fold state.
            let rows: Vec<_> = rows
                .iter()
                .map(|mut row| {
                    row.collapsed = query.trim().is_empty()
                        && collapsed_refresh.borrow().contains(row.group.as_str());
                    row
                })
                .collect();
            w.set_bottom_quick_commands(ModelRc::new(VecModel::from(rows)));
        }
    });
    window.invoke_refresh_bottom_commands();
    let weak = window.as_weak();
    window.on_toggle_bottom_group(move |group| {
        {
            let mut groups = collapsed.borrow_mut();
            if !groups.remove(group.as_str()) {
                groups.insert(group.to_string());
            }
        }
        if let Some(w) = weak.upgrade() {
            w.invoke_refresh_bottom_commands();
        }
    });

    let weak = window.as_weak();
    window.on_select_bottom_panel(move |tab| {
        let mut store = store.borrow_mut();
        let previous = store.bottom_panel_tab().to_string();
        store.set_bottom_panel_tab(&tab);
        if let Err(error) = store.save() {
            store.set_bottom_panel_tab(&previous);
            tracing::error!("Failed to save bottom panel tab: {error:#}");
            return;
        }
        let selected: SharedString = store.bottom_panel_tab().into();
        drop(store);
        if let Some(w) = weak.upgrade() {
            w.set_bottom_panel_tab(selected);
            // The new tab owns the dock; avoid a duplicate docked command list.
            if w.get_bottom_panel_tab() == "quick-commands" && w.get_quick_panel_open() {
                w.set_quick_panel_open(false);
                w.invoke_dock_layout_changed();
            }
        }
    });

    let weak = window.as_weak();
    window.on_recall_bottom_command(move |tab, command| {
        if let Some(w) = weak.upgrade() {
            if tab != w.get_active_tab_id() || !w.get_terminals().iter().any(|t| t.id == tab) {
                return;
            }
            w.set_cmd_bar_hidden(false);
            w.invoke_recall_command(command);
        }
    });
    let weak = window.as_weak();
    window.on_execute_bottom_command(move |tab, command| {
        if let Some(w) = weak.upgrade() {
            if tab == w.get_active_tab_id()
                && w.get_tabs().iter().any(|t| t.id == tab && t.connected)
                && w.get_terminals().iter().any(|t| t.id == tab)
            {
                w.invoke_run_command(tab, command, false);
            }
        }
    });
}

pub(super) fn all_quick_group_names(store: &ConfigStore) -> std::collections::HashSet<String> {
    let cmds = store.quick_commands();
    let mut set: std::collections::HashSet<String> = std::collections::HashSet::new();
    if cmds.iter().any(|c| c.group.trim().is_empty()) {
        set.insert("default".to_string());
    }
    for g in store.quick_groups() {
        set.insert(g.clone());
    }
    for c in cmds {
        let g = c.group.trim();
        if !g.is_empty() {
            set.insert(g.to_string());
        }
    }
    set
}

/// Build the quick-command model for the command bar + manage dialog (#55).
///
/// Grouped like the welcome session list: the implicit "default" group (entries
/// with an empty group) comes first, then named groups alphabetically. Within a
/// group, entries keep their saved order. `group_header` is set on the first row
/// of each group; `collapsed` reflects `collapsed_groups` (runtime-only state);
/// `orig_index` points back into the stored vec so deletes target the right entry
/// even though the display order differs.
pub(super) fn quick_cmd_model(
    store: &ConfigStore,
    collapsed_groups: &std::collections::HashSet<String>,
) -> ModelRc<QuickCmd> {
    let cmds = store.quick_commands();

    let has_default = cmds.iter().any(|c| c.group.trim().is_empty());
    // Named groups = explicit quick-groups ∪ groups referenced by commands.
    let mut named: Vec<String> = store
        .quick_groups()
        .iter()
        .cloned()
        .chain(
            cmds.iter()
                .map(|c| c.group.trim().to_string())
                .filter(|g| !g.is_empty()),
        )
        .collect();
    named.sort_by_key(|g| g.to_lowercase());
    named.dedup();

    let mut groups: Vec<String> = Vec::new();
    if has_default {
        groups.push("default".to_string());
    }
    groups.extend(named);

    let mut rows: Vec<QuickCmd> = Vec::new();
    for group in &groups {
        let is_collapsed = collapsed_groups.contains(group);
        let members: Vec<(usize, &crate::config::QuickCommand)> = cmds
            .iter()
            .enumerate()
            .filter(|(_, c)| {
                let g = c.group.trim();
                if group == "default" {
                    g.is_empty()
                } else {
                    g == group
                }
            })
            .collect();
        if members.is_empty() {
            // Header-only placeholder for an empty group (orig_index -1) so it can
            // still be renamed / deleted, matching empty session folders.
            rows.push(QuickCmd {
                name: "".into(),
                command: "".into(),
                group: group.clone().into(),
                group_header: group.clone().into(),
                collapsed: is_collapsed,
                orig_index: -1,
                send_enter: true,
                command_preview: "".into(),
            });
        } else {
            for (i, (orig_idx, c)) in members.iter().enumerate() {
                rows.push(QuickCmd {
                    name: c.name.clone().into(),
                    command: c.command.clone().into(),
                    group: group.clone().into(),
                    group_header: if i == 0 {
                        group.clone().into()
                    } else {
                        "".into()
                    },
                    collapsed: is_collapsed,
                    orig_index: *orig_idx as i32,
                    send_enter: c.send_enter,
                    command_preview: command_preview(&c.command).into(),
                });
            }
        }
    }
    ModelRc::from(Rc::new(VecModel::from(rows)))
}

/// One-line rendering of a quick command for lists and chips (#419).
///
/// Multi-line commands (scripts, `for` loops pasted from a .bat) used to be
/// drawn with their real line breaks inside a fixed-height row and spilled
/// over the neighbouring entries. Line breaks become a visible ` ⏎ ` marker,
/// runs of whitespace collapse to one space, and very long commands are cut
/// so the UI never lays out kilobytes of text for an elided label.
pub(super) fn command_preview(command: &str) -> String {
    const MAX_CHARS: usize = 240;
    let mut out = String::new();
    for (i, line) in command
        .lines()
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|line| !line.is_empty())
        .enumerate()
    {
        if i > 0 {
            out.push_str(" \u{23CE} ");
        }
        out.push_str(&line);
        if out.chars().count() > MAX_CHARS {
            break;
        }
    }
    if out.chars().count() > MAX_CHARS {
        out = out.chars().take(MAX_CHARS).collect();
        out.push('…');
    }
    out
}

/// Where the entry being edited ends up after deleting `deleted` (#419):
/// `-1` (form reset) if it was the deleted one, shifted down if it was after it.
pub(super) fn edit_index_after_delete(edit_index: i32, deleted: i32) -> i32 {
    if edit_index < 0 || deleted < 0 {
        edit_index
    } else if edit_index == deleted {
        -1
    } else if edit_index > deleted {
        edit_index - 1
    } else {
        edit_index
    }
}

/// Where the entry being edited ends up after `a` and `b` swap places.
pub(super) fn edit_index_after_swap(edit_index: i32, a: usize, b: usize) -> i32 {
    if edit_index == a as i32 {
        b as i32
    } else if edit_index == b as i32 {
        a as i32
    } else {
        edit_index
    }
}

/// Move entry `index` one step within its group. Returns the index it was
/// swapped with, or `None` if it was already at the edge of its group.
pub(super) fn reorder_quick_command(
    commands: &mut [crate::config::QuickCommand],
    index: usize,
    move_up: bool,
) -> Option<usize> {
    let Some(current) = commands.get(index) else {
        return None;
    };
    let group = current.group.trim().to_string();
    let target = if move_up {
        (0..index)
            .rev()
            .find(|&candidate| commands[candidate].group.trim() == group)
    } else {
        (index + 1..commands.len())
            .find(|&candidate| commands[candidate].group.trim() == group)
    };
    if let Some(target) = target {
        commands.swap(index, target);
    }
    target
}

#[cfg(test)]
mod reorder_tests {
    use super::reorder_quick_command;
    use crate::config::QuickCommand;

    fn command(name: &str, group: &str) -> QuickCommand {
        QuickCommand {
            name: name.to_string(),
            command: name.to_string(),
            group: group.to_string(),
            send_enter: true,
        }
    }

    #[test]
    fn reorders_only_within_the_current_group() {
        let mut commands = vec![
            command("a", "ops"),
            command("x", "other"),
            command("b", "ops"),
        ];
        assert_eq!(reorder_quick_command(&mut commands, 2, true), Some(0));
        assert_eq!(
            commands.iter().map(|item| item.name.as_str()).collect::<Vec<_>>(),
            vec!["b", "x", "a"]
        );
        assert_eq!(reorder_quick_command(&mut commands, 0, true), None);
    }
}

#[cfg(test)]
mod preview_tests {
    use super::command_preview;

    #[test]
    fn single_line_command_is_unchanged() {
        assert_eq!(command_preview("docker exec -it goose-cli bash"), "docker exec -it goose-cli bash");
    }

    #[test]
    fn multi_line_command_becomes_one_line() {
        let cmd = "(for /f \"tokens=5\" %a in ('netstat -ano') do (\r\n    taskkill /F /PID %a\r\n))\r\n";
        let preview = command_preview(cmd);
        assert!(!preview.contains('\n') && !preview.contains('\r'));
        assert_eq!(
            preview,
            "(for /f \"tokens=5\" %a in ('netstat -ano') do ( \u{23CE} taskkill /F /PID %a \u{23CE} ))"
        );
    }

    #[test]
    fn blank_lines_and_tabs_collapse() {
        assert_eq!(command_preview("\n\tls   -la\n\n\tpwd\n"), "ls -la \u{23CE} pwd");
    }

    #[test]
    fn pasted_top_output_renders_as_one_line() {
        let pasted = "7 root       0 -20       0      0      0 I   0.0   0.0   0:00.00 kworker/R-netns\n      9 root       0 -20       0      0      0 I   0.0   0.0   0:00.00 kworker/0:0H-events_hig+\n     12 root       0 -20       0      0      0 I   0.0   0.0   0:00.00 kworker/R-mm_pe\n";
        let preview = command_preview(pasted);
        assert!(!preview.contains('\n'));
        assert!(preview.starts_with("7 root 0 -20 0 0 0 I 0.0 0.0 0:00.00 kworker/R-netns \u{23CE} 9 root"));
    }

    #[test]
    fn very_long_command_is_truncated() {
        let preview = command_preview(&"x".repeat(1000));
        assert_eq!(preview.chars().count(), 241);
        assert!(preview.ends_with('…'));
    }
}

#[cfg(test)]
mod edit_index_tests {
    use super::{edit_index_after_delete, edit_index_after_swap};

    #[test]
    fn delete_keeps_the_edited_entry_selected() {
        assert_eq!(edit_index_after_delete(3, 1), 2);
        assert_eq!(edit_index_after_delete(3, 5), 3);
        assert_eq!(edit_index_after_delete(3, 3), -1);
        assert_eq!(edit_index_after_delete(-1, 0), -1);
    }

    #[test]
    fn swap_follows_the_edited_entry() {
        assert_eq!(edit_index_after_swap(2, 2, 0), 0);
        assert_eq!(edit_index_after_swap(0, 2, 0), 2);
        assert_eq!(edit_index_after_swap(1, 2, 0), 1);
    }
}

#[cfg(test)]
#[path = "../../tests/app/bottom_panel.rs"]
mod bottom_panel_tests;
