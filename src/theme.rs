//! Colours from the current Omarchy theme, so OMAESTRO matches the rest of the desktop.
//!
//! Omarchy writes the active theme's palette to `~/.local/state/omarchy/current/theme/colors.toml`.
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

/// Keeps the theme in step with Omarchy's current one.
pub struct ThemeFile {
    path: PathBuf,
    checked: bool,
    modified: Option<SystemTime>,
}

impl ThemeFile {
    pub fn omarchy() -> ThemeFile {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_default();
        ThemeFile {
            path: home.join(".local/state/omarchy/current/theme/colors.toml"),
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
    }
}
