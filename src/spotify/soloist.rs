//! Spotify Soloist: Spotify's own headless player for Linux, which plays Spotify on this
//! computer. OMAESTRO downloads it into `~/.local/share/omaestro/soloist/` (it isn't in the Arch
//! repositories), runs it in the background and steers it over a local WebSocket.
//!
//! It needs two things from you, once:
//! - an API key from https://developer.spotify.com/dashboard/soloist (Premium accounts only),
//!   kept in `~/.config/omaestro/soloist.json`;
//! - pairing: while OMAESTRO runs `soloist --pair`, pick "OMAESTRO" in the Spotify app's device
//!   list. Soloist remembers the account in its data folder.
//!
//! Each Soloist build stops working 90 days after it was made (it exits with code 10); then
//! OMAESTRO downloads a fresh one.

use crate::config;
use crate::secrets;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Read};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

const FILE: &str = "soloist.json";
const DOWNLOAD: &str = "https://soloist-builds.spotifycdn.com/soloist_release_x86_64.tar.gz";
pub const KEY_PAGE: &str = "https://developer.spotify.com/dashboard/soloist";
/// The name Soloist shows in Spotify's device list.
pub const DEVICE_NAME: &str = "OMAESTRO";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Key {
    pub api_key: String,
}

pub fn dir() -> PathBuf {
    config::data_dir().join("soloist")
}

pub fn binary() -> PathBuf {
    dir().join("soloist")
}

/// Soloist's own folder for its device identity and saved session.
pub fn data_dir() -> PathBuf {
    dir().join("data")
}

pub fn cache_dir() -> PathBuf {
    config::cache_dir().join("soloist")
}

pub fn key() -> Option<String> {
    secrets::load::<Key>(FILE).map(|k| k.api_key)
}

pub fn save_key(api_key: &str) -> Result<()> {
    let api_key = api_key.trim();
    if api_key.len() < 16 || api_key.contains(char::is_whitespace) {
        bail!("That doesn't look like a Soloist API key");
    }
    secrets::save(
        FILE,
        &Key {
            api_key: api_key.to_string(),
        },
    )
}

pub fn forget() {
    secrets::forget(FILE);
    let _ = std::fs::remove_dir_all(data_dir());
}

pub fn installed() -> bool {
    binary().is_file()
}

/// Paired: Soloist has saved a session in its data folder.
pub fn paired() -> bool {
    std::fs::read_dir(data_dir()).is_ok_and(|mut entries| {
        entries.any(|e| {
            e.ok().is_some_and(|e| {
                let name = e.file_name();
                let name = name.to_string_lossy();
                !matches!(name.as_ref(), "soloist.pid" | "ws.addr" | "ws.port")
            })
        })
    })
}

/// What setting up reports while it works.
#[derive(Debug, Clone, PartialEq)]
pub enum Setup {
    Progress(String),
    Done(Result<(), String>),
}

/// Downloads Soloist (if needed) and pairs it. Runs on a background thread; reports through
/// `report`. Pairing waits until you pick OMAESTRO in the Spotify app, or 10 minutes.
pub fn set_up(report: impl Fn(Setup) + Send + 'static) {
    std::thread::spawn(move || {
        let result = (|| -> Result<()> {
            let key = key().context("No Soloist API key yet")?;
            if !installed() {
                report(Setup::Progress("Downloading Spotify Soloist…".into()));
                download()?;
            }
            report(Setup::Progress(format!(
                "Open Spotify on your phone or computer, tap the devices icon and pick \
                 \"{DEVICE_NAME}\""
            )));
            pair(&key)
        })();
        report(Setup::Done(result.map_err(|e| format!("{e:#}"))));
    });
}

/// Fetches the current build and unpacks the `soloist` program into OMAESTRO's folder.
pub fn download() -> Result<()> {
    let dir = dir();
    std::fs::create_dir_all(&dir)?;
    let archive = dir.join("soloist.tar.gz");
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(300)))
        .build()
        .into();
    let mut res = agent.get(DOWNLOAD).call().context("downloading Soloist")?;
    let mut bytes = Vec::new();
    res.body_mut()
        .with_config()
        .limit(500 * 1024 * 1024)
        .reader()
        .read_to_end(&mut bytes)?;
    std::fs::write(&archive, &bytes)?;
    let status = Command::new("tar")
        .arg("-xzf")
        .arg(&archive)
        .arg("-C")
        .arg(&dir)
        .status()
        .context("unpacking Soloist with tar")?;
    let _ = std::fs::remove_file(&archive);
    if !status.success() || !installed() {
        bail!("The Soloist download didn't contain the soloist program");
    }
    Ok(())
}

fn base_command(key: &str) -> Command {
    let mut cmd = Command::new(binary());
    cmd.arg("-n")
        .arg(DEVICE_NAME)
        .arg("-k")
        .arg(key)
        .arg("-D")
        .arg(data_dir())
        .arg("-C")
        .arg(cache_dir());
    cmd
}

/// Runs `soloist --pair`, which waits for you to pick the device in the Spotify app, saves
/// the session and exits.
fn pair(key: &str) -> Result<()> {
    std::fs::create_dir_all(data_dir())?;
    std::fs::create_dir_all(cache_dir())?;
    let mut child = base_command(key)
        .arg("--pair")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .context("starting Soloist")?;
    let log = last_lines(child.stderr.take());
    let deadline = std::time::Instant::now() + Duration::from_secs(600);
    loop {
        if let Some(status) = child.try_wait()? {
            return match status.code() {
                Some(0) => Ok(()),
                Some(10) => {
                    let _ = std::fs::remove_file(binary());
                    bail!("This Soloist build has expired; set up again to download a fresh one")
                }
                _ => bail!("Soloist stopped: {}", log.join().unwrap_or_default()),
            };
        }
        if std::time::Instant::now() > deadline {
            let _ = child.kill();
            bail!("Pairing timed out after 10 minutes");
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// Collects the last few lines a program writes, for error messages.
fn last_lines(pipe: Option<impl Read + Send + 'static>) -> std::thread::JoinHandle<String> {
    std::thread::spawn(move || {
        let mut lines = std::collections::VecDeque::new();
        if let Some(pipe) = pipe {
            for line in BufReader::new(pipe).lines().map_while(Result::ok) {
                lines.push_back(line);
                if lines.len() > 3 {
                    lines.pop_front();
                }
            }
        }
        Vec::from(lines).join(" / ")
    })
}

/// Starts the Soloist player in the background with its WebSocket on a free local port.
/// `device` is a PipeWire node name, or "" for the default output.
pub fn spawn(key: &str, device: &str) -> Result<Child> {
    std::fs::create_dir_all(data_dir())?;
    std::fs::create_dir_all(cache_dir())?;
    let _ = std::fs::remove_file(data_dir().join("ws.port"));
    let mut cmd = base_command(key);
    cmd.arg("--ws").arg("127.0.0.1:0");
    if !device.is_empty() {
        cmd.arg("-d").arg(device);
    }
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("starting Soloist")
}

/// The WebSocket port a just-started Soloist chose; waits up to 10 s for it.
pub fn ws_port(child: &mut Child) -> Result<u16> {
    let file = data_dir().join("ws.port");
    for _ in 0..100 {
        if let Ok(text) = std::fs::read_to_string(&file)
            && let Ok(port) = text.trim().parse()
        {
            return Ok(port);
        }
        if let Some(status) = child.try_wait()? {
            if status.code() == Some(10) {
                let _ = std::fs::remove_file(binary());
                bail!("The Soloist build expired; set Spotify up again in Settings to refresh it");
            }
            bail!("Soloist stopped right away ({status})");
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    bail!("Soloist didn't open its control port")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_a_key_that_is_clearly_wrong() {
        assert!(save_key("short").is_err());
        assert!(save_key("has spaces in the middle of it").is_err());
    }
}
