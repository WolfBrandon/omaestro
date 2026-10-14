//! Colours from an Omarchy theme: the one Omarchy is using (the default, so OMAESTRO matches
//! the rest of the desktop), or any installed theme picked in Settings.
//!
//! Omarchy writes the active theme's palette to `~/.local/state/omarchy/current/theme/colors.toml`;
//! installed themes are folders in `/usr/share/omarchy/themes` and `~/.config/omarchy/themes`.
//! Every theme names the same slots (accent, selection, foreground…), and the OTIS design uses
//! those slots, so any theme works. The file is re-read when it changes, so switching themes
//! applies while the player is open. Without the file, PhosphorWolf's colours are used.

use ratatui::style::Color;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::SystemTime;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Theme {
    /// Play button, the ▶ marker, slider fill.
    pub accent: Color,
    /// Pane borders and the selected-row bar.
    pub selection: Color,
    /// The screen behind everything (`background`). Painted only for a theme picked in
    /// Settings; Omarchy's own theme is the terminal's background already.
    pub background: Color,
    /// Browse panels and popups (`lighter_background`).
    pub panel: Color,
    /// Bars set into a popup, such as column headings (`darker_background`).
    pub darker: Color,
    /// Labels, column headings, times, played rows (`dark_foreground`).
    pub dim: Color,
    /// Secondary text such as artist names (`foreground`).
    pub fg: Color,
    /// Normal text such as titles (`light_foreground`).
    pub light: Color,
    /// The selected or playing row (`bright_foreground`).
    pub bright: Color,
    pub red: Color,
    pub yellow: Color,
    pub orange: Color,
}

impl Default for Theme {
    /// PhosphorWolf.
    fn default() -> Self {
        Theme {
            accent: rgb(0x64ce8d),
            selection: rgb(0x19482b),
            background: rgb(0x050a06),
            panel: rgb(0x0c1c10),
            darker: rgb(0x020403),
            dim: rgb(0x609174),
            fg: rgb(0x83c99b),
            light: rgb(0xa1dfb4),
            bright: rgb(0xc1e8cb),
            red: rgb(0xdf7c87),
            yellow: rgb(0xd3b76b),
            orange: rgb(0xce9869),
        }
    }
}

fn rgb(hex: u32) -> Color {
    Color::Rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

/// "#64ce8d" → a colour. Anything else (such as Hyprland's "rgba(…)") is skipped.
fn parse_hex(s: &str) -> Option<Color> {
    let hex = s.strip_prefix('#')?;
    (hex.len() == 6)
        .then(|| u32::from_str_radix(hex, 16).ok())
        .flatten()
        .map(rgb)
}

impl Theme {
    /// Reads a colors.toml. Slots it doesn't have keep PhosphorWolf's colour.
    pub fn from_toml(text: &str) -> Theme {
        let slots: HashMap<String, toml::Value> = toml::from_str(text).unwrap_or_default();
        let get = |name: &str| {
            slots
                .get(name)
                .and_then(toml::Value::as_str)
                .and_then(parse_hex)
        };
        let d = Theme::default();
        Theme {
            accent: get("accent").unwrap_or(d.accent),
            selection: get("selection").unwrap_or(d.selection),
            background: get("background").unwrap_or(d.background),
            panel: get("lighter_background").unwrap_or(d.panel),
            darker: get("darker_background")
                .or_else(|| get("background"))
                .unwrap_or(d.darker),
            dim: get("dark_foreground").unwrap_or(d.dim),
            fg: get("foreground").unwrap_or(d.fg),
            light: get("light_foreground").unwrap_or(d.light),
            bright: get("bright_foreground").unwrap_or(d.bright),
            red: get("red").unwrap_or(d.red),
            yellow: get("yellow").unwrap_or(d.yellow),
            // A few themes have no orange.
            orange: get("orange").or_else(|| get("yellow")).unwrap_or(d.orange),
        }
    }
}

/// The theme setting that means "whatever Omarchy is using".
pub const FOLLOW: &str = "omarchy";

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default()
}

/// Where themes are installed: Omarchy's own, then yours (which replace Omarchy's of the
/// same name).
fn theme_dirs() -> Vec<PathBuf> {
    vec![
        PathBuf::from("/usr/share/omarchy/themes"),
        home().join(".config/omarchy/themes"),
    ]
}

/// An installed Omarchy theme.
#[derive(Debug, Clone, PartialEq)]
pub struct Installed {
    /// Its folder's name, as saved in the settings: "tokyo-night".
    pub slug: String,
    /// "Tokyo Night".
    pub name: String,
    /// A light theme (dark text on a pale background).
    pub light: bool,
    pub path: PathBuf,
}

/// "tokyo-night" → "Tokyo Night".
pub fn nice(slug: &str) -> String {
    slug.split(['-', '_'])
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut c = w.chars();
            c.next()
                .map(|f| f.to_uppercase().chain(c).collect::<String>())
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The installed themes, by name. Folders without a colors.toml are left out.
pub fn installed() -> Vec<Installed> {
    installed_in(&theme_dirs())
}

fn installed_in(dirs: &[PathBuf]) -> Vec<Installed> {
    let mut found = std::collections::BTreeMap::new();
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path().join("colors.toml");
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            let slug = entry.file_name().to_string_lossy().into_owned();
            let light = is_light(&text);
            found.insert(
                slug.clone(),
                Installed {
                    name: nice(&slug),
                    slug,
                    light,
                    path,
                },
            );
        }
    }
    found.into_values().collect()
}

/// A theme says `mode = "light"`; failing that, a pale background means light.
fn is_light(text: &str) -> bool {
    let slots: HashMap<String, toml::Value> = toml::from_str(text).unwrap_or_default();
    match slots.get("mode").and_then(toml::Value::as_str) {
        Some(mode) => mode == "light",
        None => match Theme::from_toml(text).background {
            Color::Rgb(r, g, b) => 299 * r as u32 + 587 * g as u32 + 114 * b as u32 > 140_000,
            _ => false,
        },
    }
}

/// The name of the theme Omarchy is using now ("Phosphor Wolf").
pub fn omarchy_current() -> Option<String> {
    let name =
        std::fs::read_to_string(home().join(".local/state/omarchy/current/theme.name")).ok()?;
    Some(nice(name.trim()))
}

/// Keeps the theme in step with its file: Omarchy's current theme, or the one picked.
pub struct ThemeFile {
    path: PathBuf,
    checked: bool,
    modified: Option<SystemTime>,
    /// A theme picked in Settings rather than Omarchy's, so OMAESTRO paints its background.
    pub fixed: bool,
}

impl ThemeFile {
    /// For a theme setting: "omarchy", or an installed theme's folder name. A theme that
    /// isn't installed (any more) falls back to following Omarchy.
    pub fn for_setting(setting: &str) -> ThemeFile {
        let picked = installed()
            .into_iter()
            .find(|t| t.slug == setting && setting != FOLLOW);
        ThemeFile {
            fixed: picked.is_some(),
            path: picked
                .map(|t| t.path)
                .unwrap_or_else(|| home().join(".local/state/omarchy/current/theme/colors.toml")),
            checked: false,
            modified: None,
        }
    }

    /// The theme, on the first call and whenever the file has changed since the last one.
    pub fn changed(&mut self) -> Option<Theme> {
        let modified = std::fs::metadata(&self.path)
            .and_then(|m| m.modified())
            .ok();
        if self.checked && modified == self.modified {
            return None;
        }
        self.checked = true;
        self.modified = modified;
        Some(
            std::fs::read_to_string(&self.path)
                .map(|text| Theme::from_toml(&text))
                .unwrap_or_default(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_omarchy_slots_and_keeps_defaults_for_missing_ones() {
        let theme = Theme::from_toml(
            "mode = \"dark\"\naccent = \"#7aa2f7\"\nselection = \"#292e42\"\n\
             yellow = \"#e0af68\"\nhyprland_active_border = \"rgba(64ce8dcc)\"\n",
        );
        assert_eq!(theme.accent, Color::Rgb(0x7a, 0xa2, 0xf7));
        assert_eq!(theme.selection, Color::Rgb(0x29, 0x2e, 0x42));
        assert_eq!(
            theme.orange, theme.yellow,
            "no orange: falls back to yellow"
        );
        assert_eq!(theme.bright, Theme::default().bright);
        assert_eq!(Theme::from_toml("not toml ["), Theme::default());
        let tokyo = Theme::from_toml("background = \"#1a1b26\"\n");
        assert_eq!(tokyo.background, Color::Rgb(0x1a, 0x1b, 0x26));
    }

    #[test]
    fn finds_installed_themes_yours_first_and_names_them() {
        let root = std::env::temp_dir().join(format!("omaestro-themes-{}", std::process::id()));
        let (system, mine) = (root.join("system"), root.join("mine"));
        let theme = |dir: &PathBuf, slug: &str, body: &str| {
            std::fs::create_dir_all(dir.join(slug)).unwrap();
            if !body.is_empty() {
                std::fs::write(dir.join(slug).join("colors.toml"), body).unwrap();
            }
        };
        theme(
            &system,
            "tokyo-night",
            "mode = \"dark\"\nbackground = \"#1a1b26\"\n",
        );
        theme(&system, "catppuccin-latte", "background = \"#eff1f5\"\n");
        theme(&system, "phosphor-wolf", "mode = \"dark\"\n");
        theme(
            &mine,
            "phosphor-wolf",
            "mode = \"dark\"\naccent = \"#64ce8d\"\n",
        );
        theme(&mine, "aether", "");
        let found = installed_in(&[system.clone(), mine.clone()]);
        let names: Vec<&str> = found.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["Catppuccin Latte", "Phosphor Wolf", "Tokyo Night"]);
        assert!(found[0].light, "a pale background without a mode is light");
        assert!(!found[2].light);
        assert!(found[1].path.starts_with(&mine), "yours replaces Omarchy's");
        assert_eq!(nice("retro-82"), "Retro 82");
        std::fs::remove_dir_all(root).unwrap();

        // A theme that isn't installed follows Omarchy.
        assert!(!ThemeFile::for_setting("no-such-theme").fixed);
        assert!(!ThemeFile::for_setting(FOLLOW).fixed);
    }
}
