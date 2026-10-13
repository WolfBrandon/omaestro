//! The library browser that slides up over the queue (`b`): a search box, Artist | Album | Song
//! toggles, results grouped by kind on the left, and the songs of the selected result on the
//! right.
//!
//! Artists and albums are few enough (about 1,200 and 8,500 here) to filter in memory on every
//! key press. Songs come from the index's full-text search.

use crate::app::Action;
use crate::library::{Album, Db, Track};
use anyhow::Result;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::cell::Cell;

/// While more than one kind is shown, each group lists only its best matches; the toggles
/// show a whole group.
const GROUPED_CAP: usize = 5;
const GROUPED_SONGS: usize = 50;
const ALL_SONGS: usize = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Artist,
    Album,
    Song,
}

impl Kind {
    pub const ALL: [Kind; 3] = [Kind::Artist, Kind::Album, Kind::Song];

    pub fn label(self) -> &'static str {
        match self {
            Kind::Artist => "Artist",
            Kind::Album => "Album",
            Kind::Song => "Song",
        }
    }

    pub fn heading(self) -> &'static str {
        match self {
            Kind::Artist => "ARTISTS",
            Kind::Album => "ALBUMS",
            Kind::Song => "SONGS",
        }
    }
}

/// One line of the results list.
#[derive(Debug, Clone, PartialEq)]
pub enum Row {
    Heading {
        kind: Kind,
        shown: usize,
        total: usize,
    },
    Note(&'static str),
    Gap,
    Artist(usize),
    Album(usize),
    Song(usize),
}

impl Row {
    pub fn selectable(&self) -> bool {
        matches!(self, Row::Artist(_) | Row::Album(_) | Row::Song(_))
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Pane {
    Search,
    Results,
    Songs,
}

/// What a key did to the browser.
#[derive(Debug, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum Step {
    /// Handled; nothing for the main loop to do.
    Stay,
    Close,
    Do(Action),
    /// Not a browser key: try the keys that work everywhere.
    Ignored,
}

/// Which result is selected, so it stays selected when the list is rebuilt.
#[derive(Debug, Clone, PartialEq)]
enum Picked {
    Artist(String),
    Album(String),
    Song(i64),
}

/// Every artist and album in the index, each with a lower-case, accent-free copy for matching.
#[derive(Default)]
pub struct Catalog {
    artists: Vec<(String, String)>,
    albums: Vec<(Album, String, String)>,
}

impl Catalog {
    pub fn album_count(&self) -> usize {
        self.albums.len()
    }

    pub fn load(db: &Db) -> Result<Catalog> {
        Ok(Catalog {
            artists: db
                .artists()?
                .into_iter()
                .map(|a| {
                    let key = fold(&a);
                    (a, key)
                })
                .collect(),
            albums: db
                .albums()?
                .into_iter()
                .map(|a| {
                    let title = fold(&a.title);
                    let both = format!("{title} {}", fold(&a.artist));
                    (a, title, both)
                })
                .collect(),
        })
    }
}

pub struct Browse {
    pub query: String,
    pub pane: Pane,
    /// Artist, Album, Song toggles.
    pub show: [bool; 3],
    pub artists: Vec<String>,
    pub albums: Vec<Album>,
    pub songs: Vec<Track>,
    pub rows: Vec<Row>,
    /// The selected row in `rows`; always a selectable one when there is one.
    pub sel: usize,
    /// The right-hand list: the selected artist's or album's songs.
    pub tracks: Vec<Track>,
    pub tracks_title: String,
    /// The list holds one artist's albums, so each song's album is worth showing.
    pub tracks_by_artist: bool,
    pub track_sel: usize,
    /// First row shown in each list; kept by the screen code as the selection moves.
    pub results_top: Cell<usize>,
    pub tracks_top: Cell<usize>,
    loaded: Option<Picked>,
}

impl Default for Browse {
    fn default() -> Self {
        Browse {
            query: String::new(),
            pane: Pane::Search,
            show: [true; 3],
            artists: Vec::new(),
            albums: Vec::new(),
            songs: Vec::new(),
            rows: Vec::new(),
            sel: 0,
            tracks: Vec::new(),
            tracks_title: String::new(),
            tracks_by_artist: false,
            track_sel: 0,
            results_top: Cell::new(0),
            tracks_top: Cell::new(0),
            loaded: None,
        }
    }
}

impl Browse {
    /// Rebuilds the results for the current query and toggles, keeping the same result
    /// selected if it's still there.
    pub fn refresh(&mut self, catalog: &Catalog, db: &Db) -> Result<()> {
        let picked = self.picked();
        let folded = fold(&self.query);
        let words = words(&folded);
        let kinds_on = self.show.iter().filter(|s| **s).count();
        let grouped = kinds_on > 1 && !words.is_empty();
        let cap = |n: usize| if grouped { n.min(GROUPED_CAP) } else { n };

        self.rows.clear();
        let mut gap = false;
        for kind in Kind::ALL {
            if !self.show[kind as usize] {
                continue;
            }
            if gap {
                self.rows.push(Row::Gap);
            }
            gap = true;
            let total = match kind {
                Kind::Artist => {
                    let mut hits: Vec<(u8, &String)> = catalog
                        .artists
                        .iter()
                        .filter(|(_, key)| matches(key, &words))
                        .map(|(name, key)| (rank(key, &words), name))
                        .collect();
                    hits.sort_by_key(|(r, _)| *r);
                    let total = hits.len();
                    self.artists = hits
                        .into_iter()
                        .take(cap(total))
                        .map(|(_, a)| a.clone())
                        .collect();
                    total
                }
                Kind::Album => {
                    let mut hits: Vec<(u8, &Album)> = catalog
                        .albums
                        .iter()
                        .filter(|(_, _, both)| matches(both, &words))
                        .map(|(album, title, _)| (rank(title, &words), album))
                        .collect();
                    hits.sort_by_key(|(r, _)| *r);
                    let total = hits.len();
                    self.albums = hits
                        .into_iter()
                        .take(cap(total))
                        .map(|(_, a)| a.clone())
                        .collect();
                    total
                }
                Kind::Song => {
                    if words.is_empty() {
                        self.songs.clear();
                        0
                    } else {
                        let limit = if grouped { GROUPED_SONGS } else { ALL_SONGS };
                        self.songs = db.search_songs(&self.query, limit)?;
                        db.count_songs(&self.query)?
                    }
                }
            };
            let shown = match kind {
                Kind::Artist => self.artists.len(),
                Kind::Album => self.albums.len(),
                Kind::Song => self.songs.len(),
            };
            self.rows.push(Row::Heading { kind, shown, total });
            self.rows.extend((0..shown).map(|i| match kind {
                Kind::Artist => Row::Artist(i),
                Kind::Album => Row::Album(i),
                Kind::Song => Row::Song(i),
            }));
            if shown == 0 {
                self.rows.push(Row::Note(match kind {
                    Kind::Song if words.is_empty() => "Type to search songs",
                    _ if words.is_empty() => "Nothing in the library yet",
                    _ => "No matches",
                }));
            }
        }

        self.sel = picked
            .and_then(|p| (0..self.rows.len()).find(|&i| self.picked_at(i).as_ref() == Some(&p)))
            .or_else(|| self.rows.iter().position(Row::selectable))
            .unwrap_or(0);
        self.load_tracks(db)
    }

    fn picked(&self) -> Option<Picked> {
        self.picked_at(self.sel)
    }

    fn picked_at(&self, i: usize) -> Option<Picked> {
        Some(match self.rows.get(i)? {
            Row::Artist(n) => Picked::Artist(self.artists.get(*n)?.clone()),
            Row::Album(n) => Picked::Album(self.albums.get(*n)?.dir.clone()),
            Row::Song(n) => Picked::Song(self.songs.get(*n)?.id),
            _ => return None,
        })
    }

    pub fn selected(&self) -> Option<&Row> {
        self.rows.get(self.sel).filter(|r| r.selectable())
    }

    /// Fills the right-hand list for the selected result. A song shows its album, with the
    /// song selected.
    fn load_tracks(&mut self, db: &Db) -> Result<()> {
        let picked = self.picked();
        let song = match self.selected() {
            Some(Row::Song(n)) => self.songs.get(*n).cloned(),
            _ => None,
        };
        // The album a song belongs to, or the artist or album itself.
        let source = match (&picked, &song) {
            (Some(Picked::Song(_)), Some(song)) => Some(Picked::Album(song.album_dir.clone())),
            _ => picked,
        };
        if source != self.loaded {
            self.loaded = source.clone();
            self.track_sel = 0;
            self.tracks_top.set(0);
            match &source {
                Some(Picked::Artist(name)) => {
                    self.tracks = db.tracks_by(name)?;
                    self.tracks_title = name.clone();
                    self.tracks_by_artist = true;
                }
                Some(Picked::Album(dir)) => {
                    self.tracks = db.tracks_in(dir)?;
                    self.tracks_title = self
                        .tracks
                        .first()
                        .map(|t| format!("{} · {}", t.album, t.album_artist))
                        .unwrap_or_default();
                    self.tracks_by_artist = false;
                }
                _ => {
                    self.tracks.clear();
                    self.tracks_title.clear();
                }
            }
        }
        if let Some(song) = song {
            self.track_sel = self
                .tracks
                .iter()
                .position(|t| t.id == song.id)
                .unwrap_or(0);
        }
        Ok(())
    }

    /// What Enter does in the results or song list.
    pub fn enter(&mut self) -> Step {
        if self.pane == Pane::Songs {
            return self.play_songs();
        }
        match self.selected() {
            // An artist has too many songs to just start; go and pick one.
            Some(Row::Artist(_)) => {
                self.pane = Pane::Songs;
                Step::Stay
            }
            Some(_) => self.play_songs(),
            None => Step::Stay,
        }
    }

    /// Selects a results row (a click). Headings and gaps can't be selected.
    pub fn select(&mut self, row: usize, db: &Db) -> Result<()> {
        if self.rows.get(row).is_some_and(Row::selectable) {
            self.sel = row;
            self.load_tracks(db)?;
        }
        Ok(())
    }

    /// Moves the results selection by `delta` selectable rows (headings are skipped).
    pub fn move_results(&mut self, delta: isize, db: &Db) -> Result<()> {
        let selectable: Vec<usize> = (0..self.rows.len())
            .filter(|&i| self.rows[i].selectable())
            .collect();
        let Some(here) = selectable.iter().position(|&i| i == self.sel) else {
            return Ok(());
        };
        let new = step(here, selectable.len(), delta);
        self.sel = selectable[new];
        self.load_tracks(db)
    }

    fn first_selected(&self) -> bool {
        self.rows.iter().position(Row::selectable) == Some(self.sel)
    }

    /// The tracks Enter or `a` act on in the results list.
    fn tracks_for_selected(&self) -> Vec<Track> {
        match self.selected() {
            Some(Row::Song(n)) => self.songs.get(*n).cloned().into_iter().collect(),
            Some(_) => self.tracks.clone(),
            None => Vec::new(),
        }
    }

    /// Turns a kind on or off. The last one stays on, so there's always something to show.
    pub fn toggle(&mut self, kind: Kind, catalog: &Catalog, db: &Db) -> Result<()> {
        let i = kind as usize;
        if self.show[i] && self.show.iter().filter(|s| **s).count() == 1 {
            return Ok(());
        }
        self.show[i] = !self.show[i];
        self.refresh(catalog, db)
    }

    pub fn on_key(&mut self, key: KeyEvent, catalog: &Catalog, db: &Db) -> Result<Step> {
        if self.pane == Pane::Search {
            return self.search_key(key, catalog, db);
        }
        if let Some(delta) = movement(key.code) {
            match self.pane {
                Pane::Results if delta < 0 && key.code == KeyCode::Up && self.first_selected() => {
                    self.pane = Pane::Search
                }
                Pane::Results => self.move_results(delta, db)?,
                _ => self.track_sel = step(self.track_sel, self.tracks.len(), delta),
            }
            return Ok(Step::Stay);
        }
        let on_songs = self.pane == Pane::Songs;
        Ok(match key.code {
            KeyCode::Esc | KeyCode::Char('b') => Step::Close,
            KeyCode::Char('/') => {
                self.pane = Pane::Search;
                Step::Stay
            }
            KeyCode::Tab => {
                self.pane = if on_songs { Pane::Search } else { Pane::Songs };
                Step::Stay
            }
            KeyCode::BackTab => {
                self.pane = if on_songs {
                    Pane::Results
                } else {
                    Pane::Search
                };
                Step::Stay
            }
            KeyCode::Right | KeyCode::Char('l') if !on_songs => {
                self.pane = Pane::Songs;
                Step::Stay
            }
            KeyCode::Left | KeyCode::Char('h') if on_songs => {
                self.pane = Pane::Results;
                Step::Stay
            }
            KeyCode::Char(c @ '1'..='3') => {
                let kind = Kind::ALL[c as usize - '1' as usize];
                self.toggle(kind, catalog, db)?;
                Step::Stay
            }
            KeyCode::Enter => self.enter(),
            KeyCode::Char('a') => {
                let tracks = if on_songs {
                    self.tracks
                        .get(self.track_sel)
                        .cloned()
                        .into_iter()
                        .collect()
                } else {
                    self.tracks_for_selected()
                };
                if tracks.is_empty() {
                    Step::Stay
                } else {
                    Step::Do(Action::Enqueue(tracks))
                }
            }
            _ => Step::Ignored,
        })
    }

    /// Plays the right-hand list from its selected song: the album (or artist) carries on
    /// afterwards.
    fn play_songs(&self) -> Step {
        if self.tracks.is_empty() {
            Step::Stay
        } else {
            Step::Do(Action::Play(self.tracks.clone(), self.track_sel))
        }
    }

    fn search_key(&mut self, key: KeyEvent, catalog: &Catalog, db: &Db) -> Result<Step> {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc if self.query.is_empty() => return Ok(Step::Close),
            KeyCode::Esc | KeyCode::Enter | KeyCode::Down | KeyCode::Tab => {
                self.pane = Pane::Results
            }
            KeyCode::BackTab => self.pane = Pane::Songs,
            KeyCode::Backspace => {
                self.query.pop();
                self.refresh(catalog, db)?;
            }
            KeyCode::Char('u') if ctrl => {
                self.query.clear();
                self.refresh(catalog, db)?;
            }
            KeyCode::Char(c) if !ctrl => {
                self.query.push(c);
                self.refresh(catalog, db)?;
            }
            _ => {}
        }
        Ok(Step::Stay)
    }
}

/// How far a key moves a list selection, if it's a movement key.
pub fn movement(code: KeyCode) -> Option<isize> {
    match code {
        KeyCode::Down | KeyCode::Char('j') => Some(1),
        KeyCode::Up | KeyCode::Char('k') => Some(-1),
        KeyCode::PageDown => Some(20),
        KeyCode::PageUp => Some(-20),
        KeyCode::Home | KeyCode::Char('g') => Some(isize::MIN),
        KeyCode::End | KeyCode::Char('G') => Some(isize::MAX),
        _ => None,
    }
}

/// Moves a selection by `delta`, staying inside a list of `len` rows.
pub fn step(current: usize, len: usize, delta: isize) -> usize {
    if len == 0 {
        return 0;
    }
    (current as isize)
        .saturating_add(delta)
        .clamp(0, len as isize - 1) as usize
}

/// Lower case without accents, so "beyonce" finds "Beyoncé" and "motorhead" finds "Motörhead".
pub fn fold(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars().flat_map(char::to_lowercase) {
        match c {
            'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' => out.push('a'),
            'æ' => out.push_str("ae"),
            'ç' | 'č' | 'ć' => out.push('c'),
            'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ě' => out.push('e'),
            'ì' | 'í' | 'î' | 'ï' | 'ī' => out.push('i'),
            'ñ' | 'ń' => out.push('n'),
            'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' => out.push('o'),
            'œ' => out.push_str("oe"),
            'ß' => out.push_str("ss"),
            'š' | 'ś' => out.push('s'),
            'ù' | 'ú' | 'û' | 'ü' | 'ū' | 'ů' => out.push('u'),
            'ý' | 'ÿ' => out.push('y'),
            'ž' | 'ź' | 'ż' => out.push('z'),
            '’' | '‘' => out.push('\''),
            _ => out.push(c),
        }
    }
    out
}

fn words(folded: &str) -> Vec<&str> {
    folded
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect()
}

/// Every typed word starts a word of `key`: "led zep" matches "led zeppelin".
fn matches(key: &str, typed: &[&str]) -> bool {
    let have = words(key);
    typed.iter().all(|t| have.iter().any(|h| h.starts_with(t)))
}

/// 0 for an exact name, 1 for a name that starts with what was typed, 2 for the rest.
/// A leading "the" doesn't count, so "black keys" ranks The Black Keys first.
fn rank(key: &str, typed: &[&str]) -> u8 {
    if typed.is_empty() {
        return 2;
    }
    let have = words(key);
    let have = match have.split_first() {
        Some((&"the", rest)) if !rest.is_empty() && typed.first() != Some(&"the") => rest,
        _ => &have[..],
    };
    if have == typed {
        0
    } else if have.len() >= typed.len()
        && have[..typed.len() - 1] == typed[..typed.len() - 1]
        && have[typed.len() - 1].starts_with(typed[typed.len() - 1])
    {
        1
    } else {
        2
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{TempDir, sample_library};

    fn titles(tracks: &[Track]) -> Vec<&str> {
        tracks.iter().map(|t| t.title.as_str()).collect()
    }

    fn press(b: &mut Browse, c: &Catalog, db: &Db, code: KeyCode) -> Step {
        b.on_key(KeyEvent::from(code), c, db).unwrap()
    }

    fn typed(b: &mut Browse, c: &Catalog, db: &Db, text: &str) {
        for ch in text.chars() {
            assert_eq!(press(b, c, db, KeyCode::Char(ch)), Step::Stay);
        }
    }

    #[test]
    fn matching_ignores_case_accents_and_a_leading_the() {
        assert_eq!(fold("Beyoncé · Motörhead"), "beyonce · motorhead");
        let key = fold("The Black Keys");
        assert!(matches(&key, &["black", "ke"]));
        assert!(!matches(&key, &["lack"]), "words match from their start");
        assert_eq!(rank(&key, &["black", "keys"]), 0);
        assert_eq!(rank(&key, &["black", "k"]), 1);
        assert_eq!(rank(&fold("Black Flag"), &["flag"]), 2);
    }

    #[test]
    fn empty_query_lists_artists_and_albums_and_skips_headings() {
        let tmp = TempDir::new("browse-empty");
        let db = sample_library(&tmp.0);
        let catalog = Catalog::load(&db).unwrap();
        let mut b = Browse::default();
        b.refresh(&catalog, &db).unwrap();
        assert_eq!(b.artists, ["Abba", "Zappa"]);
        assert_eq!(b.albums.len(), 3);
        assert!(b.rows.contains(&Row::Note("Type to search songs")));
        assert_eq!(b.selected(), Some(&Row::Artist(0)));
        assert_eq!(titles(&b.tracks), ["Dancing Queen", "Knowing Me"]);

        // Down from the last artist skips the gap and the ALBUMS heading.
        b.pane = Pane::Results;
        press(&mut b, &catalog, &db, KeyCode::Char('j'));
        press(&mut b, &catalog, &db, KeyCode::Char('j'));
        assert_eq!(b.selected(), Some(&Row::Album(0)));
        assert_eq!(b.tracks_title, "Gold · Abba");
        // Up at the top goes back to the search box.
        press(&mut b, &catalog, &db, KeyCode::Char('g'));
        press(&mut b, &catalog, &db, KeyCode::Up);
        assert_eq!(b.pane, Pane::Search);
    }

    #[test]
    fn typing_groups_results_and_enter_plays_the_album_from_the_song() {
        let tmp = TempDir::new("browse-search");
        let db = sample_library(&tmp.0);
        let catalog = Catalog::load(&db).unwrap();
        let mut b = Browse::default();
        b.refresh(&catalog, &db).unwrap();

        // "gold" is an album: it shows as one, not as both of its songs.
        typed(&mut b, &catalog, &db, "gold");
        assert!(b.artists.is_empty());
        assert_eq!(b.albums.len(), 1);
        assert!(b.songs.is_empty());
        assert_eq!(b.selected(), Some(&Row::Album(0)));
        for _ in 0..4 {
            press(&mut b, &catalog, &db, KeyCode::Backspace);
        }

        typed(&mut b, &catalog, &db, "know");
        assert_eq!(titles(&b.songs), ["Knowing Me"]);
        assert_eq!(b.selected(), Some(&Row::Song(0)));
        // The right-hand list is the song's album, with the song selected.
        assert_eq!(titles(&b.tracks), ["Dancing Queen", "Knowing Me"]);
        assert_eq!(b.track_sel, 1);

        press(&mut b, &catalog, &db, KeyCode::Enter);
        assert_eq!(b.pane, Pane::Results);
        match press(&mut b, &catalog, &db, KeyCode::Enter) {
            Step::Do(Action::Play(tracks, 1)) => assert_eq!(tracks.len(), 2),
            other => panic!("expected Play(album, 1), got {other:?}"),
        }
        assert_eq!(
            press(&mut b, &catalog, &db, KeyCode::Char('a')),
            Step::Do(Action::Enqueue(b.songs.clone()))
        );
        assert_eq!(
            press(&mut b, &catalog, &db, KeyCode::Char('b')),
            Step::Close
        );
    }

    #[test]
    fn artists_open_their_songs_and_toggles_narrow_the_results() {
        let tmp = TempDir::new("browse-artist");
        let db = sample_library(&tmp.0);
        let catalog = Catalog::load(&db).unwrap();
        let mut b = Browse::default();
        b.refresh(&catalog, &db).unwrap();
        typed(&mut b, &catalog, &db, "zap");
        press(&mut b, &catalog, &db, KeyCode::Tab);
        assert_eq!(b.selected(), Some(&Row::Artist(0)));
        assert!(b.tracks_by_artist);
        assert_eq!(titles(&b.tracks), ["Cosmik Debris", "Peaches"]);

        // Enter on an artist moves to its songs; Enter there plays from the chosen one.
        assert_eq!(press(&mut b, &catalog, &db, KeyCode::Enter), Step::Stay);
        assert_eq!(b.pane, Pane::Songs);
        press(&mut b, &catalog, &db, KeyCode::Char('j'));
        match press(&mut b, &catalog, &db, KeyCode::Enter) {
            Step::Do(Action::Play(tracks, 1)) => assert_eq!(tracks.len(), 2),
            other => panic!("expected Play(artist, 1), got {other:?}"),
        }

        // 1 hides artists, so the album matches come first; the last toggle can't go off.
        press(&mut b, &catalog, &db, KeyCode::Char('h'));
        press(&mut b, &catalog, &db, KeyCode::Char('1'));
        assert!(!b.rows.iter().any(|r| matches!(r, Row::Artist(_))));
        assert_eq!(b.selected(), Some(&Row::Album(0)));
        press(&mut b, &catalog, &db, KeyCode::Char('3'));
        press(&mut b, &catalog, &db, KeyCode::Char('2'));
        assert_eq!(b.show, [false, true, false]);

        // A space or a letter is typed in the search box, not taken as a key.
        press(&mut b, &catalog, &db, KeyCode::Char('/'));
        typed(&mut b, &catalog, &db, " b");
        assert_eq!(b.query, "zap b");
        press(&mut b, &catalog, &db, KeyCode::Esc);
        assert_eq!(b.pane, Pane::Results, "Esc leaves the search box first");
        assert_eq!(press(&mut b, &catalog, &db, KeyCode::Esc), Step::Close);
    }
}
