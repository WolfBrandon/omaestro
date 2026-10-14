//! What's on screen and what each key or click does. Nothing here touches mpv or draws
//! anything: input turns into `Action`s that the main loop carries out, and `ui` draws this
//! state. Slow work (searching online, signing in) runs on background threads and comes back
//! as `News`.

use crate::browse::{Browse, Catalog, Kind, Pane, Step, movement, step};
use crate::config::Config;
use crate::connect::{self, SpotifyStep, YouTubeStep};
use crate::library::scan::{Progress, Summary};
use crate::library::{Db, Source, Track};
use crate::player::{AudioDevice, Named, Output};
use crate::playlist::{self, Saved};
use crate::search::{self, Found, Search};
use crate::settings::{self, Facts, Settings};
use crate::spotify::{self, Spotify, soloist};
use crate::theme::Theme;
use crate::{secrets, youtube};
use anyhow::Result;
use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::{Position, Rect};
use std::cell::Cell;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How long the browser takes to slide up or down.
const SLIDE: Duration = Duration::from_millis(150);
/// Two clicks on the same row within this time are a double-click.
const DOUBLE_CLICK: Duration = Duration::from_millis(400);
/// Clicks are ignored this long after the window gains focus, so the click that focuses the
/// window doesn't also press a button.
const FOCUS_GRACE: Duration = Duration::from_millis(200);
/// Online searches wait this long after the last key, so typing doesn't send a request per
/// letter.
const SEARCH_PAUSE: Duration = Duration::from_millis(350);

/// Something for the main loop to do.
#[derive(Debug, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum Action {
    /// Play this song now, after the one playing.
    PlayNow(Track),
    Enqueue(Vec<Track>),
    /// Play this queue entry.
    Jump(usize),
    TogglePause,
    Stop,
    Next,
    Previous,
    /// Seek by this many seconds.
    Seek(f64),
    /// Seek to this point of the song, 0 to 1.
    SeekTo(f64),
    /// Change the volume by this much.
    Volume(f64),
    SetVolume(f64),
    /// Play here from now on.
    SetOutput(OutputChoice),
    /// The Sonos speakers were found: hand them to the player.
    UseSonos,
    /// Take this queue entry out.
    Remove(usize),
    /// Move a queue entry to another place (dragged).
    MoveTo(usize, usize),
    /// Move a queue entry up (true) or down.
    Move(usize, bool),
    Undo,
    Clear,
    /// Save the queue as a playlist with this name.
    SavePlaylist(String),
    /// Put a playlist in the queue. `path` is the file for saved ones, none for imported ones.
    LoadPlaylist {
        name: String,
        path: Option<PathBuf>,
        tracks: Vec<Track>,
    },
    /// The settings changed: save them and apply what the player needs.
    Settings {
        rescan: bool,
    },
    /// Tuck the window away into the Hyprland scratchpad.
    Minimize,
    Rescan,
    Quit,
}

/// Results of background work.
#[derive(Debug)]
pub enum News {
    SpotifySignedIn(Result<String, String>),
    Soloist(soloist::Setup),
    YouTubeSignedIn(Result<String, String>),
    /// Search results from one source, for search number `generation`.
    Found {
        generation: u64,
        source: Source,
        result: Result<(Vec<Track>, usize), String>,
    },
    /// A service's playlists, for the Open list.
    Lists {
        source: Source,
        result: Result<Vec<RemoteList>, String>,
    },
    /// The Sonos household, or why it couldn't be found.
    Sonos(Result<crate::sonos::Household, String>),
    /// A song's cover, read in the background (none if it has none).
    Cover {
        key: String,
        image: Option<image::DynamicImage>,
    },
    /// A service's playlist, fetched to go in the queue.
    Imported {
        name: String,
        source: Source,
        result: Result<Vec<Track>, String>,
    },
}

/// Player details the input handling needs.
pub struct Ctx<'a> {
    pub queue: &'a [Track],
    pub devices: &'a [AudioDevice],
    pub output: &'a Output,
    /// The saved playlist the queue holds, if any.
    pub playlist: Option<&'a Named>,
}

/// A place music can play, as the output picker lists it: an mpv device name on this
/// computer, or a Sonos group by its coordinator's id.
#[derive(Debug, Clone, PartialEq)]
pub enum OutputChoice {
    Here(String),
    Sonos(String),
}

impl OutputChoice {
    /// The id kept in the favourites and hidden lists.
    pub fn id(&self) -> &str {
        match self {
            OutputChoice::Here(name) | OutputChoice::Sonos(name) => name,
        }
    }
}

/// One row of the output picker.
#[derive(Debug, Clone, PartialEq)]
pub struct PickerRow {
    pub choice: OutputChoice,
    pub name: String,
    pub note: String,
    pub favourite: bool,
    pub hidden: bool,
    pub current: bool,
    /// A home theater: playing there switches the TV's sound.
    pub tv: bool,
}

/// A Spotify or YouTube Music playlist in the Open list.
#[derive(Debug, Clone, PartialEq)]
pub struct RemoteList {
    pub id: String,
    pub name: String,
    pub tracks: Option<usize>,
}

/// One tab of the Open list for a service.
#[derive(Debug, Clone, PartialEq)]
pub enum Listing {
    NotConnected,
    Loading,
    Ready(Vec<RemoteList>),
    Failed(String),
}

/// The Open list: your saved playlists, and those on Spotify and YouTube Music.
#[derive(Debug, Clone, PartialEq)]
pub struct Open {
    pub tab: Source,
    pub sel: usize,
    pub local: Vec<Saved>,
    pub spotify: Listing,
    pub youtube: Listing,
}

impl Open {
    pub fn len(&self) -> usize {
        match self.tab {
            Source::Local => self.local.len(),
            Source::Spotify => listing_len(&self.spotify),
            Source::YouTube => listing_len(&self.youtube),
        }
    }
}

fn listing_len(l: &Listing) -> usize {
    match l {
        Listing::Ready(list) => list.len(),
        _ => 0,
    }
}

/// A box drawn over the screen. Only one at a time.
pub enum Overlay {
    Help,
    /// The queue's show/hide-columns menu; the highlighted row.
    Columns(usize),
    /// The output picker: the highlighted row, and whether hidden outputs show.
    Picker {
        sel: usize,
        all: bool,
    },
    SaveAs(String),
    Open(Open),
    /// The Search screen; its state lives in `App::search` so it's kept between visits.
    Search,
    Settings(Settings),
    Spotify(SpotifyStep),
    YouTube(YouTubeStep),
}

/// Something on screen that reacts to the mouse. The screen code lists where each one is on
/// every frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Target {
    Stop,
    Previous,
    PlayPause,
    Next,
    /// The volume slider: where you click sets the level.
    Volume,
    /// The song position bar: where you click is where it plays from.
    Seek,
    Output,
    Settings,
    Help,
    Save,
    SaveAs,
    Open,
    Clear,
    /// A queue row.
    Queue(usize),
    /// The queue pane, for the scroll wheel.
    QueueList,
    BrowseSearch,
    Toggle(Kind),
    /// A row of the browser's results.
    Result(usize),
    Results,
    /// A row of the browser's song list.
    Song(usize),
    Songs,
    /// A row in the output picker.
    Device(usize),
    /// A row in the Open list, and its tabs.
    Playlist(usize),
    OpenTab(Source),
    /// The Search screen's box, source chips and result rows.
    SearchBox,
    /// The × at the top right of Search.
    CloseSearch,
    SearchChip(Source),
    SearchRow(usize),
    /// Settings: a section in the list on the left, a row, one of a row's choices, a button.
    SettingsSection(usize),
    SettingsRow(usize),
    SettingsChoice(usize, usize),
    SettingsButton(usize),
    /// A browser in the YouTube Music sign-in list.
    Browser(usize),
    /// The list icon at the queue's top left: the show/hide-columns menu.
    ColumnsMenu,
    /// A row of that menu.
    ColumnItem(usize),
    /// The right edge of a queue column's heading, dragged to resize it; where the column
    /// starts, and the widest it can go (until the last column is as narrow as it gets).
    ColumnEdge(crate::columns::Column, u16, u16),
    /// A web address on a sign-in screen (opens it), and its Copy button.
    Link(connect::Link),
    Copy(connect::Link),
    /// The inside of a popup: clicks there don't close it.
    Popup,
}

impl Target {
    /// Things inside popups, which a click shouldn't close the popup over.
    fn in_popup(self) -> bool {
        matches!(
            self,
            Target::Popup
                | Target::Device(_)
                | Target::Playlist(_)
                | Target::OpenTab(_)
                | Target::SearchBox
                | Target::CloseSearch
                | Target::SearchChip(_)
                | Target::SearchRow(_)
                | Target::SettingsSection(_)
                | Target::SettingsRow(_)
                | Target::SettingsChoice(..)
                | Target::SettingsButton(_)
                | Target::Browser(_)
                | Target::Link(_)
                | Target::Copy(_)
                | Target::ColumnItem(_)
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hit {
    pub area: Rect,
    pub target: Target,
}

/// The topmost thing at a screen position (popups are listed last).
pub fn hit_at(hits: &[Hit], x: u16, y: u16) -> Option<Hit> {
    hits.iter()
        .rev()
        .find(|h| h.area.contains(Position { x, y }))
        .copied()
}

/// How far along a slider a column is, 0 to 1.
fn fraction(area: Rect, x: u16) -> f64 {
    let span = area.width.saturating_sub(1).max(1) as f64;
    (x.saturating_sub(area.x) as f64 / span).clamp(0.0, 1.0)
}

/// Something being dragged with the mouse.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Drag {
    /// A queue row, from where it was to the row under the mouse.
    Row { from: usize, over: usize },
    /// A column's right edge; where the column starts.
    Column {
        column: crate::columns::Column,
        start: u16,
    },
}

/// The browser on its way up or down.
struct Slide {
    start: Instant,
    from: f32,
    to: f32,
}

pub struct App {
    db: Db,
    pub config: Config,
    news: Sender<News>,
    pub catalog: Catalog,
    pub browse: Browse,
    pub browse_open: bool,
    slide: Option<Slide>,
    pub search: Search,
    /// The newest search, shared with the threads asking Spotify and YouTube Music.
    latest_search: Arc<AtomicU64>,
    /// The selected queue row (the highlight bar).
    pub queue_sel: usize,
    /// First queue row on screen; kept by the screen code as the selection moves.
    pub queue_top: Cell<usize>,
    /// What the mouse is dragging: a queue row, or a column's edge.
    pub drag: Option<Drag>,
    /// Where the list icon was drawn, so its menu can hang from it.
    pub columns_anchor: Cell<Option<Rect>>,
    pub overlay: Option<Overlay>,
    /// Settings, put aside while a sign-in screen opened from it is showing.
    parked_settings: Option<Settings>,
    /// Where playlists are saved.
    pub playlist_dir: PathBuf,
    /// One line of news for the status bar.
    pub status: String,
    pub scanning: bool,
    /// Tracks in the index, for Settings.
    track_count: usize,
    pub theme: Theme,
    /// The theme was picked in Settings (not Omarchy's), so its background is painted.
    pub theme_fixed: bool,
    /// The Omarchy themes installed, for Settings; looked up again when Settings opens.
    pub themes: Vec<crate::theme::Installed>,
    pub spotify: Option<Arc<Mutex<Spotify>>>,
    spotify_login: Option<spotify::Login>,
    /// The Sonos household once found, and what to say about it in Settings.
    pub sonos: Option<crate::sonos::Household>,
    sonos_status: String,
    /// Turns pictures into terminal graphics; found out from the terminal at start.
    pub picker: Option<ratatui_image::picker::Picker>,
    /// The best picture protocol the terminal said it has, when it was asked at start.
    pub best_pictures: Option<ratatui_image::picker::ProtocolType>,
    /// The playing song's cover as a picture, kept to redraw it when the setting changes.
    cover_image: Option<image::DynamicImage>,
    /// The playing song's cover, ready to draw, and the song it was asked for.
    pub cover: Option<ratatui_image::protocol::Protocol>,
    cover_wanted: Option<String>,
    /// The same cover as a file, for the desktop's media widget.
    pub cover_file: Option<PathBuf>,
    /// Leave the cover out of the next frame (see the main loop).
    pub cover_blink: bool,
    /// The visualizer's bars, 0 to 1, refreshed by the main loop while it listens.
    pub bands: Vec<f32>,
    /// Made on first use: it reads the site's current API version.
    youtube: Arc<Mutex<Option<youtube::Client>>>,
    pub youtube_account: Option<youtube::Account>,
    /// The button or row the mouse went down on, and when the last click landed.
    pressed: Option<Target>,
    last_click: Option<(Target, Instant)>,
    ignore_clicks_until: Option<Instant>,
}

impl App {
    pub fn new(db: Db, config: Config, news: Sender<News>) -> Result<App> {
        let mut app = App {
            db,
            config,
            news,
            catalog: Catalog::default(),
            browse: Browse::default(),
            browse_open: false,
            slide: None,
            search: Search::default(),
            latest_search: Arc::new(AtomicU64::new(0)),
            queue_sel: 0,
            queue_top: Cell::new(0),
            drag: None,
            columns_anchor: Cell::new(None),
            overlay: None,
            parked_settings: None,
            playlist_dir: playlist::dir(),
            status: String::new(),
            scanning: false,
            track_count: 0,
            theme: Theme::default(),
            theme_fixed: false,
            themes: crate::theme::installed(),
            spotify: Spotify::load().map(|s| Arc::new(Mutex::new(s))),
            spotify_login: None,
            sonos: None,
            sonos_status: "Not looked for yet".into(),
            picker: None,
            best_pictures: None,
            cover_image: None,
            cover: None,
            cover_wanted: None,
            cover_file: None,
            cover_blink: false,
            bands: Vec::new(),
            youtube: Arc::new(Mutex::new(None)),
            youtube_account: youtube::account(),
            pressed: None,
            last_click: None,
            ignore_clicks_until: None,
        };
        app.reload()?;
        Ok(app)
    }

    /// Re-reads artists and albums from the index. The browser keeps its search and selection.
    pub fn reload(&mut self) -> Result<()> {
        self.catalog = Catalog::load(&self.db)?;
        self.track_count = self.db.track_count()? as usize;
        self.browse.refresh(&self.catalog, &self.db)
    }

    pub fn on_scan_progress(&mut self, progress: Progress) {
        self.scanning = true;
        self.status = match progress {
            Progress::Walking(_) => "Checking the library for changes…".into(),
            Progress::Reading(done, total) => format!("Reading tags… {done}/{total}"),
            Progress::Finishing => "Updating albums and search…".into(),
        };
    }

    pub fn on_scan_done(&mut self, result: Result<Summary, String>) -> Result<()> {
        self.scanning = false;
        match result {
            Ok(s) => {
                self.status = if s.updated == 0 && s.removed == 0 {
                    format!("Library up to date: {} tracks", group_thousands(s.files))
                } else {
                    format!(
                        "Library: {} tracks ({} new or changed, {} removed)",
                        group_thousands(s.files),
                        s.updated,
                        s.removed
                    )
                };
                if !s.skipped_roots.is_empty() {
                    self.status
                        .push_str(" · some folders couldn't be read, so nothing was removed");
                }
                if s.updated > 0 || s.removed > 0 || self.browse.rows.len() < 4 {
                    self.reload()?;
                }
            }
            Err(e) => self.status = format!("Library scan failed: {e}"),
        }
        Ok(())
    }

    /// How far up the browser is: 0 hidden, 1 fully open.
    pub fn browse_fraction(&self, now: Instant) -> f32 {
        match &self.slide {
            Some(s) => {
                let t = (now.duration_since(s.start).as_secs_f32() / SLIDE.as_secs_f32()).min(1.0);
                // Ease out: quick at first, settling gently.
                let eased = 1.0 - (1.0 - t).powi(3);
                s.from + (s.to - s.from) * eased
            }
            None if self.browse_open => 1.0,
            None => 0.0,
        }
    }

    /// Whether the browser is moving, so the main loop should draw a frame now. True once
    /// more when the slide ends, for the frame where it's all the way up or down.
    pub fn animating(&mut self, now: Instant) -> bool {
        match &self.slide {
            Some(s) if now.duration_since(s.start) < SLIDE => true,
            Some(_) => {
                self.slide = None;
                true
            }
            None => false,
        }
    }

    fn set_browse(&mut self, open: bool) {
        if open == self.browse_open {
            return;
        }
        let now = Instant::now();
        self.slide = Some(Slide {
            start: now,
            from: self.browse_fraction(now),
            to: if open { 1.0 } else { 0.0 },
        });
        self.browse_open = open;
    }

    pub fn open_browse(&mut self) {
        self.browse.pane = Pane::Search;
        self.set_browse(true);
    }

    pub fn close_browse(&mut self) {
        self.set_browse(false);
    }

    /// The window was focused: ignore the click that did it.
    pub fn on_focus(&mut self) {
        self.ignore_clicks_until = Some(Instant::now() + FOCUS_GRACE);
    }

    fn open_picker(&mut self, ctx: &Ctx) {
        let rows = self.picker_rows(ctx, false);
        let sel = rows.iter().position(|r| r.current).unwrap_or(0);
        self.overlay = Some(Overlay::Picker { sel, all: false });
    }

    /// Everything the output picker lists, in one list: this computer's devices, then the
    /// Sonos groups, starred ones first, hidden ones only with `all` (and the current one
    /// always).
    pub fn picker_rows(&self, ctx: &Ctx, all: bool) -> Vec<PickerRow> {
        let favourites = &self.config.outputs.favourites;
        let hidden = &self.config.outputs.hidden;
        let mut here: Vec<PickerRow> = ctx
            .devices
            .iter()
            .map(|d| {
                let default = d.name == "pipewire";
                let current = match ctx.output {
                    Output::Here(dev) => *dev == d.name || (dev.is_empty() && default),
                    Output::Sonos(_) => false,
                };
                PickerRow {
                    choice: OutputChoice::Here(d.name.clone()),
                    name: if default {
                        "Default output".into()
                    } else {
                        short_device(&d.description)
                    },
                    note: if default {
                        "follows the desktop".into()
                    } else {
                        String::new()
                    },
                    favourite: favourites.contains(&d.name),
                    hidden: hidden.contains(&d.name),
                    current,
                    tv: false,
                }
            })
            .collect();
        let sonos: Vec<PickerRow> = self
            .sonos
            .iter()
            .flat_map(|h| &h.groups)
            .map(|g| {
                let id = g.coordinator.uuid.clone();
                PickerRow {
                    current: matches!(ctx.output, Output::Sonos(o) if o.coordinator.uuid == id),
                    favourite: favourites.contains(&id),
                    hidden: hidden.contains(&id),
                    name: g.name(),
                    note: if g.home_theater {
                        "switches the TV".into()
                    } else if g.rooms.len() > 1 {
                        "group".into()
                    } else {
                        String::new()
                    },
                    tv: g.home_theater,
                    choice: OutputChoice::Sonos(id),
                }
            })
            .collect();
        // One list: this computer's outputs, then the Sonos groups, starred ones first.
        here.extend(sonos);
        here.sort_by_key(|r| !r.favourite);
        if !all {
            here.retain(|r| !r.hidden || r.current);
        }
        here
    }

    /// Asks for the playing song's cover, if it's a different one from last time.
    pub fn want_cover(&mut self, track: Option<&Track>) {
        let key = track
            .filter(|_| self.config.appearance.album_art != crate::config::Art::Off)
            .map(crate::art::key);
        if key == self.cover_wanted {
            return;
        }
        self.cover = None;
        self.cover_image = None;
        self.cover_file = None;
        self.cover_wanted = key.clone();
        if let (Some(key), Some(track)) = (key, track.cloned()) {
            let news = self.news.clone();
            std::thread::spawn(move || {
                let image = crate::art::load(&track);
                let _ = news.send(News::Cover { key, image });
            });
        }
    }

    /// Follows the Album art setting: switches between pictures and coloured blocks, and
    /// redraws the cover that's showing.
    pub fn apply_art(&mut self) {
        use crate::config::Art;
        use ratatui_image::picker::{Picker, ProtocolType};
        let art = self.config.appearance.album_art;
        if art == Art::Off {
            self.cover = None;
            return;
        }
        let picker = self.picker.get_or_insert_with(Picker::halfblocks);
        let wanted = match (art, self.best_pictures) {
            (Art::HiRes, Some(best)) => best,
            (Art::HiRes, None) => {
                self.status = "Hi-res covers start next time OMAESTRO opens (it asks the \
                               terminal then)"
                    .into();
                ProtocolType::Halfblocks
            }
            _ => ProtocolType::Halfblocks,
        };
        if picker.protocol_type() != wanted {
            picker.set_protocol_type(wanted);
        }
        if let Some(img) = &self.cover_image {
            self.cover = crate::art::encode(picker, img);
            self.cover_blink = true;
        } else if self.cover.is_none() {
            // Off before: fetch the playing song's cover again.
            self.cover_wanted = None;
        }
    }

    /// Looks for the Sonos speakers in the background (read-only).
    pub fn find_sonos(&mut self) {
        self.sonos_status = "Looking for speakers…".into();
        let known = self.config.sonos.speakers.clone();
        let news = self.news.clone();
        std::thread::spawn(move || {
            let result = crate::sonos::discover(&known).map_err(|e| format!("{e:#}"));
            let _ = news.send(News::Sonos(result));
        });
    }

    /// Stars or hides an output (or undoes it), saving the choice.
    fn mark_output(&mut self, id: &str, favourite: bool) -> Option<Action> {
        let outputs = &mut self.config.outputs;
        let list = if favourite {
            &mut outputs.favourites
        } else {
            &mut outputs.hidden
        };
        match list.iter().position(|x| x == id) {
            Some(i) => {
                list.remove(i);
            }
            None => list.push(id.to_string()),
        }
        Some(Action::Settings { rescan: false })
    }

    // ---- playlists ---------------------------------------------------------------------

    fn open_playlists(&mut self) {
        let open = Open {
            tab: Source::Local,
            sel: 0,
            local: playlist::list(&self.playlist_dir),
            spotify: if self.spotify.is_some() {
                Listing::Loading
            } else {
                Listing::NotConnected
            },
            youtube: if self.youtube_account.is_some() {
                Listing::Loading
            } else {
                Listing::NotConnected
            },
        };
        if open.spotify == Listing::Loading {
            self.fetch_lists(Source::Spotify);
        }
        if open.youtube == Listing::Loading {
            self.fetch_lists(Source::YouTube);
        }
        self.overlay = Some(Overlay::Open(open));
    }

    /// Asks a service for its playlists in the background.
    fn fetch_lists(&self, source: Source) {
        let news = self.news.clone();
        match source {
            Source::Spotify => {
                let Some(spotify) = self.spotify.clone() else {
                    return;
                };
                std::thread::spawn(move || {
                    let result = (|| -> Result<Vec<RemoteList>> {
                        let mut s = spotify.lock().map_err(|_| anyhow::anyhow!("busy"))?;
                        let mut out = vec![RemoteList {
                            id: "liked".into(),
                            name: "Liked Songs".into(),
                            tracks: None,
                        }];
                        out.extend(s.playlists()?.into_iter().map(|p| RemoteList {
                            id: p.id,
                            name: p.name,
                            tracks: Some(p.tracks),
                        }));
                        Ok(out)
                    })();
                    let _ = news.send(News::Lists {
                        source: Source::Spotify,
                        result: result.map_err(|e| format!("{e:#}")),
                    });
                });
            }
            Source::YouTube => {
                let client = self.youtube.clone();
                std::thread::spawn(move || {
                    let result = with_youtube(&client, |c| c.playlists()).map(|lists| {
                        lists
                            .into_iter()
                            .map(|p| RemoteList {
                                id: p.id,
                                name: p.name,
                                tracks: None,
                            })
                            .collect()
                    });
                    let _ = news.send(News::Lists {
                        source: Source::YouTube,
                        result,
                    });
                });
            }
            Source::Local => {}
        }
    }

    /// Fetches a service's playlist for the queue, in the background.
    fn import(&mut self, source: Source, list: RemoteList) {
        self.status = format!("Loading {} from {}…", list.name, source.name());
        let news = self.news.clone();
        let spotify = self.spotify.clone();
        let youtube = self.youtube.clone();
        std::thread::spawn(move || {
            let result = match source {
                Source::Spotify => (|| -> Result<Vec<Track>> {
                    let spotify = spotify.ok_or_else(|| anyhow::anyhow!("not connected"))?;
                    let mut s = spotify.lock().map_err(|_| anyhow::anyhow!("busy"))?;
                    if list.id == "liked" {
                        s.liked(2000)
                    } else {
                        s.playlist_tracks(&list.id)
                    }
                })()
                .map_err(|e| format!("{e:#}")),
                _ => with_youtube(&youtube, |c| c.playlist_tracks(&list.id, 2000)),
            };
            let _ = news.send(News::Imported {
                name: list.name,
                source,
                result,
            });
        });
    }

    /// Save to the playlist the queue belongs to, or ask for a name.
    fn save(&mut self, ctx: &Ctx) -> Option<Action> {
        if ctx.queue.is_empty() {
            self.status = "The queue is empty, so there's nothing to save".into();
            return None;
        }
        match ctx.playlist {
            Some(p) => Some(Action::SavePlaylist(p.name.clone())),
            None => {
                self.overlay = Some(Overlay::SaveAs(String::new()));
                None
            }
        }
    }

    fn save_as(&mut self, ctx: &Ctx) {
        if ctx.queue.is_empty() {
            self.status = "The queue is empty, so there's nothing to save".into();
            return;
        }
        let name = ctx.playlist.map(|p| p.name.clone()).unwrap_or_default();
        self.overlay = Some(Overlay::SaveAs(name));
    }

    /// Reads a saved playlist for the queue.
    fn load_playlist(&mut self, saved: &Saved) -> Option<Action> {
        match playlist::load(&saved.path, &self.db) {
            Ok((name, tracks)) => Some(Action::LoadPlaylist {
                name,
                path: Some(saved.path.clone()),
                tracks,
            }),
            Err(e) => {
                self.status = format!("Couldn't open {}: {e:#}", saved.name);
                None
            }
        }
    }

    /// Enter (or a double-click) in the Open list.
    fn open_chosen(&mut self) -> Option<Action> {
        let Some(Overlay::Open(open)) = &self.overlay else {
            return None;
        };
        let (tab, sel) = (open.tab, open.sel);
        let chosen = match tab {
            Source::Local => open.local.get(sel).cloned().map(Err),
            Source::Spotify => match &open.spotify {
                Listing::Ready(l) => l.get(sel).cloned().map(Ok),
                _ => None,
            },
            Source::YouTube => match &open.youtube {
                Listing::Ready(l) => l.get(sel).cloned().map(Ok),
                _ => None,
            },
        }?;
        self.overlay = None;
        match chosen {
            Err(saved) => self.load_playlist(&saved),
            Ok(remote) => {
                self.import(tab, remote);
                None
            }
        }
    }

    // ---- search ------------------------------------------------------------------------

    pub fn open_search(&mut self) {
        self.search.typing = true;
        self.overlay = Some(Overlay::Search);
    }

    /// Searches every shown source for the current words. Local answers at once; the online
    /// services a moment later, on their own threads.
    pub fn run_search(&mut self) -> Result<()> {
        let generation = self.search.generation + 1;
        self.search.generation = generation;
        self.latest_search.store(generation, Ordering::Relaxed);
        let query = self.search.query.trim().to_string();
        let shown = self.search.show.iter().filter(|s| **s).count();
        for source in search::SOURCES {
            let i = search::index(source);
            if !self.search.show[i] || query.is_empty() {
                self.search.found[i] = Found::Idle;
                continue;
            }
            match source {
                Source::Local => {
                    let limit = if shown > 1 { 10 } else { 100 };
                    let songs = self.db.search(&query, limit)?;
                    let total = self.db.count(&query)?;
                    self.search.found[i] = Found::Songs(songs, total);
                }
                Source::Spotify => match self.spotify.clone() {
                    None => {
                        self.search.found[i] =
                            Found::Unavailable("Connect Spotify in Settings to search it".into())
                    }
                    Some(spotify) => {
                        self.search.found[i] = Found::Searching;
                        self.ask(
                            generation,
                            source,
                            move |q| {
                                let mut s = spotify.lock().map_err(|_| "busy".to_string())?;
                                let songs = s.search(q).map_err(|e| format!("{e:#}"))?;
                                let n = songs.len();
                                Ok((songs, n))
                            },
                            query.clone(),
                        );
                    }
                },
                Source::YouTube => {
                    self.search.found[i] = Found::Searching;
                    let client = self.youtube.clone();
                    self.ask(
                        generation,
                        source,
                        move |q| {
                            let songs = with_youtube(&client, |c| c.search(q))?;
                            let n = songs.len();
                            Ok((songs, n))
                        },
                        query.clone(),
                    );
                }
            }
        }
        self.search.settle();
        Ok(())
    }

    /// Runs one online search after a short pause, unless a newer search has started.
    fn ask(
        &self,
        generation: u64,
        source: Source,
        search: impl FnOnce(&str) -> Result<(Vec<Track>, usize), String> + Send + 'static,
        query: String,
    ) {
        let news = self.news.clone();
        let latest = self.latest_search.clone();
        std::thread::spawn(move || {
            std::thread::sleep(SEARCH_PAUSE);
            if latest.load(Ordering::Relaxed) != generation {
                return;
            }
            let result = search(&query);
            let _ = news.send(News::Found {
                generation,
                source,
                result,
            });
        });
    }

    // ---- settings and signing in -----------------------------------------------------

    pub fn open_settings(&mut self) {
        self.themes = crate::theme::installed();
        let rows = settings::rows(&self.config, &self.facts());
        self.overlay = Some(Overlay::Settings(Settings::new(&rows)));
    }

    /// What Settings shows about the library and accounts.
    pub fn facts(&self) -> Facts {
        let soloist_ready = soloist::installed() && soloist::key().is_some() && soloist::paired();
        Facts {
            tracks: self.track_count,
            albums: self.catalog.album_count(),
            spotify: self
                .spotify
                .as_ref()
                .and_then(|s| s.lock().ok().map(|s| s.user().to_string())),
            soloist: if soloist_ready {
                "Ready: Spotify Soloist plays here".into()
            } else if soloist::key().is_some() {
                "Soloist isn't finished setting up".into()
            } else {
                "Not set up (needed to play Spotify here)".into()
            },
            soloist_ready,
            youtube: self.youtube_account.as_ref().map(|a| a.user.clone()),
            sonos: self.sonos_status.clone(),
            themes: self.themes.clone(),
            omarchy_theme: crate::theme::omarchy_current(),
        }
    }

    /// Leaves a sign-in screen: back to Settings if that's where it was opened from.
    fn close_connect(&mut self) {
        if let Some(login) = self.spotify_login.take() {
            login.cancel();
        }
        self.overlay = self.parked_settings.take().map(Overlay::Settings);
    }

    /// Puts Settings aside (keeping its place) while a sign-in screen shows.
    fn park_settings(&mut self) {
        if let Some(Overlay::Settings(s)) = self.overlay.take() {
            self.parked_settings = Some(s);
        }
    }

    fn connect(&mut self, source: Source) {
        self.park_settings();
        self.overlay = Some(match source {
            Source::Spotify => Overlay::Spotify(SpotifyStep::start(self.spotify.is_some())),
            _ => Overlay::YouTube(YouTubeStep::start(youtube::logins())),
        });
    }

    fn sign_out(&mut self, source: Source) {
        match source {
            Source::Spotify => {
                Spotify::sign_out();
                self.spotify = None;
                self.status = "Signed out of Spotify".into();
            }
            _ => {
                youtube::sign_out();
                self.youtube_account = None;
                // The next search makes a fresh, signed-out client.
                if let Ok(mut c) = self.youtube.lock() {
                    *c = None;
                }
                self.status = "Signed out of YouTube Music".into();
            }
        }
    }

    fn settings_outcome(
        &mut self,
        outcome: settings::Outcome,
        folders_before: Vec<String>,
    ) -> Option<Action> {
        use settings::Outcome as O;
        match outcome {
            O::Stay => None,
            O::Close => {
                self.overlay = None;
                None
            }
            O::Saved => Some(Action::Settings {
                rescan: self.config.library_paths != folders_before,
            }),
            O::Do(action) => Some(action),
            O::Connect(source) => {
                self.connect(source);
                None
            }
            O::SignOut(source) => {
                self.sign_out(source);
                None
            }
            O::SetUpSoloist => {
                self.park_settings();
                self.overlay = Some(Overlay::Spotify(SpotifyStep::start(true)));
                None
            }
            O::RemoveSoloist => {
                soloist::forget();
                self.status = "Removed the Soloist key and pairing".into();
                None
            }
            O::FindSonos => {
                self.find_sonos();
                None
            }
        }
    }

    fn connect_outcome(&mut self, outcome: connect::Outcome) {
        use connect::Outcome as O;
        match outcome {
            O::Stay => {}
            O::Close => self.close_connect(),
            O::SpotifyLogin(client_id) => {
                let news = self.news.clone();
                match spotify::login(&client_id, move |r| {
                    let _ = news.send(News::SpotifySignedIn(r));
                }) {
                    Ok(login) => {
                        let _ = secrets::open_in_browser(&login.url);
                        self.overlay = Some(Overlay::Spotify(SpotifyStep::Waiting {
                            url: login.url.clone(),
                        }));
                        self.spotify_login = Some(login);
                    }
                    Err(e) => {
                        self.overlay = Some(Overlay::Spotify(SpotifyStep::ClientId {
                            input: client_id,
                            error: Some(format!("{e:#}")),
                        }))
                    }
                }
            }
            O::SoloistKey(key) => match soloist::save_key(&key) {
                Ok(()) => {
                    self.overlay =
                        Some(Overlay::Spotify(SpotifyStep::SettingUp("Starting…".into())));
                    let news = self.news.clone();
                    soloist::set_up(move |s| {
                        let _ = news.send(News::Soloist(s));
                    });
                }
                Err(e) => {
                    self.overlay = Some(Overlay::Spotify(SpotifyStep::Playback {
                        input: key,
                        error: Some(format!("{e:#}")),
                    }))
                }
            },
            O::YouTubeBrowser(login) => {
                self.overlay = Some(Overlay::YouTube(YouTubeStep::Connecting(login.label)));
                let news = self.news.clone();
                std::thread::spawn(move || {
                    let result = youtube::connect(&login.arg).map_err(|e| format!("{e:#}"));
                    let _ = news.send(News::YouTubeSignedIn(result));
                });
            }
        }
    }

    /// Background work finished.
    pub fn on_news(&mut self, news: News) -> Result<Option<Action>> {
        match news {
            News::SpotifySignedIn(result) => {
                self.spotify_login = None;
                match result {
                    Ok(user) => {
                        self.spotify = Spotify::load().map(|s| Arc::new(Mutex::new(s)));
                        self.status = format!("Connected to Spotify as {user}");
                        if let Some(Overlay::Spotify(step)) = &mut self.overlay {
                            let ready = soloist::installed() && soloist::paired();
                            *step = if ready {
                                SpotifyStep::Done(user)
                            } else {
                                SpotifyStep::start(true)
                            };
                        }
                    }
                    Err(e) => {
                        if let Some(Overlay::Spotify(step)) = &mut self.overlay {
                            *step = SpotifyStep::ClientId {
                                input: String::new(),
                                error: Some(e),
                            };
                        }
                    }
                }
            }
            News::Soloist(setup) => {
                if let Some(Overlay::Spotify(step)) = &mut self.overlay {
                    *step = match setup {
                        soloist::Setup::Progress(msg) => SpotifyStep::SettingUp(msg),
                        soloist::Setup::Done(Ok(())) => {
                            SpotifyStep::Done("Spotify plays on this computer now".into())
                        }
                        soloist::Setup::Done(Err(e)) => SpotifyStep::Playback {
                            input: soloist::key().unwrap_or_default(),
                            error: Some(e),
                        },
                    };
                }
            }
            News::YouTubeSignedIn(result) => match result {
                Ok(user) => {
                    self.youtube_account = youtube::account();
                    if let Ok(mut c) = self.youtube.lock() {
                        *c = None;
                    }
                    self.status = format!("Connected to YouTube Music as {user}");
                    if let Some(Overlay::YouTube(step)) = &mut self.overlay {
                        *step = YouTubeStep::Done(user);
                    }
                }
                Err(e) => {
                    if let Some(Overlay::YouTube(step)) = &mut self.overlay {
                        *step = YouTubeStep::Browser {
                            list: youtube::logins(),
                            sel: 0,
                            error: Some(e),
                        };
                    }
                }
            },
            News::Found {
                generation,
                source,
                result,
            } => {
                if generation == self.search.generation {
                    self.search.found[search::index(source)] = match result {
                        Ok((songs, total)) => Found::Songs(songs, total),
                        Err(e) => Found::Failed(e),
                    };
                    self.search.settle();
                }
            }
            News::Lists { source, result } => {
                if let Some(Overlay::Open(open)) = &mut self.overlay {
                    let listing = match result {
                        Ok(list) => Listing::Ready(list),
                        Err(e) => Listing::Failed(e),
                    };
                    match source {
                        Source::Spotify => open.spotify = listing,
                        _ => open.youtube = listing,
                    }
                }
            }
            News::Cover { key, image } => {
                if Some(&key) == self.cover_wanted.as_ref()
                    && let Some(img) = &image
                {
                    self.cover = self
                        .picker
                        .as_ref()
                        .and_then(|p| crate::art::encode(p, img));
                    self.cover_file = crate::art::save_for_desktop(img);
                    self.cover_image = image;
                }
            }
            News::Sonos(result) => match result {
                Ok(household) => {
                    self.sonos_status = match household.groups.len() {
                        1 => "1 group found".into(),
                        n => format!("{n} groups found"),
                    };
                    self.sonos = Some(household);
                    return Ok(Some(Action::UseSonos));
                }
                Err(e) => self.sonos_status = e,
            },
            News::Imported {
                name,
                source,
                result,
            } => match result {
                Ok(tracks) if !tracks.is_empty() => {
                    return Ok(Some(Action::LoadPlaylist {
                        name: format!("{name} ({})", source.name()),
                        path: None,
                        tracks,
                    }));
                }
                Ok(_) => self.status = format!("{name} has no songs OMAESTRO can play"),
                Err(e) => self.status = format!("Couldn't load {name}: {e}"),
            },
        }
        Ok(None)
    }

    /// Pasted text (Ctrl+Shift+V) goes into whatever text box is showing.
    pub fn on_paste(&mut self, text: &str) -> Result<()> {
        let text = text.replace(['\n', '\r'], " ");
        match &mut self.overlay {
            Some(Overlay::Spotify(step)) => step.paste(&text),
            Some(Overlay::SaveAs(name)) => name.push_str(text.trim()),
            Some(Overlay::Settings(s)) => {
                if let Some(folder) = &mut s.adding {
                    folder.push_str(text.trim());
                }
            }
            Some(Overlay::Search) => {
                self.search.typing = true;
                self.search.query.push_str(&text);
                self.run_search()?;
            }
            None if self.browse_open => {
                self.browse.pane = Pane::Search;
                self.browse.query.push_str(&text);
                self.browse.refresh(&self.catalog, &self.db)?;
            }
            _ => {}
        }
        Ok(())
    }

    // ---- keys --------------------------------------------------------------------------

    /// Handles one key press. Returns what the main loop should do, if anything.
    pub fn on_key(&mut self, key: KeyEvent, ctx: &Ctx) -> Result<Option<Action>> {
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('c') => return Ok(Some(Action::Quit)),
                // Undo works everywhere, even while typing in a search box.
                KeyCode::Char('z') => return Ok(Some(Action::Undo)),
                _ => {}
            }
        }
        if self.overlay.is_some() {
            return self.overlay_key(key, ctx);
        }
        if self.browse_open {
            match self.browse.on_key(key, &self.catalog, &self.db)? {
                Step::Stay => return Ok(None),
                Step::Close => {
                    self.close_browse();
                    return Ok(None);
                }
                Step::Do(action) => return Ok(Some(action)),
                Step::Ignored => {}
            }
        }

        // Keys that work everywhere else.
        let action = match key.code {
            KeyCode::Char('q') => Some(Action::Quit),
            KeyCode::Char(' ') => Some(Action::TogglePause),
            KeyCode::Char('x') => Some(Action::Stop),
            KeyCode::Char('n') => Some(Action::Next),
            KeyCode::Char('p') => Some(Action::Previous),
            KeyCode::Char('.') => Some(Action::Seek(10.0)),
            KeyCode::Char(',') => Some(Action::Seek(-10.0)),
            KeyCode::Char('-') => Some(Action::Volume(-5.0)),
            KeyCode::Char('+' | '=') => Some(Action::Volume(5.0)),
            KeyCode::Char('r') if !self.scanning => Some(Action::Rescan),
            KeyCode::Char('m') => Some(Action::Minimize),
            _ => None,
        };
        if action.is_some() {
            return Ok(action);
        }
        match key.code {
            KeyCode::Char('b') => self.open_browse(),
            KeyCode::Char('/') => self.open_search(),
            KeyCode::Char('o') => self.open_picker(ctx),
            KeyCode::Char('?') => self.overlay = Some(Overlay::Help),
            KeyCode::F(2) => self.open_settings(),
            _ if self.browse_open => {}
            _ => return Ok(self.queue_key(key, ctx)),
        }
        Ok(None)
    }

    fn queue_key(&mut self, key: KeyEvent, ctx: &Ctx) -> Option<Action> {
        if let Some(delta) = movement(key.code) {
            self.queue_sel = step(self.queue_sel, ctx.queue.len(), delta);
            return None;
        }
        let here = (self.queue_sel < ctx.queue.len()).then_some(self.queue_sel);
        match key.code {
            KeyCode::Enter => here.map(Action::Jump),
            KeyCode::Char('d') | KeyCode::Delete => here.map(Action::Remove),
            KeyCode::Char('K') => here.map(|i| Action::Move(i, true)),
            KeyCode::Char('J') => here.map(|i| Action::Move(i, false)),
            KeyCode::Char('u') => Some(Action::Undo),
            KeyCode::Char('c') => Some(Action::Clear),
            KeyCode::Char('s') => self.save(ctx),
            KeyCode::Char('S') => {
                self.save_as(ctx);
                None
            }
            KeyCode::Char('C') => {
                self.overlay = Some(Overlay::Columns(0));
                None
            }
            KeyCode::Char('O') => {
                self.open_playlists();
                None
            }
            _ => None,
        }
    }

    fn overlay_key(&mut self, key: KeyEvent, ctx: &Ctx) -> Result<Option<Action>> {
        let Some(overlay) = self.overlay.as_mut() else {
            return Ok(None);
        };
        match overlay {
            // Any key closes the help.
            Overlay::Help => self.overlay = None,
            Overlay::Columns(sel) => {
                if let Some(delta) = movement(key.code) {
                    *sel = step(*sel, crate::columns::Column::ALL.len(), delta);
                    return Ok(None);
                }
                let column = crate::columns::Column::ALL[*sel];
                match key.code {
                    KeyCode::Esc | KeyCode::Char('C' | 'q') => self.overlay = None,
                    KeyCode::Enter | KeyCode::Char(' ') => {
                        self.config.queue.toggle(column);
                        return Ok(Some(Action::Settings { rescan: false }));
                    }
                    _ => {}
                }
            }
            Overlay::Picker { sel, all } => {
                let (here, show_all) = (*sel, *all);
                let rows = self.picker_rows(ctx, show_all);
                if let Some(delta) = movement(key.code) {
                    if let Some(Overlay::Picker { sel, .. }) = &mut self.overlay {
                        *sel = step(here, rows.len(), delta);
                    }
                    return Ok(None);
                }
                let row = rows.get(here);
                match key.code {
                    KeyCode::Esc | KeyCode::Char('o' | 'q') => self.overlay = None,
                    KeyCode::Enter => {
                        self.overlay = None;
                        return Ok(row.map(|r| Action::SetOutput(r.choice.clone())));
                    }
                    KeyCode::Char('f') => {
                        if let Some(id) = row.map(|r| r.choice.id().to_string()) {
                            return Ok(self.mark_output(&id, true));
                        }
                    }
                    KeyCode::Char('h') => {
                        if let Some(id) = row.map(|r| r.choice.id().to_string()) {
                            return Ok(self.mark_output(&id, false));
                        }
                    }
                    KeyCode::Char('H') => {
                        if let Some(Overlay::Picker { all, .. }) = &mut self.overlay {
                            *all = !*all;
                        }
                    }
                    _ => {}
                }
            }
            Overlay::SaveAs(name) => match key.code {
                KeyCode::Esc => self.overlay = None,
                KeyCode::Enter if !name.trim().is_empty() => {
                    let name = name.trim().to_string();
                    self.overlay = None;
                    return Ok(Some(Action::SavePlaylist(name)));
                }
                _ => {
                    connect::edit(name, key);
                }
            },
            Overlay::Open(open) => {
                if let Some(delta) = movement(key.code) {
                    open.sel = step(open.sel, open.len(), delta);
                    return Ok(None);
                }
                match key.code {
                    KeyCode::Esc | KeyCode::Char('q') => self.overlay = None,
                    KeyCode::Tab | KeyCode::Right | KeyCode::Char('l') => {
                        open.tab = next_source(open.tab, 1);
                        open.sel = 0;
                    }
                    KeyCode::BackTab | KeyCode::Left | KeyCode::Char('h') => {
                        open.tab = next_source(open.tab, -1);
                        open.sel = 0;
                    }
                    KeyCode::Enter => return Ok(self.open_chosen()),
                    _ => {}
                }
            }
            Overlay::Search => match self.search.on_key(key) {
                search::Outcome::Stay => {}
                search::Outcome::Close => self.overlay = None,
                search::Outcome::Changed => self.run_search()?,
                search::Outcome::Do(action) => return Ok(Some(action)),
            },
            Overlay::Settings(_) => {
                let rows = settings::rows(&self.config, &self.facts());
                let before = self.config.library_paths.clone();
                let Some(Overlay::Settings(s)) = &mut self.overlay else {
                    return Ok(None);
                };
                let outcome = s.on_key(key, &mut self.config, &rows);
                return Ok(self.settings_outcome(outcome, before));
            }
            Overlay::Spotify(step) => {
                let outcome = step.on_key(key);
                self.connect_outcome(outcome);
            }
            Overlay::YouTube(step) => {
                let outcome = step.on_key(key);
                self.connect_outcome(outcome);
            }
        }
        Ok(None)
    }

    // ---- mouse -------------------------------------------------------------------------

    /// Handles the mouse. Buttons act when the mouse is released over the one it went down
    /// on, as in a desktop app; sliders follow a drag.
    pub fn on_mouse(&mut self, m: MouseEvent, hits: &[Hit], ctx: &Ctx) -> Result<Option<Action>> {
        let now = Instant::now();
        let at = hit_at(hits, m.column, m.row);
        let target = at.map(|h| h.target);
        match m.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                if self.ignore_clicks_until.is_some_and(|until| now < until) {
                    self.pressed = None;
                    return Ok(None);
                }
                // A click outside a popup closes it (the help closes on any click).
                if let Some(overlay) = &self.overlay {
                    let inside = target.is_some_and(Target::in_popup);
                    if !inside || matches!(overlay, Overlay::Help) {
                        match overlay {
                            Overlay::Spotify(_) | Overlay::YouTube(_) => self.close_connect(),
                            _ => self.overlay = None,
                        }
                        self.pressed = None;
                        return Ok(None);
                    }
                }
                self.pressed = target;
                // The volume follows the mouse from the first click.
                if let Some(Hit {
                    area,
                    target: Target::Volume,
                }) = at
                {
                    return Ok(Some(Action::SetVolume(fraction(area, m.column) * 100.0)));
                }
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                match (self.pressed, &mut self.drag) {
                    // Dragging a column's edge resizes it as the mouse moves.
                    (Some(Target::ColumnEdge(column, start, max)), _) => {
                        let width = m.column.saturating_sub(start).min(max);
                        self.config.queue.set_width(column, width);
                        self.drag = Some(Drag::Column { column, start });
                        return Ok(None);
                    }
                    // Dragging a queue row: it will drop on whichever row is under the mouse.
                    (Some(Target::Queue(from)), drag) => {
                        if let Some(Target::Queue(over)) = target {
                            *drag = Some(Drag::Row { from, over });
                        }
                        return Ok(None);
                    }
                    _ => {}
                }
                if self.pressed == Some(Target::Volume)
                    && let Some(h) = hits.iter().find(|h| h.target == Target::Volume)
                {
                    return Ok(Some(Action::SetVolume(fraction(h.area, m.column) * 100.0)));
                }
            }
            MouseEventKind::Up(MouseButton::Left) => {
                match self.drag.take() {
                    Some(Drag::Row { from, over }) if from != over => {
                        self.pressed = None;
                        return Ok(Some(Action::MoveTo(from, over)));
                    }
                    Some(Drag::Column { .. }) => {
                        self.pressed = None;
                        return Ok(Some(Action::Settings { rescan: false }));
                    }
                    _ => {}
                }
                let Some(pressed) = self.pressed.take() else {
                    return Ok(None);
                };
                // The position bar seeks to wherever the mouse is let go, even past its ends.
                if pressed == Target::Seek
                    && let Some(h) = hits.iter().find(|h| h.target == Target::Seek)
                {
                    return Ok(Some(Action::SeekTo(fraction(h.area, m.column))));
                }
                if Some(pressed) != target || pressed == Target::Volume {
                    return Ok(None);
                }
                let double = self.last_click.is_some_and(|(t, when)| {
                    t == pressed && now.duration_since(when) < DOUBLE_CLICK
                });
                self.last_click = if double { None } else { Some((pressed, now)) };
                return self.click(pressed, double, ctx);
            }
            MouseEventKind::ScrollDown | MouseEventKind::ScrollUp => {
                let delta = if m.kind == MouseEventKind::ScrollDown {
                    1
                } else {
                    -1
                };
                return self.wheel(target, delta, ctx);
            }
            _ => {}
        }
        Ok(None)
    }

    fn click(&mut self, target: Target, double: bool, ctx: &Ctx) -> Result<Option<Action>> {
        Ok(match target {
            Target::Stop => Some(Action::Stop),
            Target::Previous => Some(Action::Previous),
            Target::PlayPause => Some(Action::TogglePause),
            Target::Next => Some(Action::Next),
            Target::Output => {
                self.open_picker(ctx);
                None
            }
            Target::Settings => {
                self.open_settings();
                None
            }
            Target::Help => {
                self.overlay = Some(Overlay::Help);
                None
            }
            Target::Save => self.save(ctx),
            Target::SaveAs => {
                self.save_as(ctx);
                None
            }
            Target::Open => {
                self.open_playlists();
                None
            }
            Target::Clear => Some(Action::Clear),
            Target::ColumnsMenu => {
                self.overlay = Some(Overlay::Columns(0));
                None
            }
            Target::ColumnItem(i) => {
                if let Some(Overlay::Columns(sel)) = &mut self.overlay {
                    *sel = i;
                }
                self.config.queue.toggle(crate::columns::Column::ALL[i]);
                Some(Action::Settings { rescan: false })
            }
            Target::ColumnEdge(..) => None,
            Target::Queue(i) => {
                self.queue_sel = i;
                double.then_some(Action::Jump(i))
            }
            Target::BrowseSearch => {
                self.browse.pane = Pane::Search;
                None
            }
            Target::Toggle(kind) => {
                self.browse.toggle(kind, &self.catalog, &self.db)?;
                None
            }
            Target::Result(i) => {
                self.browse.pane = Pane::Results;
                self.browse.select(i, &self.db)?;
                match double.then(|| self.browse.enter()) {
                    Some(Step::Do(action)) => Some(action),
                    _ => None,
                }
            }
            Target::Song(i) => {
                self.browse.pane = Pane::Songs;
                self.browse.track_sel = i;
                match double.then(|| self.browse.enter()) {
                    Some(Step::Do(action)) => Some(action),
                    _ => None,
                }
            }
            Target::Device(i) => {
                let all = matches!(self.overlay, Some(Overlay::Picker { all: true, .. }));
                let choice = self.picker_rows(ctx, all).get(i).map(|r| r.choice.clone());
                self.overlay = None;
                choice.map(Action::SetOutput)
            }
            Target::Playlist(i) => {
                if let Some(Overlay::Open(open)) = &mut self.overlay {
                    open.sel = i;
                }
                if double { self.open_chosen() } else { None }
            }
            Target::OpenTab(source) => {
                if let Some(Overlay::Open(open)) = &mut self.overlay {
                    open.tab = source;
                    open.sel = 0;
                }
                None
            }
            Target::SearchBox => {
                self.search.typing = true;
                None
            }
            Target::CloseSearch => {
                self.overlay = None;
                None
            }
            Target::SearchChip(source) => {
                if self.search.toggle(source) == search::Outcome::Changed {
                    self.run_search()?;
                }
                None
            }
            Target::SearchRow(i) => {
                self.search.select(i);
                double
                    .then(|| self.search.selected())
                    .flatten()
                    .map(Action::PlayNow)
            }
            Target::SettingsSection(n) => {
                let rows = settings::rows(&self.config, &self.facts());
                if let Some(Overlay::Settings(s)) = &mut self.overlay {
                    s.go_to_section(&rows, n);
                }
                None
            }
            Target::SettingsRow(i) | Target::SettingsChoice(i, _) | Target::SettingsButton(i) => {
                let rows = settings::rows(&self.config, &self.facts());
                let before = self.config.library_paths.clone();
                let Some(Overlay::Settings(s)) = &mut self.overlay else {
                    return Ok(None);
                };
                s.sel = i;
                let outcome = match target {
                    Target::SettingsChoice(_, n) => match rows.get(i) {
                        Some(row) => s.choose(&mut self.config, row, n),
                        None => settings::Outcome::Stay,
                    },
                    Target::SettingsButton(_) => s.press(&mut self.config, &rows),
                    _ => settings::Outcome::Stay,
                };
                self.settings_outcome(outcome, before)
            }
            Target::Browser(i) => {
                if let Some(Overlay::YouTube(YouTubeStep::Browser { list, sel, .. })) =
                    &mut self.overlay
                {
                    *sel = i;
                    if double && let Some(login) = list.get(i).cloned() {
                        self.connect_outcome(connect::Outcome::YouTubeBrowser(login));
                    }
                }
                None
            }
            Target::Link(link) | Target::Copy(link) => {
                let step = match &self.overlay {
                    Some(Overlay::Spotify(step)) => Some(step),
                    _ => None,
                };
                if let Some(url) = link.url(step) {
                    let copy = matches!(target, Target::Copy(_));
                    self.status = match (copy, copy && secrets::copy_to_clipboard(&url).is_ok()) {
                        (true, true) => "Copied to the clipboard".into(),
                        (true, false) => format!("Couldn't copy; the address is {url}"),
                        (false, _) => match secrets::open_in_browser(&url) {
                            Ok(()) => "Opened in your browser".into(),
                            Err(e) => format!("{e:#}"),
                        },
                    };
                }
                None
            }
            Target::Volume
            | Target::Seek
            | Target::QueueList
            | Target::Results
            | Target::Songs
            | Target::Popup => None,
        })
    }

    /// The scroll wheel moves the highlight in whatever list it's over.
    fn wheel(&mut self, target: Option<Target>, delta: isize, ctx: &Ctx) -> Result<Option<Action>> {
        match target {
            Some(Target::Queue(_) | Target::QueueList) => {
                self.queue_sel = step(self.queue_sel, ctx.queue.len(), delta);
            }
            Some(Target::Result(_) | Target::Results) => {
                self.browse.pane = Pane::Results;
                self.browse.move_results(delta, &self.db)?;
            }
            Some(Target::Song(_) | Target::Songs) => {
                self.browse.pane = Pane::Songs;
                self.browse.track_sel =
                    step(self.browse.track_sel, self.browse.tracks.len(), delta);
            }
            Some(Target::Volume) => return Ok(Some(Action::Volume(-delta as f64 * 5.0))),
            Some(Target::Device(_)) => {
                let all = matches!(self.overlay, Some(Overlay::Picker { all: true, .. }));
                let n = self.picker_rows(ctx, all).len();
                if let Some(Overlay::Picker { sel, .. }) = &mut self.overlay {
                    *sel = step(*sel, n, delta);
                }
            }
            Some(Target::Playlist(_)) => {
                if let Some(Overlay::Open(open)) = &mut self.overlay {
                    open.sel = step(open.sel, open.len(), delta);
                }
            }
            Some(Target::SearchRow(_)) => {
                self.search.typing = false;
                self.search.move_by(delta);
            }
            Some(
                Target::SettingsRow(_) | Target::SettingsChoice(..) | Target::SettingsButton(_),
            ) => {
                let rows = settings::rows(&self.config, &self.facts());
                if let Some(Overlay::Settings(s)) = &mut self.overlay {
                    let key = if delta > 0 {
                        KeyCode::Down
                    } else {
                        KeyCode::Up
                    };
                    s.on_key(KeyEvent::from(key), &mut self.config, &rows);
                }
            }
            _ => {}
        }
        Ok(None)
    }
}

/// Uses the shared YouTube Music client, making it first if needed.
fn with_youtube<T>(
    client: &Mutex<Option<youtube::Client>>,
    f: impl FnOnce(&mut youtube::Client) -> Result<T>,
) -> Result<T, String> {
    let mut guard = client.lock().map_err(|_| "busy".to_string())?;
    if guard.is_none() {
        *guard = Some(youtube::Client::new().map_err(|e| format!("{e:#}"))?);
    }
    let c = guard.as_mut().ok_or("no client")?;
    f(c).map_err(|e| format!("{e:#}"))
}

/// Local → Spotify → YouTube Music, and around.
fn next_source(source: Source, delta: i32) -> Source {
    let i = search::index(source) as i32;
    search::SOURCES[(i + delta).rem_euclid(3) as usize]
}

/// "Schiit Modi+ Analog Stereo" → "Schiit Modi+".
pub fn short_device(description: &str) -> String {
    [" Analog Stereo", " Digital Stereo", " Stereo"]
        .iter()
        .find_map(|suffix| description.find(suffix).map(|i| &description[..i]))
        .unwrap_or(description)
        .to_string()
}

/// 117707 → "117,707".
pub fn group_thousands(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::testutil::{TempDir, sample_library};
    use ratatui::crossterm::event::{KeyEvent, KeyModifiers};
    use std::sync::mpsc;

    /// An app on the sample library, with nothing signed in and playlists kept in `tmp`.
    pub fn app(tmp: &TempDir) -> App {
        let (tx, _rx) = mpsc::channel();
        let mut app = App::new(sample_library(&tmp.0), Config::default(), tx).unwrap();
        app.playlist_dir = tmp.0.join("playlists");
        app.spotify = None;
        app.youtube_account = None;
        app
    }

    fn press(app: &mut App, code: KeyCode, ctx: &Ctx) -> Option<Action> {
        app.on_key(KeyEvent::from(code), ctx).unwrap()
    }

    fn mouse(
        app: &mut App,
        kind: MouseEventKind,
        x: u16,
        y: u16,
        hits: &[Hit],
        ctx: &Ctx,
    ) -> Option<Action> {
        let m = MouseEvent {
            kind,
            column: x,
            row: y,
            modifiers: KeyModifiers::NONE,
        };
        app.on_mouse(m, hits, ctx).unwrap()
    }

    fn click(app: &mut App, x: u16, y: u16, hits: &[Hit], ctx: &Ctx) -> Option<Action> {
        mouse(
            app,
            MouseEventKind::Down(MouseButton::Left),
            x,
            y,
            hits,
            ctx,
        );
        mouse(app, MouseEventKind::Up(MouseButton::Left), x, y, hits, ctx)
    }

    const NO_PLAYER: Ctx = Ctx {
        queue: &[],
        devices: &[],
        output: &crate::player::Output::Here(String::new()),
        playlist: None,
    };

    #[test]
    fn b_slides_the_browser_up_and_typing_goes_to_its_search_box() {
        let tmp = TempDir::new("app-browse");
        let mut app = app(&tmp);
        let start = Instant::now();
        assert_eq!(app.browse_fraction(start), 0.0);

        press(&mut app, KeyCode::Char('b'), &NO_PLAYER);
        assert!(app.browse_open);
        let halfway = app.browse_fraction(Instant::now() + SLIDE / 2);
        assert!(halfway > 0.0 && halfway < 1.0, "{halfway}");
        assert_eq!(app.browse_fraction(Instant::now() + SLIDE * 2), 1.0);
        // It asks for frames while it moves, and one more once it's all the way up.
        assert!(app.animating(Instant::now()));
        assert!(app.animating(Instant::now() + SLIDE * 2));
        assert!(!app.animating(Instant::now() + SLIDE * 2));
        assert_eq!(app.browse_fraction(Instant::now()), 1.0);

        // Letters, including the ones that are keys elsewhere, are typed.
        for c in "zap q".chars() {
            assert_eq!(press(&mut app, KeyCode::Char(c), &NO_PLAYER), None);
        }
        assert_eq!(app.browse.query, "zap q");
        // Esc leaves the search box; then the player keys work again; b closes.
        press(&mut app, KeyCode::Esc, &NO_PLAYER);
        assert_eq!(
            press(&mut app, KeyCode::Char(' '), &NO_PLAYER),
            Some(Action::TogglePause)
        );
        press(&mut app, KeyCode::Char('b'), &NO_PLAYER);
        assert!(!app.browse_open);
        // Reopening keeps the search.
        press(&mut app, KeyCode::Char('b'), &NO_PLAYER);
        assert_eq!(app.browse.query, "zap q");
    }

    #[test]
    fn queue_keys_edit_and_picker_sets_the_device() {
        let tmp = TempDir::new("app-queue");
        let mut app = app(&tmp);
        let queue = app.browse.tracks.clone();
        let devices = vec![
            AudioDevice {
                name: "pipewire".into(),
                description: "Default".into(),
            },
            AudioDevice {
                name: "pipewire/modi".into(),
                description: "Schiit Modi+".into(),
            },
        ];
        let ctx = Ctx {
            queue: &queue,
            devices: &devices,
            output: &crate::player::Output::Here(String::new()),
            playlist: None,
        };

        press(&mut app, KeyCode::Char('j'), &ctx);
        assert_eq!(press(&mut app, KeyCode::Enter, &ctx), Some(Action::Jump(1)));
        assert_eq!(
            press(&mut app, KeyCode::Char('d'), &ctx),
            Some(Action::Remove(1))
        );
        assert_eq!(
            press(&mut app, KeyCode::Char('K'), &ctx),
            Some(Action::Move(1, true))
        );
        assert_eq!(
            press(&mut app, KeyCode::Char('u'), &ctx),
            Some(Action::Undo)
        );
        assert_eq!(
            press(&mut app, KeyCode::Char('c'), &ctx),
            Some(Action::Clear)
        );
        assert_eq!(
            press(&mut app, KeyCode::Char('-'), &ctx),
            Some(Action::Volume(-5.0))
        );
        assert_eq!(
            press(&mut app, KeyCode::Char('x'), &ctx),
            Some(Action::Stop)
        );

        press(&mut app, KeyCode::Char('o'), &ctx);
        press(&mut app, KeyCode::Char('j'), &ctx);
        assert_eq!(
            press(&mut app, KeyCode::Enter, &ctx),
            Some(Action::SetOutput(OutputChoice::Here(
                "pipewire/modi".into()
            )))
        );
        assert!(app.overlay.is_none());

        press(&mut app, KeyCode::Char('?'), &ctx);
        assert!(matches!(app.overlay, Some(Overlay::Help)));
        assert_eq!(
            press(&mut app, KeyCode::Char('q'), &ctx),
            None,
            "closes help"
        );
        assert_eq!(
            press(&mut app, KeyCode::Char('q'), &ctx),
            Some(Action::Quit)
        );
    }

    #[test]
    fn s_asks_for_a_name_then_saves_and_capital_o_opens() {
        let tmp = TempDir::new("app-save");
        let mut app = app(&tmp);
        let queue = app.browse.tracks.clone();
        let ctx = Ctx {
            queue: &queue,
            devices: &[],
            output: &crate::player::Output::Here(String::new()),
            playlist: None,
        };
        assert_eq!(press(&mut app, KeyCode::Char('s'), &ctx), None);
        assert!(matches!(&app.overlay, Some(Overlay::SaveAs(n)) if n.is_empty()));
        for c in "Mix".chars() {
            press(&mut app, KeyCode::Char(c), &ctx);
        }
        assert_eq!(
            press(&mut app, KeyCode::Enter, &ctx),
            Some(Action::SavePlaylist("Mix".into()))
        );

        // What the main loop does with it, then Open lists and loads it.
        let path = playlist::file_for(&app.playlist_dir, "Mix");
        playlist::save(&path, "Mix", &queue).unwrap();
        press(&mut app, KeyCode::Char('O'), &ctx);
        assert!(matches!(&app.overlay, Some(Overlay::Open(o)) if o.local.len() == 1));
        assert!(
            matches!(&app.overlay, Some(Overlay::Open(o)) if o.spotify == Listing::NotConnected)
        );
        match press(&mut app, KeyCode::Enter, &ctx) {
            Some(Action::LoadPlaylist { name, tracks, path }) => {
                assert_eq!(name, "Mix");
                assert_eq!(tracks, queue);
                assert_eq!(path.as_deref(), Some(path_ref(&app, "Mix").as_path()));
            }
            other => panic!("expected LoadPlaylist, got {other:?}"),
        }

        // Once the queue has a name, s saves straight away.
        let named = Named {
            name: "Mix".into(),
            path,
            saved: 0,
        };
        let ctx = Ctx {
            playlist: Some(&named),
            ..ctx
        };
        assert_eq!(
            press(&mut app, KeyCode::Char('s'), &ctx),
            Some(Action::SavePlaylist("Mix".into()))
        );
        // Ctrl+Z undoes from anywhere, even with a text box open.
        app.overlay = Some(Overlay::SaveAs("half typed".into()));
        let ctrl_z = KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL);
        assert_eq!(app.on_key(ctrl_z, &ctx).unwrap(), Some(Action::Undo));
    }

    fn path_ref(app: &App, name: &str) -> PathBuf {
        playlist::file_for(&app.playlist_dir, name)
    }

    #[test]
    fn search_screen_finds_local_songs_and_says_what_isnt_connected() {
        let tmp = TempDir::new("app-search");
        let mut app = app(&tmp);
        app.search.show = [true, true, false];
        press(&mut app, KeyCode::Char('/'), &NO_PLAYER);
        assert!(matches!(app.overlay, Some(Overlay::Search)));
        for c in "know".chars() {
            press(&mut app, KeyCode::Char(c), &NO_PLAYER);
        }
        match &app.search.found[0] {
            Found::Songs(songs, 1) => assert_eq!(songs[0].title, "Knowing Me"),
            other => panic!("{other:?}"),
        }
        assert!(matches!(app.search.found[1], Found::Unavailable(_)));
        press(&mut app, KeyCode::Enter, &NO_PLAYER);
        match press(&mut app, KeyCode::Enter, &NO_PLAYER) {
            Some(Action::PlayNow(track)) => assert_eq!(track.title, "Knowing Me"),
            other => panic!("{other:?}"),
        }
        // Late answers to an older search are ignored.
        app.on_news(News::Found {
            generation: 1,
            source: Source::Spotify,
            result: Ok((vec![], 0)),
        })
        .unwrap();
        assert!(matches!(app.search.found[1], Found::Unavailable(_)));
    }

    #[test]
    fn settings_saves_changes_and_opens_the_sign_in_screens() {
        let tmp = TempDir::new("app-settings");
        let mut app = app(&tmp);
        app.open_settings();
        press(&mut app, KeyCode::Tab, &NO_PLAYER);
        assert_eq!(
            press(&mut app, KeyCode::Right, &NO_PLAYER),
            Some(Action::Settings { rescan: false })
        );
        assert_eq!(
            app.config.playback.replaygain, "off",
            "Album → Off, wrapping round"
        );

        // Sources → Spotify → Connect opens its sign-in; Esc goes back to Settings.
        for _ in 0..2 {
            press(&mut app, KeyCode::Tab, &NO_PLAYER);
        }
        press(&mut app, KeyCode::Enter, &NO_PLAYER);
        assert!(matches!(
            &app.overlay,
            Some(Overlay::Spotify(SpotifyStep::ClientId { .. }))
        ));
        app.on_paste("not-an-id").unwrap();
        press(&mut app, KeyCode::Enter, &NO_PLAYER);
        assert!(matches!(
            &app.overlay,
            Some(Overlay::Spotify(SpotifyStep::ClientId {
                error: Some(_),
                ..
            }))
        ));
        press(&mut app, KeyCode::Esc, &NO_PLAYER);
        assert!(matches!(&app.overlay, Some(Overlay::Settings(_))));
    }

    #[test]
    fn clicks_press_buttons_on_release_and_double_clicks_play() {
        let tmp = TempDir::new("app-mouse");
        let mut app = app(&tmp);
        let queue = app.browse.tracks.clone();
        let ctx = Ctx {
            queue: &queue,
            devices: &[],
            output: &crate::player::Output::Here(String::new()),
            playlist: None,
        };
        let hits = [
            Hit {
                area: Rect::new(10, 0, 7, 3),
                target: Target::PlayPause,
            },
            Hit {
                area: Rect::new(20, 1, 15, 1),
                target: Target::Volume,
            },
            Hit {
                area: Rect::new(0, 10, 50, 1),
                target: Target::Queue(1),
            },
        ];

        assert_eq!(
            click(&mut app, 12, 1, &hits, &ctx),
            Some(Action::TogglePause)
        );
        // Pressed on the button but let go somewhere else: nothing happens.
        mouse(
            &mut app,
            MouseEventKind::Down(MouseButton::Left),
            12,
            1,
            &hits,
            &ctx,
        );
        assert_eq!(
            mouse(
                &mut app,
                MouseEventKind::Up(MouseButton::Left),
                40,
                5,
                &hits,
                &ctx
            ),
            None
        );

        // The slider follows the mouse: its middle is 50%, a drag past the end is 100%.
        assert_eq!(
            mouse(
                &mut app,
                MouseEventKind::Down(MouseButton::Left),
                27,
                1,
                &hits,
                &ctx
            ),
            Some(Action::SetVolume(50.0))
        );
        assert_eq!(
            mouse(
                &mut app,
                MouseEventKind::Drag(MouseButton::Left),
                60,
                1,
                &hits,
                &ctx
            ),
            Some(Action::SetVolume(100.0))
        );
        mouse(
            &mut app,
            MouseEventKind::Up(MouseButton::Left),
            60,
            1,
            &hits,
            &ctx,
        );

        // One click selects a row, a second one plays it.
        assert_eq!(click(&mut app, 5, 10, &hits, &ctx), None);
        assert_eq!(app.queue_sel, 1);
        assert_eq!(click(&mut app, 5, 10, &hits, &ctx), Some(Action::Jump(1)));

        // Right after the window gains focus, clicks are ignored.
        app.on_focus();
        assert_eq!(click(&mut app, 12, 1, &hits, &ctx), None);
    }

    #[test]
    fn dragging_rows_reorders_and_dragging_edges_resizes() {
        let tmp = TempDir::new("app-drag");
        let mut app = app(&tmp);
        let queue = app.browse.tracks.clone();
        let ctx = Ctx {
            queue: &queue,
            devices: &[],
            output: &crate::player::Output::Here(String::new()),
            playlist: None,
        };
        let row = |i: u16| Hit {
            area: Rect::new(0, 10 + i, 80, 1),
            target: Target::Queue(i as usize),
        };
        let mut hits = vec![row(0), row(1), row(2)];
        hits.push(Hit {
            area: Rect::new(40, 8, 2, 1),
            target: Target::ColumnEdge(crate::columns::Column::Title, 10, 45),
        });

        // Down on the first row, drag to the third, let go: it moves there.
        mouse(
            &mut app,
            MouseEventKind::Down(MouseButton::Left),
            5,
            10,
            &hits,
            &ctx,
        );
        mouse(
            &mut app,
            MouseEventKind::Drag(MouseButton::Left),
            5,
            11,
            &hits,
            &ctx,
        );
        mouse(
            &mut app,
            MouseEventKind::Drag(MouseButton::Left),
            5,
            12,
            &hits,
            &ctx,
        );
        assert_eq!(app.drag, Some(Drag::Row { from: 0, over: 2 }));
        assert_eq!(
            mouse(
                &mut app,
                MouseEventKind::Up(MouseButton::Left),
                5,
                12,
                &hits,
                &ctx
            ),
            Some(Action::MoveTo(0, 2))
        );
        // A click without moving is still a click.
        assert_eq!(click(&mut app, 5, 11, &hits, &ctx), None);
        assert_eq!(app.queue_sel, 1);

        // The title column's edge, dragged ten columns right, makes it 40 wide and saves;
        // it stops at the widest it can go.
        mouse(
            &mut app,
            MouseEventKind::Down(MouseButton::Left),
            40,
            8,
            &hits,
            &ctx,
        );
        mouse(
            &mut app,
            MouseEventKind::Drag(MouseButton::Left),
            50,
            8,
            &hits,
            &ctx,
        );
        assert_eq!(app.config.queue.title, 40);
        mouse(
            &mut app,
            MouseEventKind::Drag(MouseButton::Left),
            90,
            8,
            &hits,
            &ctx,
        );
        assert_eq!(app.config.queue.title, 45);
        assert_eq!(
            mouse(
                &mut app,
                MouseEventKind::Up(MouseButton::Left),
                50,
                8,
                &hits,
                &ctx
            ),
            Some(Action::Settings { rescan: false })
        );

        // A capital C typed into a search box is just a letter.
        press(&mut app, KeyCode::Char('b'), &ctx);
        for c in "Cars".chars() {
            press(&mut app, KeyCode::Char(c), &ctx);
        }
        assert!(app.overlay.is_none());
        assert_eq!(app.browse.query, "Cars");
        app.browse_open = false;
        press(&mut app, KeyCode::Char('/'), &ctx);
        for c in "Cars".chars() {
            press(&mut app, KeyCode::Char(c), &ctx);
        }
        assert!(matches!(app.overlay, Some(Overlay::Search)));
        assert_eq!(app.search.query, "Cars");
        app.overlay = None;

        // C opens the columns menu; Enter hides the highlighted one (Number).
        press(&mut app, KeyCode::Char('C'), &ctx);
        assert!(matches!(app.overlay, Some(Overlay::Columns(0))));
        assert_eq!(
            press(&mut app, KeyCode::Enter, &ctx),
            Some(Action::Settings { rescan: false })
        );
        assert!(!app.config.queue.shown(crate::columns::Column::Number));
    }

    #[test]
    fn the_cross_closes_search() {
        let tmp = TempDir::new("app-close-search");
        let mut app = app(&tmp);
        let ctx = Ctx {
            queue: &[],
            devices: &[],
            output: &crate::player::Output::Here(String::new()),
            playlist: None,
        };
        press(&mut app, KeyCode::Char('/'), &ctx);
        assert!(matches!(app.overlay, Some(Overlay::Search)));
        let hits = [Hit {
            area: Rect::new(100, 10, 3, 1),
            target: Target::CloseSearch,
        }];
        click(&mut app, 101, 10, &hits, &ctx);
        assert!(app.overlay.is_none());
    }

    #[test]
    fn groups_thousands() {
        assert_eq!(group_thousands(117707), "117,707");
        assert_eq!(group_thousands(999), "999");
        assert_eq!(group_thousands(1000), "1,000");
    }
}
