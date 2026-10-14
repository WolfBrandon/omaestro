//! OMAESTRO: one terminal music player for local files, Spotify, YouTube Music and Sonos.

mod animation;
mod app;
mod art;
mod browse;
mod columns;
mod config;
mod connect;
mod desktop;
mod library;
mod player;
mod playlist;
mod search;
mod secrets;
mod settings;
mod sonos;
mod spotify;
#[cfg(test)]
mod testutil;
mod theme;
mod ui;
mod visualizer;
mod youtube;

use anyhow::Result;
use app::{Action, App, Ctx, News};
use config::Config;
use library::Db;
use library::scan::{self, Progress, Summary};
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
        Some("--version" | "-V") => {
            println!("omaestro {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some("--help" | "-h") | Some(_) => {
            print!("{USAGE}");
            Ok(())
        }
        None => run_player(),
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

fn run_player() -> Result<()> {
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
    let mut app = App::new(open_db()?, config, news_tx)?;

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
    );
    let _ = execute!(
        std::io::stdout(),
        DisableMouseCapture,
        DisableFocusChange,
        DisableBracketedPaste
    );
    ratatui::restore();
    // Quitting stops the music, wherever it plays; the volume is kept for next time.
    if matches!(player.output, player::Output::Here(_)) {
        app.config.playback.volume = player.status.volume.round() as u8;
    }
    let _ = app.config.save();
    player.shut_down();
    result
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
) -> Result<()> {
    let mut dirty = true;
    let mut theme_setting = app.config.appearance.theme.clone();
    let mut theme_file = ThemeFile::for_setting(&theme_setting);
    let mut theme_checked = Instant::now() - Duration::from_secs(60);
    let mut last_current = player.current_index();
    let mut hits = Vec::new();
    let mut had_overlay = false;
    // The visualizer's capture, which target it listens to, and when it last tried to start.
    let mut vis: Option<(String, visualizer::Visualizer)> = None;
    let mut vis_tried = Instant::now() - Duration::from_secs(1);
    // For a silent copy (Sonos): where in the song it started and when, to keep it in step.
    let mut vis_clock: Option<(f64, Instant)> = None;
    loop {
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
                if stale && vis_tried.elapsed() > Duration::from_millis(500) {
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
                    Action::Quit => return Ok(()),
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
        if desktop.obey(player)? {
            return Ok(());
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
        Action::LoadPlaylist { name, path, tracks } => {
            // Opening only fills the queue: say how to start it.
            app.status = format!(
                "Opened {name}: {} · Space or ⏎ plays",
                plural(tracks.len(), "song")
            );
            // Imported playlists have no file yet: s asks for a name to save them under.
            let named = path.map(|path| player::Named {
                name,
                path,
                saved: 0,
            });
            player.load(tracks, named)?;
            app.queue_sel = 0;
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
        Action::Quit | Action::Minimize => {}
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
