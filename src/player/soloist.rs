//! Plays Spotify on this computer through Spotify Soloist (see `spotify/soloist.rs` for setup).
//! OMAESTRO starts Soloist when the first Spotify song plays here, and steers it over its
//! local WebSocket: one song at a time, from OMAESTRO's own queue.
//!
//! Soloist doesn't say when a song has finished, so the player watches for it: the position
//! reaching the end and stopping, or Soloist moving on to a song we didn't ask for.

use crate::spotify::soloist;
use anyhow::{Context, Result};
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::net::TcpStream;
use std::process::Child;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::time::{Duration, Instant};
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

/// Soloist's word for the stream's quality, should a later version give one: its documented
/// API (and what it sent on 2026-10-13) has no such fields. This looks anywhere in a message
/// for an object with fields like "codec", "bitrate", "bit_depth", "sample_rate", "hifi" or
/// "audio_quality".
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

/// One line in `~/.cache/omaestro/soloist.log`: what was sent to Soloist and what it said
/// (all but the position ticks), so a problem shows its cause. Started afresh with Soloist.
fn log(line: &str) {
    use std::io::Write;
    let path = crate::config::cache_dir().join("soloist.log");
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(f, "{} {line}", crate::secrets::now_secs());
    }
}

/// A message from Soloist in a few words: its type and what it's about.
fn summary(v: &Value) -> String {
    let kind = v["type"].as_str().unwrap_or("?");
    let about = [
        v["item"]["uri"].as_str().map(str::to_string),
        v["item"]["decorations"]["identity"]["name"]
            .as_str()
            .map(|n| format!("“{n}”")),
        v["status"].as_str().map(str::to_string),
        v["command"].as_str().map(str::to_string),
        v["message"].as_str().map(str::to_string),
        v.get("logged_in").map(|l| format!("logged_in={l}")),
        v.get("is_active").map(|a| format!("active={a}")),
    ];
    let about: Vec<String> = about.into_iter().flatten().collect();
    format!("← {kind} {}", about.join(" "))
}

/// How long commands wait for Soloist to sign in before OMAESTRO says it isn't.
const SIGN_IN_WAIT: Duration = Duration::from_secs(20);

/// Holds commands until Soloist has signed in to Spotify. A freshly started Soloist (a new
/// output means a restart) takes a moment to sign in and refuses commands until then with
/// "command requires authentication". It says when it's ready: `auth_state` with
/// `logged_in: true`, sent when OMAESTRO connects and again whenever that changes.
#[derive(Debug, Default)]
struct Gate {
    signed_in: bool,
    held: Vec<Value>,
    /// Commands sent and not yet confirmed (`command_result`): if Soloist refuses one, these
    /// are sent again once it's signed in. Confirmed ones never are, so nothing plays twice.
    unconfirmed: VecDeque<Value>,
    /// Since when commands have been waiting.
    waiting_since: Option<Instant>,
}

/// What to do with a message from Soloist.
#[derive(Debug, PartialEq)]
enum Verdict {
    /// Pass it on to the player.
    Pass,
    /// Keep it to ourselves (signing in is expected to take a moment).
    Swallow,
    /// Pass it on, and send these commands that were waiting.
    Flush(Vec<Value>),
}

impl Gate {
    /// A command from the player: what to send now.
    fn command(&mut self, command: Value, now: Instant) -> Vec<Value> {
        if self.signed_in {
            self.unconfirmed.push_back(command.clone());
            // Only the latest few matter; older ones were confirmed or are moot.
            while self.unconfirmed.len() > 8 {
                self.unconfirmed.pop_front();
            }
            vec![command]
        } else {
            self.held.push(command);
            self.waiting_since.get_or_insert(now);
            Vec::new()
        }
    }

    fn message(&mut self, v: &Value, now: Instant) -> Verdict {
        match v["type"].as_str() {
            Some("auth_state") if v["logged_in"] == true => {
                self.signed_in = true;
                self.waiting_since = None;
                let ready = std::mem::take(&mut self.held);
                self.unconfirmed.extend(ready.iter().cloned());
                Verdict::Flush(ready)
            }
            Some("auth_state") => {
                self.signed_in = false;
                Verdict::Swallow
            }
            Some("command_result") => {
                let name = &v["command"];
                if let Some(i) = self.unconfirmed.iter().position(|c| c["command"] == *name) {
                    self.unconfirmed.remove(i);
                }
                Verdict::Pass
            }
            Some("error")
                if v["message"]
                    .as_str()
                    .is_some_and(|m| m.contains("requires authentication")) =>
            {
                self.signed_in = false;
                let refused: Vec<Value> = self.unconfirmed.drain(..).collect();
                self.held.splice(0..0, refused);
                self.waiting_since.get_or_insert(now);
                Verdict::Swallow
            }
            _ => Verdict::Pass,
        }
    }

    /// Whether commands have waited too long: then they're dropped, and it's time to say so.
    fn gave_up(&mut self, now: Instant) -> bool {
        if self
            .waiting_since
            .is_some_and(|since| now.duration_since(since) > SIGN_IN_WAIT)
        {
            self.held.clear();
            self.waiting_since = None;
            return true;
        }
        false
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
        let _ = std::fs::write(
            crate::config::cache_dir().join("soloist.log"),
            format!(
                "{} Soloist starting for output {device:?}\n",
                crate::secrets::now_secs()
            ),
        );
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
    let mut gate = Gate::default();
    let send = |socket: &mut WebSocket<TcpStream>, list: Vec<Value>| {
        list.into_iter().all(|command| {
            log(&format!("→ {command}"));
            socket.send(Message::text(command.to_string())).is_ok()
        })
    };
    loop {
        if gate.gave_up(Instant::now()) {
            events(Event::LoggedOut);
        }
        loop {
            match commands.try_recv() {
                Ok(command) => {
                    let now = gate.command(command, Instant::now());
                    if !send(&mut socket, now) {
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
                let v: Value = serde_json::from_str(&text).unwrap_or_default();
                if v["type"] != "position_sync" {
                    log(&summary(&v));
                }
                match gate.message(&v, Instant::now()) {
                    Verdict::Swallow => continue,
                    Verdict::Flush(waiting) => {
                        if !send(&mut socket, waiting) {
                            events(Event::Gone("Soloist stopped answering".into()));
                            return;
                        }
                    }
                    Verdict::Pass => {}
                }
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
    fn commands_wait_until_soloist_has_signed_in() {
        let t0 = Instant::now();
        let at = |s: u64| t0 + Duration::from_secs(s);
        let cmd = |name: &str| json!({"type": "command", "command": name});
        let auth = |on: bool| json!({"type": "auth_state", "logged_in": on});
        let refused = json!({"type": "error", "message": "command requires authentication"});
        let done = |name: &str| json!({"type": "command_result", "command": name});

        // A fresh Soloist: not signed in yet, so commands wait, and that's not news.
        let mut gate = Gate::default();
        assert_eq!(gate.message(&auth(false), at(0)), Verdict::Swallow);
        assert!(gate.command(cmd("activate"), at(0)).is_empty());
        assert!(gate.command(cmd("play"), at(1)).is_empty());
        // Signed in: they go, in order; from then on commands go straight away.
        assert_eq!(
            gate.message(&auth(true), at(2)),
            Verdict::Flush(vec![cmd("activate"), cmd("play")])
        );
        assert_eq!(gate.message(&done("activate"), at(2)), Verdict::Pass);
        assert_eq!(gate.command(cmd("pause"), at(3)), vec![cmd("pause")]);

        // Refused after all: what wasn't confirmed waits and goes again once signed in, but
        // the confirmed activate isn't repeated.
        assert_eq!(gate.message(&refused, at(4)), Verdict::Swallow);
        assert!(gate.command(cmd("seek"), at(4)).is_empty());
        assert_eq!(
            gate.message(&auth(true), at(5)),
            Verdict::Flush(vec![cmd("play"), cmd("pause"), cmd("seek")])
        );

        // Never signing in: after a while, give up and say so (once).
        let mut gate = Gate::default();
        gate.command(cmd("play"), at(0));
        assert!(!gate.gave_up(at(10)));
        assert!(gate.gave_up(at(21)));
        assert!(!gate.gave_up(at(22)));
        assert!(gate.held.is_empty());
    }

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
