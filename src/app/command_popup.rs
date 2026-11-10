//! A bare Alt release opens the searchable command list without taking Alt chords.
use super::*;
use i_slint_backend_winit::winit::{
    event::{ElementState, MouseButton},
    keyboard::{KeyCode, ModifiersState, PhysicalKey},
};
use slint::Model;
use std::collections::HashSet;

#[derive(Default)]
pub(super) struct AltTapTracker {
    keys: HashSet<PhysicalKey>,
    buttons: HashSet<MouseButton>,
    modifiers: ModifiersState,
    armed: Option<PhysicalKey>,
}

impl AltTapTracker {
    pub(super) fn reset(&mut self) {
        *self = Self::default();
    }
    pub(super) fn cancel(&mut self) {
        self.armed = None;
    }
    pub(super) fn modifiers(&mut self, modifiers: ModifiersState) {
        self.modifiers = modifiers;
        if modifiers.control_key() || modifiers.shift_key() || modifiers.super_key() {
            self.cancel();
        }
    }
    pub(super) fn mouse(&mut self, button: MouseButton, state: ElementState) {
        self.cancel();
        if state == ElementState::Pressed {
            self.buttons.insert(button);
        } else {
            self.buttons.remove(&button);
        }
    }
    pub(super) fn key(
        &mut self,
        key: PhysicalKey,
        state: ElementState,
        repeat: bool,
        alt_graph: bool,
    ) -> bool {
        if alt_graph {
            self.cancel();
            if state == ElementState::Pressed {
                self.keys.insert(key);
            } else {
                self.keys.remove(&key);
            }
            return false;
        }
        match state {
            ElementState::Pressed => {
                if repeat || self.keys.contains(&key) {
                    if self.armed != Some(key) {
                        self.cancel();
                    }
                    self.keys.insert(key);
                    return false;
                }
                let alt = matches!(key, PhysicalKey::Code(KeyCode::AltLeft | KeyCode::AltRight));
                self.armed = if alt
                    && self.keys.is_empty()
                    && self.buttons.is_empty()
                    && !self.modifiers.control_key()
                    && !self.modifiers.shift_key()
                    && !self.modifiers.super_key()
                {
                    Some(key)
                } else {
                    None
                };
                self.keys.insert(key);
                false
            }
            ElementState::Released => {
                self.keys.remove(&key);
                let tapped =
                    self.armed == Some(key) && self.keys.is_empty() && self.buttons.is_empty();
                self.armed = None;
                tapped
            }
        }
    }
}

fn refresh(window: &AppWindow) {
    let query = window.get_command_popup_query().to_lowercase();
    let rows: Vec<crate::ui::CommandPopupRow> = if window.get_command_popup_quick() {
        window
            .get_quick_commands()
            .iter()
            .filter(|q| q.orig_index >= 0)
            .filter(|q| {
                format!("{}\n{}\n{}", q.name, q.group, q.command)
                    .to_lowercase()
                    .contains(&query)
            })
            .map(|q| crate::ui::CommandPopupRow {
                label: format!("{}  —  {}", q.name, command_preview(q.command.as_str())).into(),
                command: q.command,
                quick_index: q.orig_index,
            })
            .collect()
    } else {
        window
            .get_command_history()
            .iter()
            .filter(|c| c.to_lowercase().contains(&query))
            .map(|command| crate::ui::CommandPopupRow {
                label: command_preview(command.as_str()).into(),
                command,
                quick_index: -1,
            })
            .collect()
    };
    window.set_command_popup_rows(ModelRc::new(VecModel::from(rows)));
}

fn target(window: &AppWindow) -> Option<SharedString> {
    let id = window.get_command_popup_tab();
    (id == window.get_active_tab_id() && window.get_terminals().iter().any(|t| t.id == id))
        .then_some(id)
}

pub(super) fn wire(window: &AppWindow) {
    let weak = window.as_weak();
    window.on_toggle_command_popup(move || {
        let Some(w) = weak.upgrade() else { return };
        if w.get_command_popup_open() {
            if w.get_command_popup_active() {
                w.set_command_popup_open(false);
            }
        } else if !w.get_modal_open()
            && w.get_tabs()
                .iter()
                .any(|t| t.id == w.get_active_tab_id() && t.kind == "terminal")
        {
            w.set_command_popup_tab(w.get_active_tab_id());
            w.set_command_popup_quick(false);
            w.set_command_popup_query("".into());
            refresh(&w);
            w.set_command_popup_open(true);
        }
    });
    let weak = window.as_weak();
    window.on_refresh_command_popup(move || {
        if let Some(w) = weak.upgrade() {
            refresh(&w);
        }
    });
    let weak = window.as_weak();
    window.on_command_popup_activate(move |index, execute| {
        let Some(w) = weak.upgrade() else { return };
        if !w.get_command_popup_active() {
            return;
        }
        let Some(row) = usize::try_from(index)
            .ok()
            .and_then(|i| w.get_command_popup_rows().row_data(i))
        else {
            return;
        };
        let tab = target(&w);
        w.set_command_popup_open(false);
        let Some(tab) = tab else { return };
        if execute {
            // Explicitly target this terminal; never inherit the command bar's broadcast switch.
            w.invoke_run_command(tab, row.command, false);
        } else {
            w.set_cmd_bar_hidden(false);
            let weak = w.as_weak();
            slint::Timer::single_shot(std::time::Duration::ZERO, move || {
                if let Some(w) = weak.upgrade() {
                    if !w.get_modal_open() && w.get_active_tab_id() == tab {
                        w.invoke_recall_command(row.command);
                    }
                }
            });
        }
    });
    let weak = window.as_weak();
    window.on_command_popup_copy(move |index| {
        if let Some(w) = weak.upgrade() {
            if !w.get_command_popup_active() {
                return;
            }
            if let Some(row) = usize::try_from(index)
                .ok()
                .and_then(|i| w.get_command_popup_rows().row_data(i))
            {
                w.invoke_copy_text(row.command);
            }
        }
    });
    let weak = window.as_weak();
    window.on_command_popup_remove(move |index| {
        if let Some(w) = weak.upgrade() {
            if !w.get_command_popup_active() {
                return;
            }
            if let Some(row) = usize::try_from(index)
                .ok()
                .and_then(|i| w.get_command_popup_rows().row_data(i))
            {
                if row.quick_index >= 0 {
                    w.invoke_delete_quick_command(row.quick_index);
                } else {
                    w.invoke_delete_history_cmd(row.command);
                }
                refresh(&w);
            }
        }
    });
}

#[cfg(test)]
#[path = "../../tests/app/command_popup.rs"]
mod tests;
