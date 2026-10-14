//! Plays Spotify on this computer through Spotify Soloist (see `spotify/soloist.rs` for setup).
//! OMAESTRO starts Soloist when the first Spotify song plays here, and steers it over its
//! local WebSocket: one song at a time, from OMAESTRO's own queue.
//!
//! Soloist doesn't say when a song has finished, so the player watches for it: the position
//! reaching the end and stopping, or Soloist moving on to a song we didn't ask for.

use crate::spotify::soloist;
use anyhow::{Context, Result};
use serde_json::{Value, json};
use std::net::TcpStream;
use std::process::Child;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::time::Duration;
use tungstenite::{Message, WebSocket};

/// What Soloist reports. Fields are present when the message said something about them.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct State {
    /// "idle", "playing", "paused" or "buffering".
    pub status: Option<String>,
    pub uri: Option<String>,
    pub duration_ms: Option<f64>,
    /// The position at `at_ms` (wall-clock milliseconds since 1970), moving at `speed`.
    pub position_ms: Option<f64>,
    pub at_ms: Option<f64>,
    pub speed: Option<f64>,
    pub volume: Option<u8>,
    /// The stream's quality, if the message says.
    pub quality: Option<super::Quality>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    State(State),
    /// Signed out or never paired.
    LoggedOut,
    Error(String),
    /// Soloist stopped.
    Gone(String),
}

/// Reads one message from Soloist.
pub fn parse(text: &str) -> Option<Event> {
    let v: Value = serde_json::from_str(text).ok()?;
    let mut s = State::default();
    let item = |s: &mut State, item: &Value| {
        s.uri = item["uri"].as_str().map(str::to_string);
        s.duration_ms = item["decorations"]["playback"]["duration_ms"].as_f64();
    };
    s.quality = quality_in(&v);
    let position = |s: &mut State, p: &Value| {
        s.position_ms = p["position_ms"].as_f64();
        s.at_ms = p["timestamp_ms"].as_f64();
        s.speed = p["speed"].as_f64();
    };
    match v["type"].as_str()? {
        "playback_state" => {
            s.status = v["status"].as_str().map(str::to_string);
            item(&mut s, &v["item"]);
            position(&mut s, &v["position"]);
            s.volume = v["volume"].as_u64().map(|n| n as u8);
        }
        "track_changed" => item(&mut s, &v["item"]),
        "playback_changed" => s.status = v["status"].as_str().map(str::to_string),
        "position_sync" => position(&mut s, &v["position"]),
        "volume_changed" => s.volume = v["volume"].as_u64().map(|n| n as u8),
        "auth_state" if v["logged_in"] == false => return Some(Event::LoggedOut),
        "error" => {
            return Some(Event::Error(
                v["message"].as_str().unwrap_or("Soloist error").to_string(),
            ));
        }
        _ => return None,
    }
    Some(Event::State(s))
}

/// Soloist's word for the stream's quality. Its exact shape isn't documented, so this looks
/// anywhere in a message for an object with fields like "codec", "bitrate", "bit_depth",
/// "sample_rate", "hifi" or "audio_quality".
fn quality_in(v: &Value) -> Option<super::Quality> {
    const KEYS: [&str; 7] = [
        "codec",
        "bitrate",
        "bit_depth",
        "sample_rate",
        "hifi",
        "lossless",
        "audio_quality",
    ];
    match v {
        Value::Object(map) => {
            if KEYS.iter().any(|k| map.contains_key(*k))
                && let Some(q) = quality_from(map)
            {
                return Some(q);
            }
            map.values().find_map(quality_in)
        }
        Value::Array(list) => list.iter().find_map(quality_in),
        _ => None,
    }
}

fn quality_from(map: &serde_json::Map<String, Value>) -> Option<super::Quality> {
    let text = |k: &str| {
        map.get(k)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let number = |k: &str| map.get(k).and_then(Value::as_f64).filter(|n| *n > 0.0);
    let flag = |k: &str| map.get(k).and_then(Value::as_bool).unwrap_or(false);
    let codec = match text("codec") {
        c if c.is_empty() => text("format"),
        c => c,
    };
    let level = text("audio_quality").replace('_', " ").to_uppercase();
    let q = super::Quality {
        lossless: flag("hifi")
            || flag("lossless")
            || codec.to_lowercase().contains("flac")
            || level.contains("LOSSLESS")
            || level.contains("HIFI"),
        codec: super::codec_name(&codec),
        // Bits or kilobits per second, whichever it sent.
        kbps: number("bitrate").map(|b| if b >= 10_000.0 { b / 1000.0 } else { b }.round() as u32),
        bit_depth: number("bit_depth").map(|b| b as u8),
        sample_rate: number("sample_rate").map(|r| if r < 1000.0 { r * 1000.0 } else { r } as u32),
        level,
    };
    (q != super::Quality::default()).then_some(q)
}

/// A copy of Soloist's latest report on the song, for checking what it says.
fn keep_copy(text: &str) {
    if text.contains("\"playback_state\"") || text.contains("\"track_changed\"") {
        let _ = std::fs::write(crate::config::cache_dir().join("soloist-last.json"), text);
    }
}

/// The running Soloist and the line to it.
pub struct Soloist {
    child: Child,
    commands: Sender<Value>,
    /// The output it was started for ("" for the default); changing it means a restart.
    pub device: String,
}

impl Soloist {
    /// Starts Soloist on `device` (a PipeWire node name, or "" for the default output) and
    /// connects to it. Events go to `events`.
    pub fn start(device: &str, events: impl Fn(Event) + Send + 'static) -> Result<Soloist> {
        let key = soloist::key().context("Spotify playback isn't set up: Settings → Sources")?;
        if !soloist::installed() {
            anyhow::bail!("Spotify Soloist isn't downloaded yet: Settings → Sources → set up");
        }
        let mut child = soloist::spawn(&key, device)?;
        let port = match soloist::ws_port(&mut child) {
            Ok(port) => port,
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(e);
            }
        };
        let stream = TcpStream::connect(("127.0.0.1", port)).context("connecting to Soloist")?;
        let (socket, _) = tungstenite::client(format!("ws://127.0.0.1:{port}/"), stream)
            .map_err(|e| anyhow::anyhow!("Soloist's control line: {e}"))?;
        socket
            .get_ref()
            .set_read_timeout(Some(Duration::from_millis(50)))?;
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || run(socket, rx, events));
        let soloist = Soloist {
            child,
            commands: tx,
            device: device.to_string(),
        };
        soloist.send(json!({"type": "command", "command": "activate"}));
        Ok(soloist)
    }

    fn send(&self, command: Value) {
        let _ = self.commands.send(command);
    }

    fn command(&self, name: &str) {
        self.send(json!({"type": "command", "command": name}));
    }

    pub fn play(&self, uri: &str) {
        self.send(json!({"type": "command", "command": "play", "uri": uri}));
    }

    pub fn pause(&self) {
        self.command("pause");
    }

    pub fn resume(&self) {
        self.command("play");
    }

    pub fn seek(&self, seconds: f64) {
        let ms = (seconds.max(0.0) * 1000.0) as u64;
        self.send(json!({"type": "command", "command": "seek", "position_ms": ms}));
    }

    pub fn set_volume(&self, volume: u8) {
        self.send(json!({"type": "command", "command": "set_volume", "volume": volume.min(100)}));
    }

    pub fn alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }
}

impl Drop for Soloist {
    fn drop(&mut self) {
        self.command("pause");
        std::thread::sleep(Duration::from_millis(50));
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Owns the WebSocket: sends queued commands and reads events until either side goes away.
fn run(mut socket: WebSocket<TcpStream>, commands: Receiver<Value>, events: impl Fn(Event)) {
    loop {
        loop {
            match commands.try_recv() {
                Ok(command) => {
                    if socket.send(Message::text(command.to_string())).is_err() {
                        events(Event::Gone("Soloist stopped answering".into()));
                        return;
                    }
                }
                Err(TryRecvError::Empty) => break,
                // OMAESTRO dropped its Soloist: close quietly.
                Err(TryRecvError::Disconnected) => {
                    let _ = socket.close(None);
                    return;
                }
            }
        }
        match socket.read() {
            Ok(Message::Text(text)) => {
                keep_copy(&text);
                if let Some(event) = parse(&text) {
                    events(event);
                }
            }
            Ok(_) => {}
            Err(tungstenite::Error::Io(e))
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(e) => {
                events(Event::Gone(format!("Soloist stopped: {e}")));
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_soloist_messages() {
        let snapshot = r#"{"type":"playback_state","status":"playing",
            "item":{"uri":"spotify:track:2JRo","decorations":{"playback":{"duration_ms":210000}}},
            "position":{"position_ms":45000,"timestamp_ms":1747654321000,"speed":1.0},"volume":65}"#;
        match parse(snapshot) {
            Some(Event::State(s)) => {
                assert_eq!(s.status.as_deref(), Some("playing"));
                assert_eq!(s.uri.as_deref(), Some("spotify:track:2JRo"));
                assert_eq!(
                    (s.duration_ms, s.position_ms, s.speed),
                    (Some(210000.0), Some(45000.0), Some(1.0))
                );
                assert_eq!(s.volume, Some(65));
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(
            parse(r#"{"type":"playback_changed","status":"paused"}"#),
            Some(Event::State(State {
                status: Some("paused".into()),
                ..State::default()
            }))
        );
        assert_eq!(
            parse(r#"{"type":"auth_state","logged_in":false}"#),
            Some(Event::LoggedOut)
        );
        assert_eq!(
            parse(r#"{"type":"error","message":"command requires authentication"}"#),
            Some(Event::Error("command requires authentication".into()))
        );
        assert_eq!(
            parse(r#"{"type":"command_result","command":"pause"}"#),
            None
        );
        assert_eq!(parse("not json"), None);

        // Quality, wherever and however it's given.
        let lossless = r#"{"type":"track_changed","item":{"uri":"spotify:track:A",
            "audio":{"codec":"flac","bit_depth":16,"sample_rate":44100}}}"#;
        let Some(Event::State(s)) = parse(lossless) else {
            panic!()
        };
        let q = s.quality.unwrap();
        assert!(q.lossless && q.codec == "FLAC");
        assert_eq!((q.bit_depth, q.sample_rate), (Some(16), Some(44100)));
        let lossy = r#"{"type":"track_changed","item":{"uri":"spotify:track:A"},
            "format":{"codec":"ogg_vorbis","bitrate":320000}}"#;
        let Some(Event::State(s)) = parse(lossy) else {
            panic!()
        };
        let q = s.quality.unwrap();
        assert_eq!(
            (q.codec.as_str(), q.kbps, q.lossless),
            ("VORBIS", Some(320), false)
        );
        let words = r#"{"type":"playback_state","status":"playing","audio_quality":"very_high"}"#;
        let Some(Event::State(s)) = parse(words) else {
            panic!()
        };
        assert_eq!(s.quality.unwrap().level, "VERY HIGH");
        let none = r#"{"type":"playback_changed","status":"paused"}"#;
        let Some(Event::State(s)) = parse(none) else {
            panic!()
        };
        assert_eq!(s.quality, None);
    }
}
