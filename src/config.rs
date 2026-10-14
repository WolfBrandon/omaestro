//! Settings from `~/.config/omaestro/config.toml`, and the standard folders OMAESTRO uses.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Folders scanned into the library. A leading `~` means your home folder.
    pub library_paths: Vec<String>,
    /// How many files to read tags from at once. NAS disks spend most of their time seeking,
    /// so more readers than CPU cores still helps.
    pub scan_threads: usize,
    /// Check the music folders for changes each time OMAESTRO opens.
    pub scan_on_launch: bool,
    pub playback: Playback,
    pub appearance: Appearance,
    /// The queue's columns: which show and how wide.
    pub queue: crate::columns::QueueColumns,
    pub outputs: Outputs,
    pub sonos: Sonos,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Playback {
    /// ReplayGain mode handed to mpv: "off", "track" or "album".
    pub replaygain: String,
    /// No silence between songs that run into each other (live albums, DJ mixes).
    pub gapless: bool,
    /// PipeWire device for local playback, e.g. "alsa_output.usb-…". Empty means the system default.
    pub audio_device: String,
    /// Where music plays: "" for this computer, or "sonos:<speaker id>" for a Sonos group.
    pub output: String,
    /// 0 to 100.
    pub volume: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Appearance {
    pub album_art: Art,
    pub visualizer: bool,
    /// "omarchy" to follow Omarchy's theme, or an installed theme's folder name.
    pub theme: String,
}

/// How the cover is drawn: as a real picture (sixel or the kitty protocol, in terminals that
/// can), as coloured blocks (anywhere), or not at all.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Art {
    LoRes,
    #[default]
    HiRes,
    Off,
}

impl Serialize for Art {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(match self {
            Art::LoRes => "lo-res",
            Art::HiRes => "hi-res",
            Art::Off => "off",
        })
    }
}

impl<'de> Deserialize<'de> for Art {
    /// "hi-res", "lo-res" or "off"; older settings files said true or false.
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Art, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Bool(bool),
            Text(String),
        }
        Ok(match Raw::deserialize(d)? {
            Raw::Bool(false) => Art::Off,
            Raw::Bool(true) => Art::HiRes,
            Raw::Text(t) => match t.as_str() {
                "off" => Art::Off,
                "lo-res" | "low" => Art::LoRes,
                _ => Art::HiRes,
            },
        })
    }
}

/// The output picker's lists: device ids (mpv names or Sonos RINCON ids).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Outputs {
    /// Starred: listed first.
    pub favourites: Vec<String>,
    /// Hidden unless "show all" is on.
    pub hidden: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Sonos {
    /// Speaker addresses to ask first. Any one answering lists the whole household; with
    /// none answering, OMAESTRO looks around the local network.
    pub speakers: Vec<String>,
    /// The port speakers fetch local files from. Allow it in the firewall for your network.
    pub media_port: u16,
    /// Speaker ids (RINCON_…) OMAESTRO must never send anything to. Bonded speakers are
    /// already left alone; this is an extra guard.
    pub never: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            library_paths: vec!["~/Music".into()],
            scan_threads: 16,
            scan_on_launch: true,
            playback: Playback::default(),
            appearance: Appearance::default(),
            queue: crate::columns::QueueColumns::default(),
            outputs: Outputs::default(),
            sonos: Sonos::default(),
        }
    }
}

impl Default for Playback {
    fn default() -> Self {
        Playback {
            replaygain: "album".into(),
            gapless: true,
            audio_device: String::new(),
            output: String::new(),
            volume: 100,
        }
    }
}

impl Default for Appearance {
    fn default() -> Self {
        Appearance {
            album_art: Art::HiRes,
            visualizer: true,
            theme: crate::theme::FOLLOW.to_string(),
        }
    }
}

impl Default for Sonos {
    fn default() -> Self {
        Sonos {
            speakers: Vec::new(),
            media_port: 47100,
            never: Vec::new(),
        }
    }
}

const HEADER: &str = "\
# OMAESTRO settings. Delete this file to get the defaults back.
# library_paths: folders to scan, e.g. [\"~/Music\", \"/mnt/nas/music\"]
# scan_threads:  files read at once while scanning (16 suits a NAS)
# replaygain:    \"off\", \"track\" or \"album\"
# Most of these can be changed in the app: the gear button or Settings.

";

impl Config {
    /// Reads the config file, writing a default one first if there isn't one yet.
    pub fn load() -> Result<Config> {
        let path = config_dir().join("config.toml");
        if !path.exists() {
            let config = Config::default();
            config.save()?;
            return Ok(config);
        }
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))
    }

    pub fn save(&self) -> Result<()> {
        let dir = config_dir();
        std::fs::create_dir_all(&dir)?;
        let text = format!("{HEADER}{}", toml::to_string_pretty(self)?);
        std::fs::write(dir.join("config.toml"), text)?;
        Ok(())
    }

    pub fn library_roots(&self) -> Vec<PathBuf> {
        self.library_paths.iter().map(|p| expand_tilde(p)).collect()
    }
}

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

/// `$XDG_<name>` if set, otherwise `~/<fallback>`, plus `omaestro`.
fn xdg_dir(name: &str, fallback: &str) -> PathBuf {
    std::env::var_os(name)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(fallback))
        .join("omaestro")
}

pub fn config_dir() -> PathBuf {
    xdg_dir("XDG_CONFIG_HOME", ".config")
}

/// Things worth keeping: playlists.
pub fn data_dir() -> PathBuf {
    xdg_dir("XDG_DATA_HOME", ".local/share")
}

/// Rebuildable data: the library index and cover thumbnails.
pub fn cache_dir() -> PathBuf {
    xdg_dir("XDG_CACHE_HOME", ".cache")
}

/// Sockets that only live while OMAESTRO runs.
pub fn runtime_dir() -> PathBuf {
    match std::env::var_os("XDG_RUNTIME_DIR") {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir).join("omaestro"),
        _ => cache_dir().join("run"),
    }
}

pub fn expand_tilde(path: &str) -> PathBuf {
    match path.strip_prefix("~") {
        Some(rest) => home().join(rest.trim_start_matches('/')),
        None => PathBuf::from(path),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn album_art_reads_old_and_new_settings() {
        let read = |text: &str| toml::from_str::<Appearance>(text).unwrap().album_art;
        assert_eq!(read("album_art = true"), Art::HiRes);
        assert_eq!(read("album_art = false"), Art::Off);
        assert_eq!(read("album_art = \"lo-res\""), Art::LoRes);
        assert_eq!(read(""), Art::HiRes);
        let written = toml::to_string(&Appearance {
            album_art: Art::LoRes,
            ..Appearance::default()
        })
        .unwrap();
        assert!(written.contains("album_art = \"lo-res\""), "{written}");
        // Older settings files have no theme: follow Omarchy's.
        let old: Appearance = toml::from_str("visualizer = true").unwrap();
        assert_eq!(old.theme, crate::theme::FOLLOW);
    }
}
