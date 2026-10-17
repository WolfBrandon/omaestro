//! The visualizer: listens to what OMAESTRO plays and splits it into frequency bands for the
//! VISUALIZER pane's bars.
//!
//! Sound is captured with `pw-record` (part of PipeWire) and analysed here with an FFT; nothing
//! is ever written to disk. It listens to mpv's own output only (the "omaestro" stream), or for
//! Spotify to the monitor of the device it plays on. `node.dont-fallback` makes sure it never
//! falls back to a microphone when that target isn't there; it just stops instead.
//!
//! On Sonos the sound never passes through this computer, so the bars come from a silent copy
//! instead: ffmpeg decodes the same file from where the speaker is, straight into the
//! analysis, which takes each moment of it when the song's clock (`follow`) gets there.
//! Nothing of it reaches any speaker.

use rustfft::{Fft, FftPlanner, num_complex::Complex};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Instant;

const RATE: usize = 48_000;
const FFT_SIZE: usize = 2048;
const HOP: usize = 800; // 48000 / 800 = 60 updates per second
const F_MIN: f32 = 40.0;
const F_MAX: f32 = 16_000.0;
const RANGE_DB: f32 = 45.0; // dB shown below the auto-gain ceiling
const AGC_FALL_DB: f32 = 0.03; // ceiling drops ~1.8 dB/s after loud parts
const AGC_FLOOR_DB: f32 = -50.0; // never amplify more than this (near-silence stays dark)
const ATTACK: f32 = 0.7; // smoothing toward a higher value (per frame)
const FALL: f32 = 0.2; // smoothing toward a lower value (per frame): ~0.2 s to drop away

#[derive(Clone, Debug)]
pub enum Target {
    /// Monitor of the default sink; follows the default if it changes.
    DefaultSink,
    /// Monitor of the sink with this node.name (whole-sink mix).
    Sink(String),
    /// A playback stream node, e.g. mpv's "omaestro" (= --audio-client-name).
    /// Only exists while mpv has audio open: pw-record exits when it goes away.
    Stream(String),
    /// A song playing somewhere else (Sonos): decode this file silently, in step with it.
    File(PathBuf),
}

impl Target {
    /// mpv `audio-device` -> target. "pipewire" / "auto" / "" -> default sink,
    /// "pipewire/<node.name>" -> that sink's monitor.
    pub fn from_mpv_device(dev: &str) -> Target {
        match dev.strip_prefix("pipewire/") {
            Some(name) if !name.is_empty() => Target::Sink(name.to_string()),
            _ => Target::DefaultSink,
        }
    }
}

struct Shared {
    bands: Vec<f32>,
    updated: Instant,
    alive: bool,
    /// A silent copy's clock: where the song is, as of when.
    clock: (f64, Instant),
    /// A silent copy: where in the song the newest sound analysed is.
    copy_at: Option<f64>,
    /// Set when the visualizer goes, so a copy waiting for the clock stops waiting.
    stop: bool,
}

pub struct Visualizer {
    child: Child,
    shared: Arc<Mutex<Shared>>,
    thread: Option<JoinHandle<()>>,
}

fn spawn_pw_record(target: &Target) -> io::Result<Child> {
    // node.dont-fallback: if a stream target is missing, fail instead of falling back
    //   to the default SOURCE (= your microphone!). Not for an output's sound
    //   (stream.capture.sink): that falls back to the default output, never a microphone
    //   (checked 2026-10-13), and with it PipeWire 1.6 can't find a busy output by name
    //   ("defined target not found"), so the capture died at once, over and over.
    // node.passive: don't keep an idle sink awake; no data flows while nothing plays.
    // node.name: must differ from mpv's "omaestro" so a Stream target can't match us.
    let (props, name) = match target {
        Target::DefaultSink => (
            "{ stream.capture.sink=true node.dont-fallback=true node.passive=true node.name=omaestro-vis }",
            None,
        ),
        Target::Sink(n) => (
            "{ stream.capture.sink=true node.passive=true node.name=omaestro-vis }",
            Some(n.clone()),
        ),
        Target::Stream(n) => (
            "{ node.dont-fallback=true node.passive=true node.name=omaestro-vis }",
            Some(n.clone()),
        ),
        Target::File(_) => return Err(io::Error::other("a file isn't captured")),
    };
    let mut cmd = Command::new("pw-record");
    cmd.args([
        "--raw",
        "--format",
        "f32",
        "--rate",
        "48000",
        "--channels",
        "2",
    ]);
    cmd.args(["--latency", "20ms", "-P", props]);
    if let Some(n) = name {
        cmd.args(["--target", &n]);
    }
    cmd.arg("-")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
}

/// Decodes `path` from `start` seconds in, as the same raw stereo 48 kHz samples pw-record
/// gives. It's read as the song plays, so ffmpeg waits on the pipe (about 0.2 s ahead).
fn spawn_ffmpeg(path: &Path, start: f64) -> io::Result<Child> {
    Command::new("ffmpeg")
        .args(["-v", "error", "-nostdin", "-ss"])
        .arg(format!("{:.2}", start.max(0.0)))
        .arg("-i")
        .arg(path)
        .args(["-vn", "-ac", "2", "-ar", "48000", "-f", "f32le", "-"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
}

impl Visualizer {
    /// Starts listening. `start` is where in the song to begin, for a `File` target.
    pub fn start(target: Target, n_bands: usize, start: f64) -> io::Result<Visualizer> {
        let mut child = match &target {
            Target::File(path) => spawn_ffmpeg(path, start)?,
            _ => spawn_pw_record(&target)?,
        };
        let stdout = child.stdout.take().expect("piped stdout");
        let copy = matches!(target, Target::File(_));
        let shared = Arc::new(Mutex::new(Shared {
            bands: vec![0.0; n_bands],
            updated: Instant::now(),
            alive: true,
            clock: (start, Instant::now()),
            copy_at: copy.then_some(start),
            stop: false,
        }));
        let sh = Arc::clone(&shared);
        let paced = copy.then_some(start);
        let thread = thread::spawn(move || reader_loop(stdout, sh, n_bands, paced));
        Ok(Visualizer {
            child,
            shared,
            thread: Some(thread),
        })
    }

    /// Current bars, 0..1. Fades out if no audio has arrived recently
    /// (pause / nothing playing: pw-record then sends nothing at all).
    pub fn bands(&self) -> Vec<f32> {
        let s = self.shared.lock().unwrap();
        let stale = s.updated.elapsed().as_secs_f32();
        let k = if stale < 0.1 {
            1.0
        } else {
            (-(stale - 0.1) * 6.0).exp()
        };
        s.bands.iter().map(|b| b * k).collect()
    }

    /// False once pw-record has exited (target vanished or failed to link).
    pub fn is_alive(&self) -> bool {
        self.shared.lock().unwrap().alive
    }

    /// For a silent copy: where the song is now, so its sound is analysed in step with it.
    pub fn follow(&self, position: f64) {
        self.shared.lock().unwrap().clock = (position, Instant::now());
    }

    /// For a silent copy: where in the song the newest sound analysed is.
    pub fn copy_at(&self) -> Option<f64> {
        self.shared.lock().unwrap().copy_at
    }
}

impl Drop for Visualizer {
    fn drop(&mut self) {
        self.shared.lock().unwrap().stop = true;
        let _ = self.child.kill();
        let _ = self.child.wait(); // reap, no zombie
        if let Some(t) = self.thread.take() {
            let _ = t.join(); // stdout hit EOF when the child died
        }
    }
}

/// Reads the sound and analyses it. A silent copy (`paced`: where it starts in the song)
/// comes faster than it plays, so each piece waits for the song's clock to get there; one
/// that's behind (just started) is caught up at once.
fn reader_loop(
    mut stdout: ChildStdout,
    shared: Arc<Mutex<Shared>>,
    n_bands: usize,
    paced: Option<f64>,
) {
    let mut an = Analyzer::new(n_bands);
    let mut buf = vec![0u8; HOP * 2 * 4]; // HOP stereo f32 frames
    let mut ring = vec![0f32; FFT_SIZE]; // last FFT_SIZE mono samples
    let mut frames: u64 = 0;
    while stdout.read_exact(&mut buf).is_ok() {
        if let Some(start) = paced {
            frames += HOP as u64;
            let at = start + frames as f64 / RATE as f64;
            loop {
                let ((pos, since), stop) = {
                    let s = shared.lock().unwrap();
                    (s.clock, s.stop)
                };
                if stop {
                    return;
                }
                let ahead = at - (pos + since.elapsed().as_secs_f64());
                if ahead <= 0.0 {
                    break;
                }
                thread::sleep(std::time::Duration::from_secs_f64(ahead.min(0.05)));
            }
            shared.lock().unwrap().copy_at = Some(at);
        }
        ring.copy_within(HOP.., 0);
        let tail = &mut ring[FFT_SIZE - HOP..];
        for (dst, fr) in tail.iter_mut().zip(buf.as_chunks::<8>().0) {
            let l = f32::from_le_bytes([fr[0], fr[1], fr[2], fr[3]]);
            let r = f32::from_le_bytes([fr[4], fr[5], fr[6], fr[7]]);
            *dst = 0.5 * (l + r);
        }
        let bands = an.process(&ring);
        let mut s = shared.lock().unwrap();
        s.bands.copy_from_slice(bands);
        s.updated = Instant::now();
    }
    shared.lock().unwrap().alive = false;
}

/// (first_bin, end_bin) per band, log-spaced F_MIN..F_MAX. Every band gets at
/// least one bin of its own (low bands are only ~23 Hz apart at 2048/48k).
fn band_bins(n: usize) -> Vec<(usize, usize)> {
    let bin_hz = RATE as f32 / FFT_SIZE as f32;
    let edge = |i: usize| F_MIN * (F_MAX / F_MIN).powf(i as f32 / n as f32) / bin_hz;
    let mut out = Vec::with_capacity(n);
    let mut prev_end = 1; // skip DC
    for i in 0..n {
        let lo = (edge(i).round() as usize).max(prev_end);
        let hi = (edge(i + 1).round() as usize).max(lo + 1).min(FFT_SIZE / 2);
        out.push((lo, hi));
        prev_end = hi;
    }
    out
}

struct Analyzer {
    fft: Arc<dyn Fft<f32>>,
    window: Vec<f32>,
    buf: Vec<Complex<f32>>,
    scratch: Vec<Complex<f32>>,
    bins: Vec<(usize, usize)>,
    db: Vec<f32>,
    out: Vec<f32>,
    agc_db: f32,
}

impl Analyzer {
    fn new(n_bands: usize) -> Analyzer {
        let fft = FftPlanner::new().plan_fft_forward(FFT_SIZE);
        let scratch = vec![Complex::default(); fft.get_inplace_scratch_len()];
        let window = (0..FFT_SIZE)
            .map(|i| {
                let x = std::f32::consts::TAU * i as f32 / (FFT_SIZE - 1) as f32;
                0.5 - 0.5 * x.cos() // Hann
            })
            .collect();
        Analyzer {
            fft,
            window,
            buf: vec![Complex::default(); FFT_SIZE],
            scratch,
            bins: band_bins(n_bands),
            db: vec![0.0; n_bands],
            out: vec![0.0; n_bands],
            agc_db: AGC_FLOOR_DB,
        }
    }

    /// `mono` = last FFT_SIZE samples. Returns smoothed bands in 0..1.
    fn process(&mut self, mono: &[f32]) -> &[f32] {
        for ((b, &x), &w) in self.buf.iter_mut().zip(mono).zip(&self.window) {
            *b = Complex::new(x * w, 0.0);
        }
        self.fft
            .process_with_scratch(&mut self.buf, &mut self.scratch);

        // Power summed per band -> dB. Scaled so a full-scale sine is ~0 dB.
        // Summing power over log-wide bands makes pink noise (and most music) look flat.
        let n = FFT_SIZE as f32;
        let norm = 32.0 / (3.0 * n * n);
        let mut frame_max = f32::MIN;
        for (d, &(lo, hi)) in self.db.iter_mut().zip(&self.bins) {
            let p: f32 = self.buf[lo..hi].iter().map(|c| c.norm_sqr()).sum();
            *d = 10.0 * (p * norm + 1e-12).log10();
            frame_max = frame_max.max(*d);
        }

        // Auto-gain: ceiling jumps up instantly, then sinks slowly.
        self.agc_db = if frame_max > self.agc_db {
            frame_max
        } else {
            (self.agc_db - AGC_FALL_DB).max(AGC_FLOOR_DB)
        };

        let floor = self.agc_db - RANGE_DB;
        for (o, &d) in self.out.iter_mut().zip(&self.db) {
            let v = ((d - floor) / RANGE_DB).clamp(0.0, 1.0);
            let k = if v > *o { ATTACK } else { FALL };
            *o += (v - *o) * k;
        }
        &self.out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{TempDir, have};
    use std::time::Duration;

    /// Listens (only) to a real output while something plays on it, e.g.
    /// `OMAESTRO_SINK=alsa_output.usb-… cargo test live_sink -- --ignored --nocapture`.
    #[test]
    #[ignore = "needs a real output with music playing on it"]
    fn live_sink_capture_stays_up() {
        let Ok(sink) = std::env::var("OMAESTRO_SINK") else {
            return;
        };
        let v = Visualizer::start(Target::Sink(sink), 32, 0.0).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2000));
        let loudest = v.bands().into_iter().fold(0.0f32, f32::max);
        println!("alive: {}, loudest band: {loudest:.2}", v.is_alive());
        assert!(v.is_alive(), "the capture stays up");
    }

    /// A 1 kHz tone decoded silently from a file lights up the band around 1 kHz most.
    #[test]
    fn a_silent_copy_of_a_file_drives_the_bars() {
        if !have("ffmpeg") {
            eprintln!("skipping: needs ffmpeg");
            return;
        }
        let tmp = TempDir::new("vis-file");
        let tone = tmp.0.join("tone.flac");
        let ok = Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=1000:duration=4",
            ])
            .arg(&tone)
            .status()
            .unwrap()
            .success();
        assert!(ok);
        let vis = Visualizer::start(Target::File(tone), 24, 1.0).unwrap();
        std::thread::sleep(Duration::from_millis(700));
        let bands = vis.bands();
        let loudest = (0..bands.len())
            .max_by(|&a, &b| bands[a].total_cmp(&bands[b]))
            .unwrap();
        let (lo, hi) = band_bins(24)[loudest];
        let hz = |bin: usize| bin as f32 * RATE as f32 / FFT_SIZE as f32;
        assert!(bands[loudest] > 0.5, "{bands:?}");
        assert!(
            hz(lo) <= 1050.0 && hz(hi) >= 950.0,
            "loudest band {}–{} Hz",
            hz(lo),
            hz(hi)
        );
        assert!(vis.is_alive());
    }

    /// A silent copy is analysed in step with the song's clock, never ahead of it, and catches
    /// up at once when the clock jumps on.
    #[test]
    fn a_silent_copy_keeps_to_the_song_clock() {
        if !have("ffmpeg") {
            eprintln!("skipping: needs ffmpeg");
            return;
        }
        let tmp = TempDir::new("vis-clock");
        let tone = tmp.0.join("tone.flac");
        let ok = Command::new("ffmpeg")
            .args(["-v", "error", "-y", "-f", "lavfi", "-i"])
            .arg("sine=frequency=440:duration=10")
            .arg(&tone)
            .status()
            .unwrap()
            .success();
        assert!(ok);
        let vis = Visualizer::start(Target::File(tone), 24, 1.0).unwrap();
        let started = Instant::now();
        std::thread::sleep(Duration::from_millis(500));
        let song = 1.0 + started.elapsed().as_secs_f64();
        let at = vis.copy_at().unwrap();
        assert!(at <= song + 0.001 && at > song - 0.1, "{at} vs {song}");
        vis.follow(6.0);
        let jumped = Instant::now();
        std::thread::sleep(Duration::from_millis(300));
        let song = 6.0 + jumped.elapsed().as_secs_f64();
        let at = vis.copy_at().unwrap();
        assert!(at <= song + 0.001 && at > song - 0.1, "{at} vs {song}");
        // Back to the start: the copy waits for the clock, and still stops at once.
        vis.follow(0.0);
        std::thread::sleep(Duration::from_millis(100));
        let dropped = Instant::now();
        drop(vis);
        assert!(dropped.elapsed() < Duration::from_millis(500));
    }
}
