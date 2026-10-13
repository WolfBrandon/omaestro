//! Sign-in details for Spotify and YouTube Music, kept in `~/.config/omaestro/` as small JSON
//! files that only you can read (permissions 600, like an SSH key). Deleting a file signs out.

use crate::config;
use anyhow::{Context, Result};
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;

pub fn path(name: &str) -> PathBuf {
    config::config_dir().join(name)
}

pub fn load<T: DeserializeOwned>(name: &str) -> Option<T> {
    let text = std::fs::read_to_string(path(name)).ok()?;
    serde_json::from_str(&text).ok()
}

pub fn save<T: Serialize>(name: &str, value: &T) -> Result<()> {
    let path = path(name);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&tmp)
        .with_context(|| format!("writing {}", tmp.display()))?;
    file.write_all(serde_json::to_string_pretty(value)?.as_bytes())?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

pub fn forget(name: &str) {
    let _ = std::fs::remove_file(path(name));
}

/// `n` random characters from `alphabet`, from the kernel's random source.
pub fn random_string(n: usize, alphabet: &[u8]) -> String {
    let mut bytes = vec![0u8; n];
    if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
        use std::io::Read;
        let _ = f.read_exact(&mut bytes);
    }
    bytes
        .iter()
        .map(|b| alphabet[*b as usize % alphabet.len()] as char)
        .collect()
}

/// Opens a web page in the default browser (`xdg-open`), without waiting for it.
pub fn open_in_browser(url: &str) -> Result<()> {
    std::process::Command::new("xdg-open")
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .context("opening the browser with xdg-open")?;
    Ok(())
}

/// Seconds since 1970, for token expiry times.
pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn random_strings_use_the_alphabet_and_differ() {
        let a = random_string(64, b"abc");
        assert_eq!(a.len(), 64);
        assert!(a.chars().all(|c| "abc".contains(c)));
        assert_ne!(
            random_string(32, b"0123456789"),
            random_string(32, b"0123456789")
        );
    }
}
