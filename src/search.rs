//! The Search screen (`/`): one search box across Local, Spotify and YouTube Music, with
//! results grouped by where they come from. The library browser (`b`) is for wandering
//! through your own collection; this is for finding a song anywhere.
//!
//! Local results come straight from the index. Spotify and YouTube Music are asked on a
//! background thread a moment after you stop typing; their answers arrive tagged with the
//! search they belong to, so a slow answer to an old search is ignored.

use crate::app::Action;
use crate::browse::{movement, step};
use crate::library::{Source, Track};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::cell::Cell;

pub const SOURCES: [Source; 3] = [Source::Local, Source::Spotify, Source::YouTube];

/// What one source has found for the current search.
#[derive(Debug, Clone, PartialEq)]
pub enum Found {
    Idle,
    Searching,
    /// The songs shown and how many there are in all.
    Songs(Vec<Track>, usize),
    Failed(String),
    /// Not connected, with what to do about it.
    Unavailable(String),
}

/// One line of the results.
#[derive(Debug, Clone, PartialEq)]
pub enum Row {
    Heading(Source),
    Song(Source, usize),
    Note(Source),
    Gap,
}

pub struct Search {
    pub query: String,
    pub typing: bool,
    /// Local, Spotify, YouTube Music toggles.
    pub show: [bool; 3],
    pub found: [Found; 3],
    /// The selected row in `rows()`; a song when there is one.
    pub sel: usize,
    pub top: Cell<usize>,
    /// Goes up with every change to the query, so late answers to older searches are dropped.
    pub generation: u64,
}

/// What a key did.
#[derive(Debug, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum Outcome {
    Stay,
    Close,
    /// The words or the toggles changed: search again.
    Changed,
    Do(Action),
}

impl Default for Search {
    fn default() -> Self {
        Search {
            query: String::new(),
            typing: true,
            show: [true; 3],
            found: [Found::Idle, Found::Idle, Found::Idle],
            sel: 0,
            top: Cell::new(0),
            generation: 0,
        }
    }
}

pub fn index(source: Source) -> usize {
    match source {
        Source::Local => 0,
        Source::Spotify => 1,
        Source::YouTube => 2,
    }
}

impl Search {
    pub fn rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        for source in SOURCES {
            let i = index(source);
            if !self.show[i] {
                continue;
            }
            if !rows.is_empty() {
                rows.push(Row::Gap);
            }
            rows.push(Row::Heading(source));
            match &self.found[i] {
                Found::Songs(songs, _) if !songs.is_empty() => {
                    rows.extend((0..songs.len()).map(|n| Row::Song(source, n)));
                }
                Found::Idle => {}
                _ => rows.push(Row::Note(source)),
            }
        }
        rows
    }

    pub fn song(&self, row: &Row) -> Option<&Track> {
        match row {
            Row::Song(source, n) => match &self.found[index(*source)] {
                Found::Songs(songs, _) => songs.get(*n),
                _ => None,
            },
            _ => None,
        }
    }

    pub fn selected(&self) -> Option<Track> {
        let rows = self.rows();
        rows.get(self.sel).and_then(|r| self.song(r)).cloned()
    }

    /// After new results, keep a song selected (the first one if the old one is gone).
    pub fn settle(&mut self) {
        let rows = self.rows();
        if rows
            .get(self.sel)
            .is_some_and(|r| matches!(r, Row::Song(..)))
        {
            return;
        }
        self.sel = rows
            .iter()
            .position(|r| matches!(r, Row::Song(..)))
            .unwrap_or(0);
    }

    /// Moves the selection by `delta` songs.
    pub fn move_by(&mut self, delta: isize) {
        let songs: Vec<usize> = self
            .rows()
            .iter()
            .enumerate()
            .filter(|(_, r)| matches!(r, Row::Song(..)))
            .map(|(i, _)| i)
            .collect();
        let here = songs.iter().position(|&i| i == self.sel).unwrap_or(0);
        if let Some(&i) = songs.get(step(here, songs.len(), delta)) {
            self.sel = i;
        }
    }

    /// Selects a row (a click); only songs can be selected.
    pub fn select(&mut self, row: usize) {
        if matches!(self.rows().get(row), Some(Row::Song(..))) {
            self.sel = row;
            self.typing = false;
        }
    }

    /// Turns a source on or off; the last one stays on.
    pub fn toggle(&mut self, source: Source) -> Outcome {
        let i = index(source);
        if self.show[i] && self.show.iter().filter(|s| **s).count() == 1 {
            return Outcome::Stay;
        }
        self.show[i] = !self.show[i];
        self.settle();
        Outcome::Changed
    }

    /// The songs `A` adds: everything the selected song's source found.
    fn all_from_selected(&self) -> Vec<Track> {
        let rows = self.rows();
        match rows.get(self.sel) {
            Some(Row::Song(source, _)) => match &self.found[index(*source)] {
                Found::Songs(songs, _) => songs.clone(),
                _ => Vec::new(),
            },
            _ => Vec::new(),
        }
    }

    pub fn on_key(&mut self, key: KeyEvent) -> Outcome {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if self.typing {
            match key.code {
                KeyCode::Esc => return Outcome::Close,
                KeyCode::Enter | KeyCode::Down | KeyCode::Tab => {
                    self.typing = false;
                    self.settle();
                }
                KeyCode::Backspace => {
                    self.query.pop();
                    return Outcome::Changed;
                }
                KeyCode::Char('u') if ctrl => {
                    self.query.clear();
                    return Outcome::Changed;
                }
                KeyCode::Char(c) if !ctrl => {
                    self.query.push(c);
                    return Outcome::Changed;
                }
                _ => {}
            }
            return Outcome::Stay;
        }
        if let Some(delta) = movement(key.code) {
            if key.code == KeyCode::Up
                && self.rows().iter().position(|r| matches!(r, Row::Song(..))) == Some(self.sel)
            {
                self.typing = true;
            } else {
                self.move_by(delta);
            }
            return Outcome::Stay;
        }
        match key.code {
            KeyCode::Esc => Outcome::Close,
            KeyCode::Char('/') | KeyCode::BackTab => {
                self.typing = true;
                Outcome::Stay
            }
            KeyCode::Char(c @ '1'..='3') => self.toggle(SOURCES[c as usize - '1' as usize]),
            KeyCode::Enter => match self.selected() {
                Some(track) => Outcome::Do(Action::PlayNow(track)),
                None => Outcome::Stay,
            },
            KeyCode::Char('a') => match self.selected() {
                Some(track) => Outcome::Do(Action::Enqueue(vec![track])),
                None => Outcome::Stay,
            },
            KeyCode::Char('A') => {
                let all = self.all_from_selected();
                if all.is_empty() {
                    Outcome::Stay
                } else {
                    Outcome::Do(Action::Enqueue(all))
                }
            }
            // Tab jumps to the next source's first song.
            KeyCode::Tab => {
                let rows = self.rows();
                let here = match rows.get(self.sel) {
                    Some(Row::Song(s, _)) => index(*s),
                    _ => 0,
                };
                if let Some(i) = rows
                    .iter()
                    .position(|r| matches!(r, Row::Song(s, _) if index(*s) > here))
                    .or_else(|| rows.iter().position(|r| matches!(r, Row::Song(..))))
                {
                    self.sel = i;
                }
                Outcome::Stay
            }
            // Typing a letter goes back to the search box.
            KeyCode::Char(c) if !ctrl && c.is_alphanumeric() => {
                self.typing = true;
                self.query.push(c);
                Outcome::Changed
            }
            _ => Outcome::Stay,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn songs(source: Source, titles: &[&str]) -> Found {
        let tracks: Vec<Track> = titles
            .iter()
            .map(|t| Track {
                source,
                title: t.to_string(),
                ..Track::default()
            })
            .collect();
        let n = tracks.len();
        Found::Songs(tracks, n)
    }

    fn press(s: &mut Search, code: KeyCode) -> Outcome {
        s.on_key(KeyEvent::from(code))
    }

    #[test]
    fn groups_by_source_and_moves_between_songs_only() {
        let mut s = Search::default();
        for c in "keys".chars() {
            assert_eq!(press(&mut s, KeyCode::Char(c)), Outcome::Changed);
        }
        s.found = [
            songs(Source::Local, &["Tighten Up", "Fever"]),
            Found::Unavailable("Connect Spotify in Settings".into()),
            songs(Source::YouTube, &["Lonely Boy"]),
        ];
        press(&mut s, KeyCode::Enter);
        assert!(!s.typing);
        assert_eq!(s.selected().unwrap().title, "Tighten Up");
        press(&mut s, KeyCode::Char('j'));
        press(&mut s, KeyCode::Char('j'));
        assert_eq!(
            s.selected().unwrap().title,
            "Lonely Boy",
            "skips Spotify's note"
        );
        assert_eq!(
            press(&mut s, KeyCode::Enter),
            Outcome::Do(Action::PlayNow(s.selected().unwrap()))
        );
        press(&mut s, KeyCode::Tab);
        assert_eq!(
            s.selected().unwrap().title,
            "Tighten Up",
            "Tab wraps to the first source"
        );
        match press(&mut s, KeyCode::Char('A')) {
            Outcome::Do(Action::Enqueue(all)) => assert_eq!(all.len(), 2),
            other => panic!("{other:?}"),
        }

        // 1 hides local results; the selection moves to what's left.
        assert_eq!(press(&mut s, KeyCode::Char('1')), Outcome::Changed);
        assert_eq!(s.selected().unwrap().title, "Lonely Boy");
        assert_eq!(press(&mut s, KeyCode::Char('3')), Outcome::Changed);
        assert_eq!(s.show, [false, true, false]);
        assert_eq!(
            press(&mut s, KeyCode::Char('2')),
            Outcome::Stay,
            "one stays on"
        );

        // A letter goes back to typing; Esc there closes.
        assert_eq!(press(&mut s, KeyCode::Char('x')), Outcome::Changed);
        assert_eq!(s.query, "keysx");
        assert_eq!(press(&mut s, KeyCode::Esc), Outcome::Close);
    }
}
