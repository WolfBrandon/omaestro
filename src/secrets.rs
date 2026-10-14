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
    config::private_dir(&config::config_dir())?;
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

/// `n` random characters from `alphabet`, from the kernel's random source. An error if that
/// can't be read: a "random" string that's always the same would be easy to guess.
pub fn random_string(n: usize, alphabet: &[u8]) -> Result<String> {
    use std::io::Read;
    let mut source = std::fs::File::open("/dev/urandom").context("opening /dev/urandom")?;
    // Bytes above the last whole round of the alphabet are skipped, so that every character
    // is equally likely.
    let limit = 256 - 256 % alphabet.len();
    let mut out = String::with_capacity(n);
    let mut bytes = [0u8; 64];
    while out.len() < n {
        source
            .read_exact(&mut bytes)
            .context("reading /dev/urandom")?;
        for b in bytes.iter().map(|b| *b as usize).filter(|b| *b < limit) {
            if out.len() < n {
                out.push(alphabet[b % alphabet.len()] as char);
            }
        }
    }
    Ok(out)
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

/// Puts text on the clipboard (`wl-copy`, Wayland's clipboard tool).
pub fn copy_to_clipboard(text: &str) -> Result<()> {
    use std::io::Write;
    let mut child = std::process::Command::new("wl-copy")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .context("copying with wl-copy")?;
    child
        .stdin
        .take()
        .context("no input to wl-copy")?
        .write_all(text.as_bytes())?;
    child.wait()?;
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
        let a = random_string(64, b"abc").unwrap();
        assert_eq!(a.len(), 64);
        assert!(a.chars().all(|c| "abc".contains(c)));
        assert_ne!(
            random_string(32, b"0123456789").unwrap(),
            random_string(32, b"0123456789").unwrap()
        );
        // Every character turns up, about as often as the others.
        let many = random_string(30_000, b"abc").unwrap();
        for c in ['a', 'b', 'c'] {
            let n = many.chars().filter(|x| *x == c).count();
            assert!((9_000..11_000).contains(&n), "{c}: {n}");
        }
    }
}
