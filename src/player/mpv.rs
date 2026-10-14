//! Runs mpv in the background and talks to it over its JSON socket.
//!
//! mpv does the decoding, gapless playback, ReplayGain and output-device handling.
//! Commands go out as one JSON line each. Replies and events come back on a reader thread
//! and are forwarded as `MpvEvent`s.

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// Properties OMAESTRO watches. mpv sends a `property-change` event whenever one changes.
pub const OBSERVED: &[&str] = &[
    "path",
    "pause",
    "time-pos",
    "duration",
    "idle-active",
    "audio-params",
    "audio-device",
    "audio-device-list",
    "volume",
];

#[derive(Debug, Clone, PartialEq)]
pub enum MpvEvent {
    /// A watched property changed. `data` is `Null` when it has no value (e.g. nothing playing).
    Property { name: String, data: Value },
    /// Any other mpv event, such as "start-file" or "end-file".
    Event {
        name: String,
        reason: Option<String>,
    },
    /// The socket closed: mpv quit or crashed.
    Gone,
}

pub struct Options {
    /// mpv device name such as "pipewire/alsa_output.usb-…", or "" for the default.
    pub audio_device: String,
    /// "off", "track" or "album".
    pub replaygain: String,
    /// Run songs together without a gap.
    pub gapless: bool,
    /// 0 to 100.
    pub volume: u8,
    /// Play into mpv's null output: nothing reaches the speakers. Used by tests.
    pub silent: bool,
}

pub struct Mpv {
    child: Child,
    writer: UnixStream,
    socket: PathBuf,
    next_id: u64,
}

impl Mpv {
    /// Starts mpv and connects to it. Events are sent to `events` until mpv exits.
    pub fn start(
        options: &Options,
        socket: &Path,
        events: impl Fn(MpvEvent) -> bool + Send + 'static,
    ) -> Result<Mpv> {
        if let Some(dir) = socket.parent() {
            crate::config::private_dir(dir)?;
        }
        // Someone answering on the socket means OMAESTRO is already open; taking the socket
        // over would cut that one off from its player.
        if UnixStream::connect(socket).is_ok() {
            bail!(
                "OMAESTRO is already running (its player answers on {})",
                socket.display()
            );
        }
        // A socket left over from a crash would make mpv fail to listen.
        let _ = std::fs::remove_file(socket);

        let replaygain = match options.replaygain.as_str() {
            "track" | "album" => options.replaygain.as_str(),
            _ => "no",
        };
        let mut cmd = Command::new("mpv");
        cmd.args([
            // Ignore ~/.config/mpv and /etc/mpv/scripts, so mpv's own MPRIS plugin doesn't add
            // a second player to the bar and nothing in mpv.conf overrides OMAESTRO's settings.
            "--no-config",
            // Without this, mpv reads keys from the terminal and fights the TUI for it.
            "--no-terminal",
            "--idle=yes",
            "--no-video",
            // OMAESTRO looks up YouTube streams itself (see youtube.rs); mpv shouldn't.
            "--ytdl=no",
            "--audio-client-name=omaestro",
            // Start opening the next file early; files come over NFS.
            "--prefetch-playlist=yes",
            // 100 is the file's own level; mpv would otherwise allow boosting to 130.
            "--volume-max=100",
        ]);
        cmd.arg(format!("--replaygain={replaygain}"));
        cmd.arg(format!(
            "--gapless-audio={}",
            if options.gapless { "weak" } else { "no" }
        ));
        cmd.arg(format!("--volume={}", options.volume.min(100)));
        cmd.arg(format!("--input-ipc-server={}", socket.display()));
        if options.silent {
            cmd.arg("--ao=null");
        } else {
            cmd.arg("--ao=pipewire");
            if !options.audio_device.is_empty() {
                cmd.arg(format!("--audio-device={}", options.audio_device));
            }
        }
        cmd.stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let mut child = cmd.spawn().context("starting mpv (is it installed?)")?;

        let stream = connect(socket, &mut child)?;
        let reader = stream.try_clone()?;
        std::thread::spawn(move || read_events(reader, events));

        let mut mpv = Mpv {
            child,
            writer: stream,
            socket: socket.to_path_buf(),
            next_id: 1,
        };
        for (n, name) in OBSERVED.iter().enumerate() {
            mpv.command(&[json!("observe_property"), json!(n + 1), json!(name)])?;
        }
        Ok(mpv)
    }

    /// Sends one command, e.g. `["loadfile", "/path.flac", "append"]`. Doesn't wait for the reply.
    pub fn command(&mut self, args: &[Value]) -> Result<()> {
        let line = json!({ "command": args, "request_id": self.next_id });
        self.next_id += 1;
        writeln!(self.writer, "{line}").context("sending a command to mpv")?;
        Ok(())
    }

    /// Plays `path` now, dropping mpv's playlist.
    pub fn load_replace(&mut self, path: &str) -> Result<()> {
        self.command(&[json!("loadfile"), json!(path), json!("replace")])
    }

    /// Plays `path` now, starting `start` seconds in.
    pub fn load_replace_at(&mut self, path: &str, start: f64) -> Result<()> {
        if start < 0.5 {
            return self.load_replace(path);
        }
        self.command(&[
            json!("loadfile"),
            json!(path),
            json!("replace"),
            json!(-1),
            json!(format!("start={start:.1}")),
        ])
    }

    /// Adds `path` after whatever is playing, so mpv can move to it without a gap.
    pub fn load_append(&mut self, path: &str) -> Result<()> {
        self.command(&[json!("loadfile"), json!(path), json!("append")])
    }

    /// Removes everything from mpv's playlist except the file playing now.
    pub fn clear_upcoming(&mut self) -> Result<()> {
        self.command(&[json!("playlist-clear")])
    }

    pub fn set(&mut self, property: &str, value: Value) -> Result<()> {
        self.command(&[json!("set_property"), json!(property), value])
    }

    pub fn toggle_pause(&mut self) -> Result<()> {
        self.command(&[json!("cycle"), json!("pause")])
    }

    /// Seeks by `seconds`, backwards when negative.
    pub fn seek(&mut self, seconds: f64) -> Result<()> {
        self.command(&[json!("seek"), json!(seconds), json!("relative")])
    }

    pub fn stop(&mut self) -> Result<()> {
        self.command(&[json!("stop")])
    }
}

impl Drop for Mpv {
    fn drop(&mut self) {
        let _ = self.command(&[json!("quit")]);
        let deadline = Instant::now() + Duration::from_millis(500);
        while Instant::now() < deadline {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_file(&self.socket);
    }
}

/// mpv takes a moment to create its socket; retry for up to 3 seconds.
fn connect(socket: &Path, child: &mut Child) -> Result<UnixStream> {
    for _ in 0..60 {
        if let Ok(stream) = UnixStream::connect(socket) {
            return Ok(stream);
        }
        if let Ok(Some(status)) = child.try_wait() {
            bail!("mpv exited right away ({status})");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = child.kill();
    bail!("mpv didn't open its control socket at {}", socket.display())
}

/// Passes mpv's events on until mpv goes away or `events` says nobody is listening.
fn read_events(stream: UnixStream, events: impl Fn(MpvEvent) -> bool) {
    for line in BufReader::new(stream).lines() {
        let Ok(line) = line else { break };
        let Some(event) = parse(&line) else { continue };
        if !events(event) {
            return;
        }
    }
    events(MpvEvent::Gone);
}

/// Turns one line from mpv into an event. Command replies are dropped: OMAESTRO follows
/// mpv's state through the watched properties instead.
fn parse(line: &str) -> Option<MpvEvent> {
    let v: Value = serde_json::from_str(line).ok()?;
    let name = v.get("event")?.as_str()?.to_string();
    if name == "property-change" {
        return Some(MpvEvent::Property {
            name: v.get("name")?.as_str()?.to_string(),
            data: v.get("data").cloned().unwrap_or(Value::Null),
        });
    }
    let reason = v.get("reason").and_then(Value::as_str).map(str::to_string);
    Some(MpvEvent::Event { name, reason })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_property_changes_and_events() {
        let p = parse(r#"{"event":"property-change","id":3,"name":"time-pos","data":1.5}"#);
        assert_eq!(
            p,
            Some(MpvEvent::Property {
                name: "time-pos".into(),
                data: json!(1.5)
            })
        );
        let e = parse(r#"{"event":"end-file","reason":"eof","playlist_entry_id":1}"#);
        assert_eq!(
            e,
            Some(MpvEvent::Event {
                name: "end-file".into(),
                reason: Some("eof".into())
            })
        );
        assert_eq!(
            parse(r#"{"request_id":4,"error":"success","data":null}"#),
            None
        );
        assert_eq!(parse("not json"), None);
    }

    #[test]
    fn a_second_copy_refuses_to_take_over_the_player() {
        if !crate::testutil::have("mpv") {
            eprintln!("skipping: needs mpv");
            return;
        }
        let tmp = crate::testutil::TempDir::new("mpv-twice");
        let socket = tmp.0.join("mpv.sock");
        let options = Options {
            audio_device: String::new(),
            replaygain: "off".into(),
            gapless: true,
            volume: 100,
            silent: true,
        };
        let first = Mpv::start(&options, &socket, |_| true).unwrap();
        let second = Mpv::start(&options, &socket, |_| true);
        assert!(second.is_err_and(|e| e.to_string().contains("already running")));
        drop(first);
        // Once the first one has quit, starting again works.
        assert!(Mpv::start(&options, &socket, |_| true).is_ok());
    }
}
