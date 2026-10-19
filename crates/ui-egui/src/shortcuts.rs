//! Rebindable keyboard shortcuts for the editing commands, kept in a small JSON file the platform
//! provides. Navigation keys (arrows, Delete, Enter, Escape) are fixed.

use crate::i18n::{tr, trf};
use std::collections::BTreeMap;
use std::path::Path;

use egui::{Key, Modifiers};

use crate::LaserFoundryApp;

/// A shortcuts file larger than this is not read (it is not ours).
const MAX_FILE_BYTES: u64 = 64 * 1024;

/// The commands whose shortcut can be changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Action {
    Undo,
    Redo,
    SelectAll,
    Duplicate,
    Group,
    Ungroup,
}

impl Action {
    pub const ALL: [Action; 6] = [Action::Undo, Action::Redo, Action::SelectAll, Action::Duplicate, Action::Group, Action::Ungroup];

    pub fn label(self) -> &'static str {
        match self {
            Action::Undo => "Undo",
            Action::Redo => "Redo",
            Action::SelectAll => "Select all",
            Action::Duplicate => "Duplicate",
            Action::Group => "Group",
            Action::Ungroup => "Ungroup",
        }
    }

    pub fn command(self) -> &'static str {
        match self {
            Action::Undo => "edit.undo",
            Action::Redo => "edit.redo",
            Action::SelectAll => "edit.select_all",
            Action::Duplicate => "edit.duplicate",
            Action::Group => "edit.group",
            Action::Ungroup => "edit.ungroup",
        }
    }

    fn default_chord(self) -> Chord {
        match self {
            Action::Undo => Chord::command(Key::Z),
            Action::Redo => Chord { shift: true, ..Chord::command(Key::Z) },
            Action::SelectAll => Chord::command(Key::A),
            Action::Duplicate => Chord::command(Key::D),
            Action::Group => Chord::command(Key::G),
            Action::Ungroup => Chord { shift: true, ..Chord::command(Key::G) },
        }
    }
}

/// A key with modifiers. `command` is Ctrl (Cmd on a Mac).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Chord {
    pub command: bool,
    pub shift: bool,
    pub alt: bool,
    pub key: Key,
}

impl Chord {
    fn command(key: Key) -> Chord {
        Chord { command: true, shift: false, alt: false, key }
    }

    pub fn text(&self) -> String {
        let mut s = String::new();
        if self.command {
            s.push_str("Ctrl+");
        }
        if self.shift {
            s.push_str("Shift+");
        }
        if self.alt {
            s.push_str("Alt+");
        }
        s.push_str(self.key.name());
        s
    }

    pub fn parse(text: &str) -> Option<Chord> {
        let mut chord = Chord { command: false, shift: false, alt: false, key: Key::A };
        let mut parts: Vec<&str> = text.split('+').collect();
        let key = Key::from_name(parts.pop()?.trim())?;
        for p in parts {
            match p.trim().to_ascii_lowercase().as_str() {
                "ctrl" | "cmd" | "command" => chord.command = true,
                "shift" => chord.shift = true,
                "alt" => chord.alt = true,
                _ => return None,
            }
        }
        chord.key = key;
        chord.is_allowed().then_some(chord)
    }

    /// Plain letters would fight with typing and the fixed keys, so a chord needs Ctrl or Alt (or is a function key).
    pub fn is_allowed(&self) -> bool {
        self.command
            || self.alt
            || matches!(
                self.key,
                Key::F1 | Key::F2 | Key::F3 | Key::F4 | Key::F5 | Key::F6 | Key::F7 | Key::F8 | Key::F9 | Key::F10 | Key::F11 | Key::F12
            )
    }

    fn modifiers(&self) -> Modifiers {
        let mut m = Modifiers::NONE;
        if self.command {
            m |= Modifiers::COMMAND;
        }
        if self.shift {
            m |= Modifiers::SHIFT;
        }
        if self.alt {
            m |= Modifiers::ALT;
        }
        m
    }

    fn weight(&self) -> u8 {
        u8::from(self.command) + u8::from(self.shift) + u8::from(self.alt)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Shortcuts {
    chords: BTreeMap<Action, Chord>,
}

impl Default for Shortcuts {
    fn default() -> Self {
        Shortcuts { chords: Action::ALL.iter().map(|a| (*a, a.default_chord())).collect() }
    }
}

impl Shortcuts {
    /// Reads the saved bindings; a missing, oversized or damaged file (or entry) falls back to the defaults.
    pub fn load(path: &Path) -> Shortcuts {
        let mut s = Shortcuts::default();
        let Ok(meta) = std::fs::metadata(path) else { return s };
        if meta.len() > MAX_FILE_BYTES {
            return s;
        }
        let Ok(text) = std::fs::read_to_string(path) else { return s };
        let Ok(map) = serde_json::from_str::<BTreeMap<String, String>>(&text) else { return s };
        for action in Action::ALL {
            if let Some(chord) = map.get(action.command()).and_then(|t| Chord::parse(t))
                && s.conflict(action, chord).is_none()
            {
                s.chords.insert(action, chord);
            }
        }
        s
    }

    pub fn save(&self, path: &Path) {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent); // a failure shows up as the write error below
        }
        let map: BTreeMap<&str, String> = self.chords.iter().map(|(a, c)| (a.command(), c.text())).collect();
        match serde_json::to_string_pretty(&map) {
            Ok(text) => {
                if let Err(e) = std::fs::write(path, text) {
                    log::warn!("could not write {}: {e}", path.display());
                }
            }
            Err(e) => log::warn!("could not encode the shortcuts: {e}"),
        }
    }

    pub fn get(&self, action: Action) -> Chord {
        self.chords.get(&action).copied().unwrap_or_else(|| action.default_chord())
    }

    pub fn text(&self, action: Action) -> String {
        self.get(action).text()
    }

    /// Another action already using this chord.
    pub fn conflict(&self, action: Action, chord: Chord) -> Option<Action> {
        Action::ALL.into_iter().find(|a| *a != action && self.chords.get(a) == Some(&chord))
    }

    /// Binds a chord, refusing one that is not allowed or belongs to another action.
    pub fn set(&mut self, action: Action, chord: Chord) -> Result<(), String> {
        if !chord.is_allowed() {
            return Err(tr("use Ctrl or Alt together with a key"));
        }
        if let Some(other) = self.conflict(action, chord) {
            return Err(trf!("{chord} is already {action}", chord = chord.text(), action = tr(other.label())));
        }
        self.chords.insert(action, chord);
        Ok(())
    }

    pub fn reset(&mut self) {
        *self = Shortcuts::default();
    }

    /// Consumes whichever bound chords were pressed, the most specific first so that
    /// Ctrl+Shift+Z is never also taken as Ctrl+Z.
    pub fn pressed(&self, ctx: &egui::Context) -> Vec<Action> {
        let mut order: Vec<(Action, Chord)> = self.chords.iter().map(|(a, c)| (*a, *c)).collect();
        order.sort_by_key(|(_, c)| std::cmp::Reverse(c.weight()));
        ctx.input_mut(|i| order.into_iter().filter(|(_, c)| i.consume_key(c.modifiers(), c.key)).map(|(a, _)| a).collect())
    }
}

/// The "Keyboard shortcuts" window.
pub(crate) fn window(app: &mut LaserFoundryApp, ctx: &egui::Context) {
    if !app.shortcuts_open {
        return;
    }
    if let Some(action) = app.capturing {
        let captured = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Key { key, pressed: true, modifiers, .. } => Some((*key, *modifiers)),
                _ => None,
            })
        });
        if let Some((key, m)) = captured {
            app.capturing = None;
            if key != Key::Escape {
                let chord = Chord { command: m.command, shift: m.shift, alt: m.alt, key };
                match app.shortcuts.set(action, chord) {
                    Ok(()) => {
                        app.shortcut_error = None;
                        app.save_shortcuts();
                    }
                    Err(e) => app.shortcut_error = Some(e),
                }
            }
        }
    }
    let mut open = true;
    egui::Window::new("Keyboard shortcuts").open(&mut open).collapsible(false).resizable(false).show(ctx, |ui| {
        egui::Grid::new("shortcut_grid").num_columns(2).show(ui, |ui| {
            for action in Action::ALL {
                ui.label(tr(action.label()));
                let waiting = app.capturing == Some(action);
                let text = if waiting { "Press keys… (Esc cancels)".to_string() } else { app.shortcuts.text(action) };
                if ui.button(text).clicked() {
                    app.capturing = Some(action);
                    app.shortcut_error = None;
                }
                ui.end_row();
            }
        });
        if let Some(e) = &app.shortcut_error {
            ui.colored_label(app.tokens.error, e);
        }
        if ui.button(tr("Reset to defaults")).clicked() {
            app.shortcuts.reset();
            app.capturing = None;
            app.shortcut_error = None;
            app.save_shortcuts();
        }
    });
    if !open {
        app.shortcuts_open = false;
        app.capturing = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chords_round_trip_through_text() {
        for action in Action::ALL {
            let c = action.default_chord();
            assert_eq!(Chord::parse(&c.text()), Some(c), "{}", c.text());
        }
        assert_eq!(Chord::parse("ctrl+alt+K").map(|c| (c.command, c.alt, c.key)), Some((true, true, Key::K)));
    }

    #[test]
    fn unusable_chords_are_refused() {
        for bad in ["", "+", "Z", "Shift+Z", "Ctrl+", "Hyper+Z", "Ctrl+NoSuchKey", "Ctrl+Z+Z"] {
            assert_eq!(Chord::parse(bad), None, "{bad:?}");
        }
        assert!(Chord::parse("F5").is_some(), "function keys need no modifier");
        let mut s = Shortcuts::default();
        assert!(s.set(Action::Undo, Chord::command(Key::A)).unwrap_err().contains("Select all"));
        assert_eq!(s.get(Action::Undo), Chord::command(Key::Z), "a refused binding changes nothing");
        assert!(s.set(Action::Undo, Chord { command: false, shift: false, alt: false, key: Key::U }).is_err());
        assert!(s.set(Action::Undo, Chord::command(Key::U)).is_ok());
        s.reset();
        assert_eq!(s, Shortcuts::default());
    }

    #[test]
    fn the_conflict_message_follows_the_translation() {
        let catalog =
            crate::i18n::parse(r#"{"strings":{"{chord} is already {action}":"{action} hat schon {chord}","Select all":"Alles auswählen"}}"#).unwrap();
        crate::i18n::set_active(Some(catalog));
        let err = Shortcuts::default().set(Action::Undo, Chord::command(Key::A)).unwrap_err();
        crate::i18n::set_active(None);
        assert!(err.starts_with("Alles auswählen hat schon"), "{err}");
    }

    #[test]
    fn a_saved_file_is_read_back_and_a_hostile_one_is_ignored() {
        let dir = std::env::temp_dir().join(format!("lf-shortcuts-{}", std::process::id()));
        let file = dir.join("shortcuts.json");
        let mut s = Shortcuts::default();
        s.set(Action::Duplicate, Chord { alt: true, ..Chord::command(Key::J) }).unwrap();
        s.save(&file);
        assert_eq!(Shortcuts::load(&file), s);
        for junk in ["not json", "[1,2]", "{\"edit.undo\": 5}", "{\"edit.undo\":\"Ctrl+A\"}", "{\"edit.undo\":\"Q\"}", "{\"nothing\":\"Ctrl+Q\"}"] {
            std::fs::write(&file, junk).unwrap();
            assert_eq!(Shortcuts::load(&file), Shortcuts::default(), "{junk}");
        }
        std::fs::write(&file, vec![b' '; 70_000]).unwrap();
        assert_eq!(Shortcuts::load(&file), Shortcuts::default());
        assert_eq!(Shortcuts::load(&dir.join("missing.json")), Shortcuts::default());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
