//! The app's menus, following each platform's conventions. On macOS they are native: Settings…
//! (Cmd+,) in the application menu, Find, Find Next and Find Previous in the Edit menu after the
//! standard text actions, the standard Window menu, and Help. Rune has no native menu bar on
//! Windows or Linux, so there the window draws the closest equivalent along its top: File (with
//! Settings on Windows), Edit (with Preferences on Linux, and Find), Window and Help. Both run the
//! same [`Command`]s as the keyboard shortcuts.

use rune::{MenuItem, Shortcut, Tx};

use crate::{App, Showing};

/// Where the project's documentation lives, for Help.
pub const HELP_URL: &str = "https://github.com/VinSpee/velvet-hammer#readme";

/// Something a menu item or a shortcut does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Settings,
    Find,
    FindNext,
    FindPrevious,
    Minimize,
    Help,
}

/// The platforms whose menu conventions differ.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    Mac,
    Windows,
    Linux,
}

impl Platform {
    pub fn current() -> Platform {
        if cfg!(target_os = "macos") {
            Platform::Mac
        } else if cfg!(target_os = "windows") {
            Platform::Windows
        } else {
            Platform::Linux
        }
    }

    /// The name of the settings destination: Settings, or Preferences on Linux, as GNOME and KDE
    /// apps say.
    pub fn settings_name(self) -> &'static str {
        match self {
            Platform::Linux => "Preferences",
            Platform::Mac | Platform::Windows => "Settings",
        }
    }

    /// How a shortcut with the primary modifier reads: `⌘,` on macOS, `Ctrl+,` elsewhere.
    fn chord(self, keys: &str) -> String {
        match self {
            Platform::Mac => keys
                .replace("Cmd+Shift+", "\u{21e7}\u{2318}")
                .replace("Cmd+", "\u{2318}"),
            Platform::Windows | Platform::Linux => keys.replace("Cmd+", "Ctrl+"),
        }
    }
}

/// One item of an in-window menu.
pub struct Item {
    pub id: &'static str,
    pub title: &'static str,
    /// Its shortcut as the platform writes it, if it has one.
    pub keys: Option<String>,
    pub command: Command,
}

/// The in-window menu bar for `platform`, menu by menu; a menu with no items is left out.
pub fn in_window(platform: Platform) -> Vec<(&'static str, Vec<Item>)> {
    let item = |id, title, keys: Option<&str>, command| Item {
        id,
        title,
        keys: keys.map(|k| platform.chord(k)),
        command,
    };
    let settings = item(
        "settings",
        platform.settings_name(),
        Some("Cmd+,"),
        Command::Settings,
    );
    let mut file = Vec::new();
    let mut edit = Vec::new();
    match platform {
        Platform::Linux => edit.push(settings),
        Platform::Mac | Platform::Windows => file.push(settings),
    }
    edit.extend([
        item("find", "Find\u{2026}", Some("Cmd+F"), Command::Find),
        item("find-next", "Find Next", Some("Cmd+G"), Command::FindNext),
        item(
            "find-previous",
            "Find Previous",
            Some("Cmd+Shift+G"),
            Command::FindPrevious,
        ),
    ]);
    let menus = vec![
        ("File", file),
        ("Edit", edit),
        (
            "Window",
            vec![item("minimize", "Minimize", None, Command::Minimize)],
        ),
        (
            "Help",
            vec![item("help", "Velvet Hammer Help", None, Command::Help)],
        ),
    ];
    menus
        .into_iter()
        .filter(|(_, items)| !items.is_empty())
        .collect()
}

/// The native menu items, for macOS and for headless runs, where scripts pick them by id.
/// `App::standard_menus` adds the Edit menu's text actions and the Window menu around them.
pub fn native() -> Vec<MenuItem<App>> {
    let keys = |chord: &str| chord.parse::<Shortcut>().expect("a valid shortcut");
    vec![
        MenuItem::new("Settings\u{2026}", |s: &mut App, tx: &mut Tx| {
            s.command(Command::Settings, tx)
        })
        .id("settings")
        .key_equivalent(keys("cmd+comma")),
        MenuItem::separator().menu("Edit").after("selectAll:"),
        MenuItem::new("Find\u{2026}", |s: &mut App, tx: &mut Tx| {
            s.command(Command::Find, tx)
        })
        .id("find")
        .menu("Edit")
        .after("selectAll:")
        .key_equivalent(keys("cmd+f")),
        MenuItem::new("Find Next", |s: &mut App, tx: &mut Tx| {
            s.command(Command::FindNext, tx)
        })
        .id("find-next")
        .menu("Edit")
        .after("selectAll:")
        .key_equivalent(keys("cmd+g")),
        MenuItem::new("Find Previous", |s: &mut App, tx: &mut Tx| {
            s.command(Command::FindPrevious, tx)
        })
        .id("find-previous")
        .menu("Edit")
        .after("selectAll:")
        .key_equivalent(keys("cmd+shift+g")),
        MenuItem::new("Velvet Hammer Help", |s: &mut App, tx: &mut Tx| {
            s.command(Command::Help, tx)
        })
        .id("help")
        .menu("Help"),
    ]
}

impl App {
    /// Runs a menu item's or a shortcut's command.
    pub fn command(&mut self, command: Command, tx: &mut Tx) {
        self.open_menu = None;
        match command {
            Command::Settings => self.showing = Showing::Settings,
            Command::Find => self.open_find(tx),
            Command::FindNext | Command::FindPrevious if !self.find.open => self.open_find(tx),
            Command::FindNext => self.step_find(1),
            Command::FindPrevious => self.step_find(-1),
            Command::Minimize => tx.minimize(),
            Command::Help => {
                let _ = tx.open_url(HELP_URL);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn titles(platform: Platform) -> Vec<(&'static str, Vec<String>)> {
        in_window(platform)
            .into_iter()
            .map(|(menu, items)| {
                let items = items
                    .iter()
                    .map(|i| match &i.keys {
                        Some(keys) => format!("{} {keys}", i.title),
                        None => i.title.to_string(),
                    })
                    .collect();
                (menu, items)
            })
            .collect()
    }

    #[test]
    fn windows_puts_settings_in_file_and_linux_puts_preferences_in_edit() {
        let windows = titles(Platform::Windows);
        assert_eq!(windows[0], ("File", vec!["Settings Ctrl+,".to_string()]));
        assert_eq!(
            windows[1].1,
            [
                "Find\u{2026} Ctrl+F",
                "Find Next Ctrl+G",
                "Find Previous Ctrl+Shift+G"
            ]
        );
        let linux = titles(Platform::Linux);
        let menus: Vec<_> = linux.iter().map(|(m, _)| *m).collect();
        assert_eq!(menus, ["Edit", "Window", "Help"], "no empty File menu");
        assert_eq!(linux[0].1[0], "Preferences Ctrl+,");
        assert_eq!(
            titles(Platform::Mac)[1].1[2],
            "Find Previous \u{21e7}\u{2318}G"
        );
    }

    #[test]
    fn the_native_items_shortcuts_parse() {
        assert_eq!(native().len(), 6);
    }
}
