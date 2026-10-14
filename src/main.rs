//! OMAESTRO: one terminal music player for local files, Spotify, YouTube Music and Sonos.

mod animation;
mod app;
mod art;
mod browse;
mod columns;
mod config;
mod connect;
mod desktop;
mod fileinfo;
mod integrations;
mod library;
mod player;
mod playlist;
mod search;
mod secrets;
mod session;
mod settings;
mod sonos;
mod spotify;
mod status;
#[cfg(test)]
mod testutil;
mod theme;
mod ui;
mod update;
mod visualizer;
mod youtube;

use anyhow::{Context, Result};
use app::{Action, App, Ctx, News};
use config::Config;
use library::scan::{self, Progress, Summary};
use library::{Db, Source};
use player::Player;
use player::mpv::Options;
use ratatui::crossterm::event::{
    self, DisableBracketedPaste, DisableFocusChange, DisableMouseCapture, EnableBracketedPaste,
    EnableFocusChange, EnableMouseCapture, Event, KeyEventKind,
};
use ratatui::crossterm::execute;
use std::io::Write;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};
use theme::ThemeFile;

const USAGE: &str = "\
usage: omaestro [command]

  (no command)      open the player in this terminal
  launch            bring back the running player, or open it in a new window
                    (what the app launcher runs)
  scan              update the library index from the music folders
  search WORDS…     search the library index
  stats             show what's in the index
  --version         show the version
";

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("launch") => desktop::launch(),
        Some("scan") => cmd_scan(),
        Some("search") => cmd_search(&args[1..].join(" ")),
        Some("stats") => cmd_stats(),
        // Started by an update in place: carry on playing from where the last version was.
        Some("--resume") => run_player(true),
        Some("--version" | "-V") => {
            println!("omaestro {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some("--help" | "-h") | Some(_) => {
            print!("{USAGE}");
            Ok(())
        }
        None => run_player(false),
    }
}

/// News from the background library scan.
enum ScanMsg {
    Progress(Progress),
    Done(Result<Summary, String>),
}

/// Refreshes the library index on a background thread, with its own connection to it.
fn start_scan(config: &Config, tx: Sender<ScanMsg>) {
    let roots = config.library_roots();
    let threads = config.scan_threads;
    std::thread::spawn(move || {
        let result = open_db().and_then(|mut db| {
            scan::scan(&mut db, &roots, threads, |p| {
                let _ = tx.send(ScanMsg::Progress(p));
            })
        });
        let _ = tx.send(ScanMsg::Done(result.map_err(|e| format!("{e:#}"))));
    });
}

/// How the main loop ended.
enum Exit {
    Quit,
    /// Switch to the new version, playing on from the same place if `resume`.
    Restart {
        resume: bool,
    },
}

fn run_player(resume: bool) -> Result<()> {
    let mut watch = update::Watch::new();
    let config = Config::load()?;
    let (mpv_tx, mpv_rx) = mpsc::channel();
    let options = Options {
        audio_device: config.playback.audio_device.clone(),
        replaygain: config.playback.replaygain.clone(),
        gapless: config.playback.gapless,
        volume: config.playback.volume,
        silent: false,
    };
    let mut player = Player::start(&options, &config::runtime_dir().join("mpv.sock"), mpv_tx)?;
    let (news_tx, news_rx) = mpsc::channel();
    let scan_on_launch = config.scan_on_launch;
    // The queue from last time, read before the library index goes to the app.
    let db = open_db()?;
    let last = session::load(&config::data_dir(), &db);
    let mut app = App::new(db, config, news_tx)?;
    if let Some(last) = last {
        app.queue_sel = last.current.unwrap_or(0);
        player.restore(last.tracks, last.current, last.playlist, last.position);
    }

    // Open straight from the saved index, and check the folders for changes meanwhile.
    let (scan_tx, scan_rx) = mpsc::channel();
    if scan_on_launch {
        start_scan(&app.config, scan_tx.clone());
        app.scanning = true;
    }
    // Look for the Sonos speakers (reading only), so the output picker can list them.
    app.find_sonos();
    sonos::server::prune_cache();

    // If OMAESTRO crashes, give the mouse back to the terminal before the error is shown.
    let previous_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = execute!(
            std::io::stdout(),
            DisableMouseCapture,
            DisableFocusChange,
            DisableBracketedPaste
        );
        previous_hook(info);
    }));
    let mut terminal = ratatui::init();
    if app.config.appearance.album_art != config::Art::Off {
        let picker = art::picker(app.config.appearance.album_art);
        app.best_pictures = Some(picker.protocol_type());
        app.picker = Some(picker);
    }
    // Clicks and the scroll wheel come to OMAESTRO (Shift+drag still selects text), and pasted
    // text arrives in one piece.
    execute!(
        std::io::stdout(),
        EnableMouseCapture,
        EnableFocusChange,
        EnableBracketedPaste
    )?;
    // Media keys, the bar's media widget and the tray icon; and the window to minimize.
    let (cmd_tx, cmd_rx) = mpsc::channel();
    let mut desktop = Desktop {
        commands: cmd_rx,
        mpris: desktop::start_mpris(cmd_tx.clone()).ok(),
        tray: desktop::start_tray(cmd_tx),
        window: desktop::Window::find(),
        song: String::new(),
        cover: None,
        status: None,
        position: 0.0,
        position_sent: Instant::now(),
        tracks: 0,
        volume: -1.0,
        minimized: false,
    };
    let result = event_loop(
        &mut terminal,
        &mut app,
        &mut player,
        &mpv_rx,
        &scan_rx,
        &scan_tx,
        &news_rx,
        &mut desktop,
        &mut watch,
        resume,
    );
    let _ = execute!(
        std::io::stdout(),
        DisableMouseCapture,
        DisableFocusChange,
        DisableBracketedPaste
    );
    ratatui::restore();
    // Quitting stops the music, wherever it plays; this computer's volume is kept for next
    // time (not a Sonos speaker's), and so is the queue, with the place in the song.
    app.config.playback.volume = player.here_volume().round() as u8;
    let _ = session::save(&config::data_dir(), &player.session());
    let _ = app.config.save();
    player.shut_down();
    match result? {
        Exit::Quit => Ok(()),
        Exit::Restart { resume } => {
            // Starting the new version replaces this process without running any clean-up:
            // stop mpv and Soloist first, so the new one can start them.
            drop(player);
            drop(desktop);
            let path = watch
                .path()
                .context("can't tell where OMAESTRO was started from")?;
            let error = update::restart(path, resume);
            Err(anyhow::anyhow!("couldn't start the new version: {error}"))
        }
    }
}

/// What the desktop side (MPRIS, the tray, the window) needs between loop turns, including
/// what it was last told, so it's only told about changes.
struct Desktop {
    commands: Receiver<desktop::Cmd>,
    mpris: Option<desktop::Mpris>,
    tray: Option<ksni::blocking::Handle<desktop::Tray>>,
    window: Option<desktop::Window>,
    song: String,
    cover: Option<std::path::PathBuf>,
    status: Option<mpris_server::PlaybackStatus>,
    position: f64,
    position_sent: Instant,
    tracks: u64,
    volume: f64,
    /// Tucked away in the scratchpad: no point drawing the visualizer.
    minimized: bool,
}

impl Desktop {
    /// Carries out what the desktop asked for. Returns true to quit.
    fn obey(&mut self, player: &mut Player) -> Result<bool> {
        use desktop::Cmd;
        while let Ok(cmd) = self.commands.try_recv() {
            let playing = !player.status.idle && !player.status.paused;
            match cmd {
                Cmd::Play if !playing => player.toggle_pause()?,
                Cmd::Pause if playing => player.toggle_pause()?,
                Cmd::Play | Cmd::Pause => {}
                Cmd::PlayPause => player.toggle_pause()?,
                Cmd::Stop => player.stop()?,
                Cmd::Next => player.next()?,
                Cmd::Previous => player.previous()?,
                Cmd::SeekBy(us) => player.seek(us as f64 / 1e6)?,
                Cmd::SetPosition(us) => player.seek_to(us as f64 / 1e6)?,
                Cmd::SetVolume(v) => player.set_volume(v * 100.0)?,
                Cmd::Raise => {
                    if let Some(w) = &self.window {
                        w.show();
                        self.minimized = false;
                    }
                }
                Cmd::ToggleWindow => {
                    if let Some(w) = &self.window {
                        let hidden = w.toggle();
                        self.minimized = hidden;
                        if let Some(tray) = &self.tray {
                            tray.update(|t| t.hidden = hidden);
                        }
                    }
                }
                Cmd::Quit => return Ok(true),
            }
        }
        Ok(false)
    }

    /// Tells MPRIS and the tray what's playing, when it changes.
    fn report(&mut self, player: &Player, app: &App) {
        use mpris_server::{PlaybackStatus, Time};
        let track = player.current().filter(|_| !player.status.idle);
        let song = track.map_or(String::new(), |t| format!("{} – {}", t.artist, t.title));
        let new_song = song != self.song || app.cover_file != self.cover;
        if new_song {
            self.song = song.clone();
            self.cover = app.cover_file.clone();
            if let (Some(t), Some(m)) = (track, &self.mpris) {
                self.tracks += 1;
                m.send(desktop::Update::Metadata(desktop::metadata(
                    t,
                    self.tracks,
                    app.cover_file.as_deref(),
                )));
            }
        }
        let status = if player.status.idle {
            PlaybackStatus::Stopped
        } else if player.status.paused {
            PlaybackStatus::Paused
        } else {
            PlaybackStatus::Playing
        };
        if self.status != Some(status) || new_song {
            self.status = Some(status);
            if let Some(m) = &self.mpris {
                m.send(desktop::Update::Status(status));
            }
            if let Some(tray) = &self.tray {
                let playing = status == PlaybackStatus::Playing;
                tray.update(|t| {
                    t.song = song;
                    t.playing = playing;
                });
            }
        }
        if player.status.volume != self.volume {
            self.volume = player.status.volume;
            if let Some(m) = &self.mpris {
                m.send(desktop::Update::Volume(self.volume / 100.0));
            }
        }
        // The position: once a second quietly, at once (with a "seeked" signal) when it jumps.
        let pos = player.status.position;
        let moving = status == PlaybackStatus::Playing;
        let expected = self.position
            + if moving {
                self.position_sent.elapsed().as_secs_f64()
            } else {
                0.0
            };
        let jumped = (pos - expected).abs() > 2.0 || new_song;
        if jumped || self.position_sent.elapsed().as_secs() >= 1 {
            let t = Time::from_micros((pos * 1e6) as i64);
            if let Some(m) = &self.mpris {
                m.send(if jumped {
                    desktop::Update::Seeked(t)
                } else {
                    desktop::Update::Position(t)
                });
            }
            self.position = pos;
            self.position_sent = Instant::now();
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn event_loop(
    terminal: &mut ratatui::DefaultTerminal,
    app: &mut App,
    player: &mut Player,
    mpv_rx: &Receiver<player::Event>,
    scan_rx: &Receiver<ScanMsg>,
    scan_tx: &Sender<ScanMsg>,
    news_rx: &Receiver<News>,
    desktop: &mut Desktop,
    watch: &mut update::Watch,
    mut resume: bool,
) -> Result<Exit> {
    let mut update_offered = false;
    let mut dirty = true;
    // The queue is kept for next time whenever it or the song it's on changes.
    let mut kept = (
        player.revision,
        player.current_index(),
        player.playlist.as_ref().map(|p| p.name.clone()),
    );
    let mut theme_setting = app.config.appearance.theme.clone();
    let mut theme_file = ThemeFile::for_setting(&theme_setting);
    let mut theme_checked = Instant::now() - Duration::from_secs(60);
    let mut last_current = player.current_index();
    let mut hits = Vec::new();
    let mut had_overlay = false;
    // The visualizer's capture, which target it listens to, and when it last tried to start.
    let mut vis: Option<(String, visualizer::Visualizer)> = None;
    let mut vis_tried = Instant::now() - Duration::from_secs(1);
    // A capture that keeps dying straight away is tried less and less often (each try makes
    // PipeWire rewire the output, which can click): what it listened to, and how many times
    // in a row it died young.
    let mut vis_last_key = String::new();
    let mut vis_fails: u32 = 0;
    // For a silent copy (Sonos): where in the song it started and when, to keep it in step.
    let mut vis_clock: Option<(f64, Instant)> = None;
    loop {
        let now = (
            player.revision,
            player.current_index(),
            player.playlist.as_ref().map(|p| p.name.clone()),
        );
        if now != kept {
            kept = now;
            let _ = session::save(&config::data_dir(), &player.session());
        }
        // After an update: play on from the same place, once the output can (Sonos needs its
        // speakers found first).
        if resume && player.can_play() {
            resume = false;
            if let Some(i) = player.current_index() {
                player.jump(i)?;
            }
        }
        // A new version installed. With auto-update on, switch now if nothing's playing, else
        // offer U and switch at the next pause; with it off, only say so (the version in the
        // corner turns yellow) and switch when asked.
        if watch.poll() {
            app.update_ready = watch.ready.clone();
            dirty = true;
        }
        if let Some(version) = &watch.ready {
            let playing = !player.status.idle && !player.status.paused;
            match (app.config.updates.auto, update::when(playing)) {
                (true, update::Switch::Now) => return Ok(Exit::Restart { resume: false }),
                _ if update_offered => {}
                (true, update::Switch::Wait) => {
                    update_offered = true;
                    app.status = format!(
                        "v{version} is ready: U switches now, or it switches at the next pause"
                    );
                }
                (false, _) => {
                    update_offered = true;
                    app.status = format!("v{version} is ready: U, or Settings → About, updates");
                }
            }
        }
        // How the sources are doing, checked in the background every couple of minutes.
        if app.sources_due() {
            app.check_sources();
        }
        // A theme picked in Settings applies at once.
        if app.config.appearance.theme != theme_setting {
            theme_setting = app.config.appearance.theme.clone();
            theme_file = ThemeFile::for_setting(&theme_setting);
            theme_checked = Instant::now() - Duration::from_secs(60);
        }
        // Follow Omarchy theme switches; checking a file's date every 2 s costs nothing.
        if theme_checked.elapsed() >= Duration::from_secs(2) {
            theme_checked = Instant::now();
            if let Some(theme) = theme_file.changed() {
                app.theme = theme;
                app.theme_fixed = theme_file.fixed;
                dirty = true;
            }
        }
        // A popup over a sixel cover leaves bits of itself behind in foot: draw the cover
        // blank for one frame, so the next frame sends the whole picture again.
        if had_overlay && app.overlay.is_none() {
            app.cover_blink = true;
            dirty = true;
        }
        had_overlay = app.overlay.is_some();
        app.want_cover(player.current());
        // Listen while music plays here; try again every half second if the stream isn't up.
        let want = player
            .listen_target()
            .filter(|_| app.config.appearance.visualizer && !desktop.minimized);
        match want {
            None => {
                vis = None;
                vis_last_key.clear();
                if !app.bands.is_empty() {
                    app.bands.clear();
                    dirty = true;
                }
            }
            Some(target) => {
                let key = format!("{target:?}");
                let copy = matches!(target, visualizer::Target::File(_));
                // A silent copy drifting from the speaker (a seek, a hiccup) starts again in step.
                let drifted = copy
                    && vis_clock.is_some_and(|(at, since)| {
                        (at + since.elapsed().as_secs_f64() - player.status.position).abs() > 0.75
                    });
                let stale = match &vis {
                    Some((k, v)) => *k != key || !v.is_alive() || drifted,
                    None => true,
                };
                let fresh = vis_last_key != key;
                let backoff = if fresh {
                    Duration::from_millis(500)
                } else {
                    Duration::from_millis(500) * (1 << vis_fails.min(4))
                };
                if stale && vis_tried.elapsed() > backoff {
                    let died_young =
                        !fresh && vis_tried.elapsed() < backoff + Duration::from_secs(3);
                    vis_fails = if died_young { vis_fails + 1 } else { 0 };
                    vis_last_key = key.clone();
                    vis_tried = Instant::now();
                    let start = player.status.position;
                    vis = visualizer::Visualizer::start(target, 32, start)
                        .ok()
                        .map(|v| (key, v));
                    vis_clock = copy.then(|| (start, Instant::now()));
                }
                if let Some((_, v)) = &vis {
                    app.bands = v.bands();
                    dirty = true;
                }
            }
        }
        let now = Instant::now();
        let sliding = app.animating(now);
        // An animation in the visualizer (Spotify on Sonos) wants a new frame each time round.
        let animated = ui::fill_in(app, player).is_some() && !desktop.minimized;
        if animated {
            dirty = true;
        }
        if dirty || sliding {
            let browse = app.browse_fraction(now);
            terminal.draw(|frame| hits = ui::draw(frame, app, player, browse))?;
            dirty = std::mem::take(&mut app.cover_blink);
        }
        // Wait for a key: briefly while the browser slides, otherwise up to 50 ms, so mpv's
        // updates still show promptly.
        let wait = if sliding {
            16
        } else if vis.is_some() || animated {
            33
        } else {
            50
        };
        if event::poll(Duration::from_millis(wait))? {
            let event = event::read()?;
            let ctx = Ctx {
                queue: player.queue(),
                devices: &player.status.devices,
                output: &player.output,
                playlist: player.playlist.as_ref(),
            };
            let action = match event {
                Event::Key(key) if key.kind == KeyEventKind::Press => app.on_key(key, &ctx)?,
                Event::Mouse(mouse) => app.on_mouse(mouse, &hits, &ctx)?,
                Event::Paste(text) => {
                    app.on_paste(&text)?;
                    None
                }
                Event::FocusGained => {
                    app.on_focus();
                    None
                }
                _ => None,
            };
            if let Some(action) = action {
                match action {
                    Action::Quit => return Ok(Exit::Quit),
                    Action::Update if watch.ready.is_some() => {
                        let playing = !player.status.idle && !player.status.paused;
                        return Ok(Exit::Restart { resume: playing });
                    }
                    Action::Update => {
                        app.status = "There's no new version to switch to".into();
                    }
                    Action::Minimize => match &desktop.window {
                        Some(w) => {
                            w.hide();
                            desktop.minimized = true;
                            if let Some(tray) = &desktop.tray {
                                tray.update(|t| t.hidden = true);
                            }
                        }
                        None => {
                            app.status =
                                "Minimize works when OMAESTRO runs in its own Hyprland window"
                                    .into()
                        }
                    },
                    action => apply(action, app, player, scan_tx)?,
                }
            }
            dirty = true;
        }
        while let Ok(news) = news_rx.try_recv() {
            if let Some(action) = app.on_news(news)? {
                apply(action, app, player, scan_tx)?;
            }
            dirty = true;
        }
        while let Ok(event) = mpv_rx.try_recv() {
            player.handle(event)?;
            dirty = true;
        }
        player.tick()?;
        // Quit from the tray or a media widget.
        if desktop.obey(player)? {
            return Ok(Exit::Quit);
        }
        desktop.report(player, app);
        // When the next song starts by itself, the highlight moves with it if it was on the
        // one that just finished.
        let current = player.current_index();
        if current != last_current {
            if last_current == Some(app.queue_sel)
                && let Some(i) = current
            {
                app.queue_sel = i;
            }
            last_current = current;
        }
        if let Some(error) = player.status.last_error.take() {
            app.status = error;
        }
        while let Ok(msg) = scan_rx.try_recv() {
            match msg {
                ScanMsg::Progress(p) => app.on_scan_progress(p),
                ScanMsg::Done(result) => app.on_scan_done(result)?,
            }
            dirty = true;
        }
    }
}

fn apply(
    action: Action,
    app: &mut App,
    player: &mut Player,
    scan_tx: &Sender<ScanMsg>,
) -> Result<()> {
    match action {
        Action::PlayNow(track) => {
            app.status = format!("Playing {}", track.title);
            player.play_now(track)?;
            if let Some(i) = player.current_index() {
                app.queue_sel = i;
            }
        }
        Action::Enqueue(tracks) => {
            app.status = match tracks.as_slice() {
                [one] => format!("Added {} to the queue (Ctrl+Z takes it back)", one.title),
                many => format!(
                    "Added {} to the queue (Ctrl+Z takes them back)",
                    plural(many.len(), "song")
                ),
            };
            player.enqueue(tracks)?;
        }
        Action::Jump(index) => player.jump(index)?,
        Action::TogglePause => player.toggle_pause()?,
        Action::Stop => player.stop()?,
        Action::Volume(change) => player.set_volume(player.status.volume + change)?,
        Action::SetVolume(volume) => player.set_volume(volume)?,
        Action::SeekTo(fraction) => player.seek_to(fraction * player.status.duration)?,
        Action::Remove(index) => {
            player.remove(index)?;
            app.queue_sel = index.min(player.queue().len().saturating_sub(1));
        }
        Action::Move(index, up) => app.queue_sel = player.move_track(index, up)?,
        Action::MoveTo(from, to) => app.queue_sel = player.move_to(from, to)?,
        Action::Undo => {
            app.status = match player.undo()? {
                Some(what) => format!("Undone: {what}"),
                None => "Nothing to undo".into(),
            };
            app.queue_sel = app.queue_sel.min(player.queue().len().saturating_sub(1));
        }
        Action::Clear => {
            player.clear()?;
            app.queue_sel = 0;
            app.status = "Queue cleared (Ctrl+Z brings it back)".into();
        }
        Action::SavePlaylist(name) => {
            let path = playlist::file_for(&app.playlist_dir, &name);
            match playlist::save(&path, &name, player.queue()) {
                Ok(()) => {
                    app.status = format!("Saved {name}");
                    player.playlist = Some(player::Named {
                        name,
                        path,
                        saved: player.revision,
                    });
                }
                Err(e) => app.status = format!("Couldn't save {name}: {e:#}"),
            }
        }
        Action::LoadPlaylist {
            name,
            path,
            tracks,
            source,
        } => {
            // Opening only fills the queue: say how to start it.
            let from = match source {
                Source::Local => String::new(),
                other => format!(" from {}", other.name()),
            };
            app.status = format!(
                "Opened {name}{from}: {} · Space or ⏎ plays",
                plural(tracks.len(), "song")
            );
            // A Spotify or YouTube Music playlist keeps its name; s saves it under that name.
            let path = path.unwrap_or_else(|| playlist::file_for(&app.playlist_dir, &name));
            player.load(
                tracks,
                Some(player::Named {
                    name,
                    path,
                    saved: 0,
                }),
            )?;
            app.queue_sel = 0;
        }
        Action::RenamePlaylist(name) => {
            let path = playlist::file_for(&app.playlist_dir, &name);
            let renamed = match player.playlist.clone() {
                // Saved before: its file is renamed too.
                Some(old) if old.path.exists() && old.path != path => {
                    playlist::rename(&old.path, &path, &name).map(|()| old.saved)
                }
                Some(old) => Ok(old.saved),
                // A queue without a name yet: it gets one, not saved until s.
                None => Ok(u64::MAX),
            };
            match renamed {
                Ok(saved) => {
                    app.status = format!("Renamed to {name}");
                    player.playlist = Some(player::Named { name, path, saved });
                }
                Err(e) => app.status = format!("Couldn't rename it: {e:#}"),
            }
        }
        Action::Settings { rescan } => {
            app.config.save()?;
            app.apply_art();
            player.set_replaygain(&app.config.playback.replaygain)?;
            player.set_gapless(app.config.playback.gapless)?;
            if rescan && !app.scanning {
                app.scanning = true;
                start_scan(&app.config, scan_tx.clone());
            }
        }
        Action::Next => player.next()?,
        Action::Previous => player.previous()?,
        Action::Seek(seconds) => player.seek(seconds)?,
        Action::SetOutput(choice) => {
            match choice {
                app::OutputChoice::Here(name) => {
                    player.set_output(player::Output::Here(name.clone()))?;
                    app.config.playback.audio_device = name;
                    app.config.playback.output.clear();
                }
                app::OutputChoice::Sonos(uuid) => {
                    let group = app.sonos.as_ref().and_then(|h| h.group(&uuid)).cloned();
                    match group {
                        Some(group) => {
                            app.status = format!("Playing on {}", group.name());
                            player.set_output(player::Output::Sonos(group))?;
                            app.config.playback.output = format!("sonos:{uuid}");
                        }
                        None => {
                            app.status =
                                "That Sonos group has changed; look again in Settings".into()
                        }
                    }
                }
            }
            app.config.save()?;
        }
        Action::UseSonos => {
            if let Some(household) = app.sonos.clone() {
                let never = app.config.sonos.never.clone();
                player.set_sonos(
                    sonos::Sonos {
                        household: household.clone(),
                        never,
                    },
                    app.config.sonos.media_port,
                );
                // Remember where the speakers are, for a quick start next time.
                let ips: Vec<String> = household
                    .groups
                    .iter()
                    .map(|g| g.coordinator.ip.clone())
                    .collect();
                if ips != app.config.sonos.speakers {
                    app.config.sonos.speakers = ips;
                    app.config.save()?;
                }
                // Back on the Sonos group used last time (silently: nothing plays yet).
                if let Some(uuid) = app.config.playback.output.strip_prefix("sonos:")
                    && matches!(player.output, player::Output::Here(_))
                    && let Some(group) = household.group(uuid)
                {
                    player.set_output(player::Output::Sonos(group.clone()))?;
                }
            }
        }
        Action::Rescan => {
            app.scanning = true;
            start_scan(&app.config, scan_tx.clone());
        }
        Action::Quit | Action::Minimize | Action::Update => {}
    }
    Ok(())
}

fn plural(n: usize, word: &str) -> String {
    if n == 1 {
        format!("1 {word}")
    } else {
        format!("{n} {word}s")
    }
}

fn open_db() -> Result<Db> {
    Db::open(&config::cache_dir().join("library.db"))
}

fn cmd_scan() -> Result<()> {
    let config = Config::load()?;
    let roots = config.library_roots();
    let mut db = open_db()?;
    eprintln!("Scanning {}", config.library_paths.join(", "));

    let summary = scan::scan(&mut db, &roots, config.scan_threads, |p| {
        match p {
            Progress::Walking(n) => eprint!("\r  finding files… {n}"),
            Progress::Reading(done, total) => eprint!("\r  reading tags… {done}/{total}      "),
            Progress::Finishing => eprint!("\r  updating albums and search…          "),
        }
        let _ = std::io::stderr().flush();
    })?;

    eprintln!("\r  done.                                        ");
    println!(
        "{} files: {} new or changed, {} removed, {} unreadable",
        summary.files, summary.updated, summary.removed, summary.unreadable
    );
    println!(
        "time: {:.1}s total (finding {:.1}s, reading tags {:.1}s)",
        summary.total_ms as f64 / 1000.0,
        summary.walk_ms as f64 / 1000.0,
        summary.read_ms as f64 / 1000.0
    );
    for root in &summary.skipped_roots {
        println!(
            "warning: {root} was missing, empty or partly unreadable; nothing under it was removed"
        );
    }
    Ok(())
}

fn cmd_search(query: &str) -> Result<()> {
    let db = open_db()?;
    for t in db.search(query, 25)? {
        println!("{} — {} — {}  [{}]", t.artist, t.title, t.album, t.path);
    }
    Ok(())
}

fn cmd_stats() -> Result<()> {
    let db = open_db()?;
    let artists = db.artists()?;
    let albums: usize = artists
        .iter()
        .map(|a| db.albums_by(a).map(|v| v.len()).unwrap_or(0))
        .sum();
    println!(
        "{} tracks, {} albums, {} album artists",
        db.track_count()?,
        albums,
        artists.len()
    );
    Ok(())
}
