//! The Settings screen (the gear button): one page of rows grouped into sections, as in the
//! OTIS design's Settings tab. Changes are saved to `~/.config/omaestro/config.toml` straight
//! away; the player picks up the ones that affect it (ReplayGain, gapless) at once.

use crate::app::{Action, group_thousands};
use crate::browse::{movement, step};
use crate::config::{Art, Config};
use crate::library::Source;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::cell::Cell;

pub const SECTIONS: [&str; 8] = [
    "Library",
    "Playback",
    "Outputs",
    "Sources",
    "Appearance",
    "Behaviour",
    "Keys",
    "About",
];

/// Which setting a row is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Id {
    Folder(usize),
    AddFolder,
    ScanOnLaunch,
    Readers,
    Index,
    ReplayGain,
    Gapless,
    OutputRate,
    OnQuit,
    Sonos,
    MediaPort,
    Local,
    Spotify,
    Soloist,
    YouTube,
    Theme,
    AlbumArt,
    Visualizer,
    Minimize,
    Info,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// Pick one: the options and which is chosen.
    Choice(Vec<&'static str>, usize),
    /// A number changed with ←→.
    Number(u32),
    Text(String),
    /// Pick one of many, shown one at a time (◂ name ▸): what each is called, what's saved
    /// for it, and which is chosen.
    Pick {
        options: Vec<String>,
        values: Vec<String>,
        chosen: usize,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub section: usize,
    pub label: &'static str,
    pub id: Id,
    pub value: Value,
    /// Dim text after the value.
    pub note: String,
    /// A button at the end of the row, if any.
    pub button: Option<&'static str>,
}

impl Row {
    fn new(section: usize, label: &'static str, id: Id, value: Value) -> Row {
        Row {
            section,
            label,
            id,
            value,
            note: String::new(),
            button: None,
        }
    }

    fn note(mut self, note: impl Into<String>) -> Row {
        self.note = note.into();
        self
    }

    fn button(mut self, label: &'static str) -> Row {
        self.button = Some(label);
        self
    }

    /// Rows that can be changed or have a button.
    pub fn active(&self) -> bool {
        self.button.is_some()
            || matches!(
                self.value,
                Value::Choice(..) | Value::Number(_) | Value::Pick { .. }
            )
    }
}

/// What the settings show about things outside the config file.
#[derive(Debug, Clone, Default)]
pub struct Facts {
    pub tracks: usize,
    pub albums: usize,
    pub spotify: Option<String>,
    pub soloist: String,
    pub soloist_ready: bool,
    pub youtube: Option<String>,
    pub sonos: String,
    /// The Omarchy themes installed, and the one Omarchy is using.
    pub themes: Vec<crate::theme::Installed>,
    pub omarchy_theme: Option<String>,
}

fn on_off(on: bool) -> Value {
    Value::Choice(vec!["On", "Off"], usize::from(!on))
}

pub fn rows(config: &Config, facts: &Facts) -> Vec<Row> {
    let mut rows = Vec::new();
    // Library
    for (i, folder) in config.library_paths.iter().enumerate() {
        let label = if i == 0 { "Music folders" } else { "" };
        rows.push(Row::new(0, label, Id::Folder(i), Value::Text(folder.clone())).button("Remove"));
    }
    let label = if config.library_paths.is_empty() {
        "Music folders"
    } else {
        ""
    };
    rows.push(Row::new(0, label, Id::AddFolder, Value::Text(String::new())).button("+ Add folder"));
    rows.push(Row::new(
        0,
        "Scan on launch",
        Id::ScanOnLaunch,
        on_off(config.scan_on_launch),
    ));
    rows.push(
        Row::new(
            0,
            "Readers at once",
            Id::Readers,
            Value::Number(config.scan_threads as u32),
        )
        .note("files read together; 16 suits a NAS"),
    );
    rows.push(
        Row::new(
            0,
            "Index",
            Id::Index,
            Value::Text(format!(
                "{} tracks · {} albums",
                group_thousands(facts.tracks),
                group_thousands(facts.albums)
            )),
        )
        .button("Rescan now"),
    );
    // Playback
    let rg = match config.playback.replaygain.as_str() {
        "off" => 0,
        "track" => 1,
        _ => 2,
    };
    rows.push(
        Row::new(
            1,
            "ReplayGain",
            Id::ReplayGain,
            Value::Choice(vec!["Off", "Track", "Album"], rg),
        )
        .note("evens out loudness"),
    );
    rows.push(Row::new(
        1,
        "Gapless",
        Id::Gapless,
        on_off(config.playback.gapless),
    ));
    rows.push(
        Row::new(
            1,
            "Output rate",
            Id::OutputRate,
            Value::Text("Set by PipeWire".into()),
        )
        .note("the badge shows → 48k if it resamples"),
    );
    rows.push(Row::new(
        1,
        "When I quit",
        Id::OnQuit,
        Value::Text("Stop the music".into()),
    ));
    // Outputs
    rows.push(
        Row::new(2, "Sonos", Id::Sonos, Value::Text(facts.sonos.clone())).button("Look again"),
    );
    rows.push(
        Row::new(
            2,
            "Media port",
            Id::MediaPort,
            Value::Text(config.sonos.media_port.to_string()),
        )
        .note("allow it in ufw for Sonos"),
    );
    // Sources
    let folders = config.library_paths.join(", ");
    rows.push(
        Row::new(3, "Local", Id::Local, Value::Text(folders))
            .note(format!("{} tracks", group_thousands(facts.tracks))),
    );
    rows.push(match &facts.spotify {
        Some(user) => Row::new(
            3,
            "Spotify",
            Id::Spotify,
            Value::Text(format!("Connected as {user}")),
        )
        .button("Sign out"),
        None => Row::new(
            3,
            "Spotify",
            Id::Spotify,
            Value::Text("Not connected".into()),
        )
        .button("Connect"),
    });
    rows.push(
        Row::new(
            3,
            "  playback",
            Id::Soloist,
            Value::Text(facts.soloist.clone()),
        )
        .button(if facts.soloist_ready {
            "Remove"
        } else {
            "Set up"
        }),
    );
    rows.push(match &facts.youtube {
        Some(user) => Row::new(
            3,
            "YouTube Music",
            Id::YouTube,
            Value::Text(format!("Connected as {user}")),
        )
        .button("Sign out"),
        None => Row::new(
            3,
            "YouTube Music",
            Id::YouTube,
            Value::Text("Not connected".into()),
        )
        .button("Connect"),
    });
    // Appearance
    let follow = match &facts.omarchy_theme {
        Some(name) => format!("Omarchy's theme ({name})"),
        None => "Omarchy's theme".to_string(),
    };
    let mut options = vec![follow];
    let mut values = vec![crate::theme::FOLLOW.to_string()];
    for theme in &facts.themes {
        options.push(if theme.light {
            format!("{} (light)", theme.name)
        } else {
            theme.name.clone()
        });
        values.push(theme.slug.clone());
    }
    let chosen = values
        .iter()
        .position(|v| *v == config.appearance.theme)
        .unwrap_or(0);
    rows.push(
        Row::new(
            4,
            "Theme",
            Id::Theme,
            Value::Pick {
                options,
                values,
                chosen,
            },
        )
        .note("←→ to try others"),
    );
    rows.push(Row::new(
        4,
        "Album art",
        Id::AlbumArt,
        Value::Choice(
            vec!["On (lo-res)", "On (hi-res)", "Off"],
            match config.appearance.album_art {
                Art::LoRes => 0,
                Art::HiRes => 1,
                Art::Off => 2,
            },
        ),
    ));
    rows.push(
        Row::new(
            4,
            "Visualizer",
            Id::Visualizer,
            on_off(config.appearance.visualizer),
        )
        .note("on Sonos too, except Spotify"),
    );
    // Behaviour
    rows.push(
        Row::new(
            5,
            "Minimize to",
            Id::Minimize,
            Value::Text("special:omaestro".into()),
        )
        .note("m hides it; the tray brings it back"),
    );
    // Keys and About
    rows.push(Row::new(
        6,
        "All keys",
        Id::Info,
        Value::Text("press ? anywhere".into()),
    ));
    rows.push(Row::new(
        7,
        "Version",
        Id::Info,
        Value::Text(format!("OMAESTRO v{}", env!("CARGO_PKG_VERSION"))),
    ));
    rows.push(Row::new(
        7,
        "Designed with",
        Id::Info,
        Value::Text("OTIS · github.com/WolfBrandon/otis".into()),
    ));
    rows
}

/// What the app should do after a key in Settings.
#[derive(Debug, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum Outcome {
    Stay,
    Close,
    /// The config changed: save it and apply what the player needs.
    Saved,
    Do(Action),
    Connect(Source),
    SignOut(Source),
    SetUpSoloist,
    RemoveSoloist,
    FindSonos,
}

pub struct Settings {
    /// The selected row (an active one).
    pub sel: usize,
    pub top: Cell<usize>,
    /// Typing a new folder: the text so far.
    pub adding: Option<String>,
}

impl Settings {
    pub fn new(rows: &[Row]) -> Settings {
        Settings {
            sel: rows.iter().position(Row::active).unwrap_or(0),
            top: Cell::new(0),
            adding: None,
        }
    }

    pub fn section(&self, rows: &[Row]) -> usize {
        rows.get(self.sel).map_or(0, |r| r.section)
    }

    fn move_by(&mut self, rows: &[Row], delta: isize) {
        let active: Vec<usize> = (0..rows.len()).filter(|&i| rows[i].active()).collect();
        let here = active.iter().position(|&i| i == self.sel).unwrap_or(0);
        if let Some(&i) = active.get(step(here, active.len(), delta)) {
            self.sel = i;
        }
    }

    /// Jumps to a section's first changeable row (or its first row).
    pub fn go_to_section(&mut self, rows: &[Row], section: usize) {
        if let Some(i) = rows
            .iter()
            .position(|r| r.section == section && r.active())
            .or_else(|| rows.iter().position(|r| r.section == section))
        {
            self.sel = i;
        }
    }

    pub fn on_key(&mut self, key: KeyEvent, config: &mut Config, rows: &[Row]) -> Outcome {
        if let Some(text) = &mut self.adding {
            match key.code {
                KeyCode::Esc => self.adding = None,
                KeyCode::Enter => {
                    let folder = text.trim().to_string();
                    self.adding = None;
                    if !folder.is_empty() && !config.library_paths.contains(&folder) {
                        config.library_paths.push(folder);
                        return Outcome::Saved;
                    }
                }
                KeyCode::Backspace => {
                    text.pop();
                }
                KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => text.push(c),
                _ => {}
            }
            return Outcome::Stay;
        }
        if let Some(delta) = movement(key.code) {
            self.move_by(rows, delta);
            return Outcome::Stay;
        }
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => Outcome::Close,
            KeyCode::Tab | KeyCode::BackTab => {
                let here = self.section(rows);
                let next = if key.code == KeyCode::Tab {
                    (here + 1) % SECTIONS.len()
                } else {
                    (here + SECTIONS.len() - 1) % SECTIONS.len()
                };
                self.go_to_section(rows, next);
                Outcome::Stay
            }
            KeyCode::Left | KeyCode::Char('h') => self.change(config, rows, -1),
            KeyCode::Right | KeyCode::Char('l') => self.change(config, rows, 1),
            KeyCode::Enter | KeyCode::Char(' ') => self.press(config, rows),
            _ => Outcome::Stay,
        }
    }

    /// ←→ on a choice or number.
    pub fn change(&mut self, config: &mut Config, rows: &[Row], delta: i32) -> Outcome {
        let Some(row) = rows.get(self.sel) else {
            return Outcome::Stay;
        };
        match &row.value {
            Value::Choice(options, chosen) => {
                let n = options.len() as i32;
                let new = (*chosen as i32 + delta).rem_euclid(n) as usize;
                self.choose(config, row, new)
            }
            Value::Pick {
                options, chosen, ..
            } => {
                let n = options.len() as i32;
                let new = (*chosen as i32 + delta).rem_euclid(n) as usize;
                self.choose(config, row, new)
            }
            Value::Number(n) if row.id == Id::Readers => {
                config.scan_threads = (*n as i32 + delta).clamp(1, 64) as usize;
                Outcome::Saved
            }
            _ => Outcome::Stay,
        }
    }

    /// Sets a choice row to option `n` (also used by clicks).
    pub fn choose(&mut self, config: &mut Config, row: &Row, n: usize) -> Outcome {
        if let (Id::Theme, Value::Pick { values, .. }) = (row.id, &row.value) {
            let Some(theme) = values.get(n) else {
                return Outcome::Stay;
            };
            config.appearance.theme = theme.clone();
            return Outcome::Saved;
        }
        let on = n == 0;
        match row.id {
            Id::ScanOnLaunch => config.scan_on_launch = on,
            Id::Gapless => config.playback.gapless = on,
            Id::AlbumArt => {
                config.appearance.album_art = [Art::LoRes, Art::HiRes, Art::Off][n.min(2)]
            }
            Id::Visualizer => config.appearance.visualizer = on,
            Id::ReplayGain => {
                config.playback.replaygain = ["off", "track", "album"][n.min(2)].to_string()
            }
            _ => return Outcome::Stay,
        }
        Outcome::Saved
    }

    /// Enter: press the row's button, or step a choice on.
    pub fn press(&mut self, config: &mut Config, rows: &[Row]) -> Outcome {
        let Some(row) = rows.get(self.sel) else {
            return Outcome::Stay;
        };
        if let Value::Choice(..) | Value::Pick { .. } = row.value {
            return self.change(config, rows, 1);
        }
        match (row.id, row.button) {
            (Id::Folder(i), _) => {
                if i < config.library_paths.len() {
                    config.library_paths.remove(i);
                    self.sel = self.sel.min(rows.len().saturating_sub(2));
                    return Outcome::Saved;
                }
                Outcome::Stay
            }
            (Id::AddFolder, _) => {
                self.adding = Some(String::new());
                Outcome::Stay
            }
            (Id::Index, _) => Outcome::Do(Action::Rescan),
            (Id::Sonos, _) => Outcome::FindSonos,
            (Id::Spotify, Some("Sign out")) => Outcome::SignOut(Source::Spotify),
            (Id::Spotify, _) => Outcome::Connect(Source::Spotify),
            (Id::Soloist, Some("Remove")) => Outcome::RemoveSoloist,
            (Id::Soloist, _) => Outcome::SetUpSoloist,
            (Id::YouTube, Some("Sign out")) => Outcome::SignOut(Source::YouTube),
            (Id::YouTube, _) => Outcome::Connect(Source::YouTube),
            _ => Outcome::Stay,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(s: &mut Settings, config: &mut Config, code: KeyCode) -> Outcome {
        let rows = rows(config, &Facts::default());
        s.on_key(KeyEvent::from(code), config, &rows)
    }

    fn selected(s: &Settings, config: &Config) -> Id {
        rows(config, &Facts::default())[s.sel].id
    }

    #[test]
    fn moves_between_changeable_rows_and_changes_them() {
        let mut config = Config::default();
        let mut s = Settings::new(&rows(&config, &Facts::default()));
        assert_eq!(selected(&s, &config), Id::Folder(0));

        // Tab goes to the next section: Playback starts with ReplayGain.
        press(&mut s, &mut config, KeyCode::Tab);
        assert_eq!(selected(&s, &config), Id::ReplayGain);
        assert_eq!(press(&mut s, &mut config, KeyCode::Left), Outcome::Saved);
        assert_eq!(config.playback.replaygain, "track");
        press(&mut s, &mut config, KeyCode::Down);
        assert_eq!(selected(&s, &config), Id::Gapless);
        press(&mut s, &mut config, KeyCode::Enter);
        assert!(!config.playback.gapless);

        // Adding a folder: type it, Enter saves; Remove takes one out.
        s.go_to_section(&rows(&config, &Facts::default()), 0);
        press(&mut s, &mut config, KeyCode::Down);
        assert_eq!(selected(&s, &config), Id::AddFolder);
        press(&mut s, &mut config, KeyCode::Enter);
        for c in "/mnt/music".chars() {
            press(&mut s, &mut config, KeyCode::Char(c));
        }
        assert_eq!(press(&mut s, &mut config, KeyCode::Enter), Outcome::Saved);
        assert_eq!(config.library_paths, ["~/Music", "/mnt/music"]);
        s.sel = 0;
        assert_eq!(press(&mut s, &mut config, KeyCode::Enter), Outcome::Saved);
        assert_eq!(config.library_paths, ["/mnt/music"]);

        // Sources: Connect buttons hand over to the sign-in screens.
        s.go_to_section(&rows(&config, &Facts::default()), 3);
        assert_eq!(selected(&s, &config), Id::Spotify);
        assert_eq!(
            press(&mut s, &mut config, KeyCode::Enter),
            Outcome::Connect(Source::Spotify)
        );
        assert_eq!(press(&mut s, &mut config, KeyCode::Esc), Outcome::Close);
    }

    #[test]
    fn picks_a_theme_from_the_installed_ones() {
        let theme = |slug: &str, light| crate::theme::Installed {
            slug: slug.into(),
            name: crate::theme::nice(slug),
            light,
            path: std::path::PathBuf::new(),
        };
        let facts = Facts {
            themes: vec![theme("catppuccin-latte", true), theme("tokyo-night", false)],
            omarchy_theme: Some("Phosphor Wolf".into()),
            ..Facts::default()
        };
        let mut config = Config::default();
        let mut s = Settings::new(&rows(&config, &facts));
        s.go_to_section(&rows(&config, &facts), 4);
        let row = |config: &Config| {
            rows(config, &facts)
                .into_iter()
                .find(|r| r.id == Id::Theme)
                .unwrap()
        };
        let Value::Pick {
            options, chosen, ..
        } = row(&config).value
        else {
            panic!("the theme row picks one of many")
        };
        assert_eq!(
            options,
            [
                "Omarchy's theme (Phosphor Wolf)",
                "Catppuccin Latte (light)",
                "Tokyo Night"
            ]
        );
        assert_eq!(chosen, 0, "follows Omarchy unless told otherwise");

        // → tries the next theme, saving it; ← from the first goes round to the last.
        s.sel = rows(&config, &facts)
            .iter()
            .position(|r| r.id == Id::Theme)
            .unwrap();
        let rows_now = rows(&config, &facts);
        assert_eq!(s.change(&mut config, &rows_now, 1), Outcome::Saved);
        assert_eq!(config.appearance.theme, "catppuccin-latte");
        let rows_now = rows(&config, &facts);
        s.change(&mut config, &rows_now, -1);
        assert_eq!(config.appearance.theme, crate::theme::FOLLOW);
        let rows_now = rows(&config, &facts);
        s.change(&mut config, &rows_now, -1);
        assert_eq!(config.appearance.theme, "tokyo-night");
        // A theme that's gone shows as following Omarchy.
        config.appearance.theme = "uninstalled".into();
        let Value::Pick { chosen, .. } = row(&config).value else {
            panic!()
        };
        assert_eq!(chosen, 0);
    }
}
