//! The screens that open over the player: Search, Settings, the Spotify and YouTube Music
//! sign-ins, and the Open list. Search and Settings follow the OTIS design's tabs of the same
//! names.

use super::*;
use crate::app::{Listing, Open};
use crate::connect::{DASHBOARD, Link, SpotifyStep, YOUTUBE, YouTubeStep};
use crate::search::{Found, Row as SearchRow, Search};
use crate::settings::{self, Facts, SECTIONS, Settings, Value};
use crate::spotify::{self, soloist};

/// The big centred box Search and Settings use: 100×46 on the design's 120×60 screen.
fn big_box(screen: Rect) -> Rect {
    let w = screen.width.saturating_sub(4).min(100);
    let h = screen.height.saturating_sub(4).min(46);
    centred(screen, w, h)
}

/// Splits text into lines of at most `w` columns, at spaces.
fn wrap(text: &str, w: u16) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split(' ') {
        if !line.is_empty() && width(&line) + 1 + width(word) > w {
            lines.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

/// Writes wrapped text from row `y`; returns the row after it.
fn paragraph(buf: &mut Buffer, clip: Rect, x: u16, y: u16, text: &str, style: Style) -> u16 {
    let w = clip.right().saturating_sub(x + 1);
    let mut y = y;
    for line in wrap(text, w) {
        put(buf, clip, x, y, &line, style);
        y += 1;
    }
    y
}

/// A one-line text box with a cursor: "▏".
fn field(buf: &mut Buffer, r: Rect, text: &str, focused: bool, t: &Theme) {
    fill(buf, r, Style::new().bg(t.selection));
    // Show the end of a long entry, so what's being typed stays in view.
    let room = r.width.saturating_sub(3);
    let mut shown = text;
    while width(shown) > room {
        let mut chars = shown.chars();
        chars.next();
        shown = chars.as_str();
    }
    let x = put(buf, r, r.x + 1, r.y, shown, bold(t.bright));
    if focused {
        put(buf, r, x, r.y, "▏", fg(t.accent));
    }
}

// ---- Search --------------------------------------------------------------------------

pub(super) fn draw_search(
    buf: &mut Buffer,
    screen: Rect,
    s: &Search,
    player: &Player,
    t: &Theme,
    hit: &mut impl FnMut(Rect, Target),
) {
    let r = big_box(screen);
    let inner = popup(buf, r, "SEARCH", t, hit);
    let (x0, x1) = (inner.x + 1, inner.right().saturating_sub(2));
    let mut y = inner.y;

    // Source chips: on ones filled, off ones dim.
    let mut x = x0;
    for source in crate::search::SOURCES {
        let on = s.show[crate::search::index(source)];
        let (icon, color) = source_icon(source, t);
        let start = x;
        if on {
            x = put(
                buf,
                inner,
                x,
                y,
                &format!(" {icon}"),
                fg(color).bg(t.selection),
            );
            x = put(
                buf,
                inner,
                x,
                y,
                &format!(" {} ", source.name()),
                bold(t.bright).bg(t.selection),
            );
        } else {
            x = put(buf, inner, x, y, &format!(" {icon}"), fg(t.dim));
            let off = fg(t.dim).add_modifier(Modifier::CROSSED_OUT);
            x = put(buf, inner, x, y, &format!(" {} ", source.name()), off);
        }
        hit(
            Rect::new(start, y, x - start, 1),
            Target::SearchChip(source),
        );
        x += 2;
    }
    put_right(buf, inner, x1, y, "1 2 3 toggle", fg(t.dim));
    y += 2;

    // The search box, with a count of what's been found on the right.
    let boxr = Rect::new(x0, y, x1 + 1 - x0, 1);
    hit(boxr, Target::SearchBox);
    fill(buf, boxr, Style::new().bg(t.panel));
    let x = put(buf, boxr, x0 + 1, y, &format!("{I_SEARCH}  "), fg(t.fg));
    if s.query.is_empty() && !s.typing {
        let hint = fg(t.dim).add_modifier(Modifier::ITALIC);
        put(
            buf,
            boxr,
            x,
            y,
            "Songs on every source you've switched on",
            hint,
        );
    } else {
        let x = put(buf, boxr, x, y, &s.query, bold(t.bright));
        if s.typing {
            put(buf, boxr, x, y, "▏", fg(t.accent));
        }
    }
    let counts: Vec<String> = crate::search::SOURCES
        .iter()
        .filter_map(|src| match &s.found[crate::search::index(*src)] {
            Found::Songs(_, total) => Some(format!(
                "{} {}",
                group_thousands(*total),
                src.name().split(' ').next().unwrap_or("").to_lowercase()
            )),
            _ => None,
        })
        .collect();
    put_right(buf, boxr, x1, y, &counts.join(" · "), fg(t.dim));
    y += 2;

    // Columns as in the design: title, artist, album, format, time.
    let w = x1.saturating_sub(x0 + 2);
    let title_x = x0 + 2;
    let artist_x = title_x + w * 37 / 100;
    let album_x = artist_x + w * 22 / 100;
    let format_x = x1.saturating_sub(16);
    let head = bold(t.dim);
    put(buf, inner, title_x, y, "TITLE", head);
    put(buf, inner, artist_x, y, "ARTIST", head);
    put(buf, inner, album_x, y, "ALBUM", head);
    put(buf, inner, format_x, y, "FORMAT", head);
    put_right(buf, inner, x1, y, "TIME", head);
    y += 1;

    let list = Rect::new(
        inner.x,
        y,
        inner.width,
        inner.bottom().saturating_sub(y + 1),
    );
    let rows = s.rows();
    let n = list.height as usize;
    let top = scroll(s.top.get(), s.sel, n, rows.len());
    s.top.set(top);
    let playing = player.current();
    for (i, row) in rows.iter().enumerate().skip(top).take(n) {
        let ry = list.y + (i - top) as u16;
        match row {
            SearchRow::Heading(source) => {
                let (icon, color) = source_icon(*source, t);
                let x = put(buf, list, x0, ry, icon, fg(color));
                let x = put(
                    buf,
                    list,
                    x + 1,
                    ry,
                    &source.name().to_uppercase(),
                    bold(color),
                );
                let note = match &s.found[crate::search::index(*source)] {
                    Found::Songs(songs, total) if *total > songs.len() => {
                        format!("{} of {}", songs.len(), group_thousands(*total))
                    }
                    Found::Songs(songs, _) => group_thousands(songs.len()),
                    Found::Searching => "searching…".into(),
                    _ => String::new(),
                };
                put(buf, list, x + 2, ry, &note, fg(t.dim));
            }
            SearchRow::Note(source) => {
                let note = match &s.found[crate::search::index(*source)] {
                    Found::Songs(..) => "No matches".to_string(),
                    Found::Searching => "Searching…".to_string(),
                    Found::Failed(e) => e.clone(),
                    Found::Unavailable(why) => why.clone(),
                    Found::Idle => String::new(),
                };
                let style = fg(t.dim).add_modifier(Modifier::ITALIC);
                put(
                    buf,
                    list,
                    title_x,
                    ry,
                    &fit(&note, x1.saturating_sub(title_x)),
                    style,
                );
            }
            SearchRow::Gap => {}
            SearchRow::Song(..) => {
                let Some(track) = s.song(row) else { continue };
                let rowr = Rect::new(list.x, ry, list.width, 1);
                hit(rowr, Target::SearchRow(i));
                let selected = i == s.sel && !s.typing;
                if selected {
                    fill(buf, rowr, Style::new().bg(t.selection));
                }
                if playing.is_some_and(|p| {
                    p.source == track.source && p.uri == track.uri && p.path == track.path
                }) {
                    put(buf, list, x0, ry, "▶", bold(t.accent));
                }
                let style = if selected {
                    bold(t.bright)
                } else {
                    fg(t.light)
                };
                put(
                    buf,
                    list,
                    title_x,
                    ry,
                    &fit(&track.title, artist_x - title_x - 2),
                    style,
                );
                put(
                    buf,
                    list,
                    artist_x,
                    ry,
                    &fit(&track.artist, album_x - artist_x - 2),
                    fg(t.fg),
                );
                let album_w = format_x.saturating_sub(album_x + 2);
                put(
                    buf,
                    list,
                    album_x,
                    ry,
                    &fit(&album_year(&track.album, track.year), album_w),
                    fg(t.dim),
                );
                let color = if hi_res(track) { t.light } else { t.fg };
                put(buf, list, format_x, ry, &format_of(track), fg(color));
                put_right(
                    buf,
                    list,
                    x1,
                    ry,
                    &clock(track.duration_ms as f64 / 1000.0),
                    fg(t.dim),
                );
            }
        }
    }
    let pairs: &[(&str, &str)] = if s.typing {
        &[("type", "to search"), ("⏎ ↓", "results"), ("Esc", "close")]
    } else {
        &[
            ("⏎", "play now"),
            ("a", "add to queue"),
            ("A", "add all"),
            ("Tab", "next source"),
            ("/", "edit search"),
            ("Esc", "close"),
        ]
    };
    hints(buf, inner, x0, inner.bottom().saturating_sub(1), pairs, t);
}

// ---- Settings ------------------------------------------------------------------------

pub(super) fn draw_settings(
    buf: &mut Buffer,
    screen: Rect,
    s: &Settings,
    config: &crate::config::Config,
    facts: &Facts,
    t: &Theme,
    hit: &mut impl FnMut(Rect, Target),
) {
    let r = big_box(screen);
    let inner = popup(buf, r, "SETTINGS", t, hit);
    let rows = settings::rows(config, facts);
    let current = s.section(&rows);

    // Sections down the left.
    let nav_x = inner.x + 1;
    let nav_w: u16 = 18;
    for (i, name) in SECTIONS.iter().enumerate() {
        let y = inner.y + 1 + i as u16;
        let area = Rect::new(nav_x, y, nav_w, 1);
        hit(area, Target::SettingsSection(i));
        if i == current {
            fill(buf, area, Style::new().bg(t.selection));
            put(buf, inner, nav_x + 1, y, name, bold(t.bright));
        } else {
            put(buf, inner, nav_x + 1, y, name, fg(t.fg));
        }
    }
    let line_x = nav_x + nav_w + 1;
    for y in inner.y..inner.bottom().saturating_sub(2) {
        put(buf, inner, line_x, y, "│", fg(t.selection));
    }

    // The rows, with a heading at the start of each section.
    let x0 = line_x + 3;
    let x1 = inner.right().saturating_sub(2);
    let label_w: u16 = 18;
    let mut lines: Vec<(Option<usize>, Option<usize>)> = Vec::new(); // (heading section, row)
    let mut last = usize::MAX;
    for (i, row) in rows.iter().enumerate() {
        if row.section != last {
            if last != usize::MAX {
                lines.push((None, None));
            }
            lines.push((Some(row.section), None));
            lines.push((None, None));
            last = row.section;
        }
        lines.push((None, Some(i)));
    }
    let view = inner.height.saturating_sub(3) as usize;
    let sel_line = lines.iter().position(|l| l.1 == Some(s.sel)).unwrap_or(0);
    let top = scroll(s.top.get(), sel_line, view, lines.len());
    s.top.set(top);
    let area = Rect::new(x0 - 1, inner.y + 1, x1 + 2 - x0, view as u16);

    for (n, (heading, row)) in lines.iter().enumerate().skip(top).take(view) {
        let y = area.y + (n - top) as u16;
        if let Some(section) = heading {
            put(
                buf,
                area,
                x0,
                y,
                &SECTIONS[*section].to_uppercase(),
                bold(t.dim),
            );
            continue;
        }
        let Some(i) = *row else { continue };
        let row = &rows[i];
        let line = Rect::new(area.x, y, area.width, 1);
        hit(line, Target::SettingsRow(i));
        let selected = i == s.sel;
        if selected {
            fill(buf, line, Style::new().bg(t.selection));
        }
        put(
            buf,
            area,
            x0,
            y,
            row.label,
            fg(if selected { t.bright } else { t.fg }),
        );
        let mut x = x0 + label_w;
        match &row.value {
            Value::Choice(options, chosen) => {
                for (n, option) in options.iter().enumerate() {
                    let on = n == *chosen;
                    let start = x;
                    x = put(
                        buf,
                        area,
                        x,
                        y,
                        if on { "● " } else { "○ " },
                        fg(if on { t.accent } else { t.dim }),
                    );
                    x = put(
                        buf,
                        area,
                        x,
                        y,
                        option,
                        if on { bold(t.bright) } else { fg(t.fg) },
                    );
                    hit(
                        Rect::new(start, y, x - start, 1),
                        Target::SettingsChoice(i, n),
                    );
                    x += 3;
                }
            }
            Value::Number(n) => {
                x = put(buf, area, x, y, &format!("◂ {n} ▸"), bold(t.bright));
                x += 2;
            }
            Value::Text(text) => {
                if row.id == settings::Id::AddFolder {
                    if let Some(typed) = &s.adding {
                        field(
                            buf,
                            Rect::new(x, y, x1.saturating_sub(x + 2).min(50), 1),
                            typed,
                            true,
                            t,
                        );
                    }
                } else {
                    let room = x1.saturating_sub(x + 2 + row.button.map_or(0, |b| width(b) + 4));
                    x = put(buf, area, x, y, &fit(text, room), fg(t.light));
                    x += 3;
                }
            }
        }
        let room = x1.saturating_sub(x + row.button.map_or(0, |b| width(b) + 4));
        if !row.note.is_empty() && room > 8 {
            put(buf, area, x, y, &fit(&row.note, room), fg(t.dim));
        }
        if let Some(label) = row.button {
            let bx = x1.saturating_sub(width(label) + 3);
            let pill_area = pill(buf, area, bx, y, label, t);
            hit(pill_area, Target::SettingsButton(i));
        }
    }

    let pairs: &[(&str, &str)] = if s.adding.is_some() {
        &[
            ("type", "a folder, e.g. ~/Music"),
            ("⏎", "add"),
            ("Esc", "cancel"),
        ]
    } else {
        &[
            ("↑↓", "move"),
            ("←→", "change"),
            ("⏎", "press"),
            ("Tab", "section"),
            ("Esc", "close"),
        ]
    };
    hints(buf, inner, x0, inner.bottom().saturating_sub(1), pairs, t);
}

// ---- signing in ------------------------------------------------------------------------

fn numbered(buf: &mut Buffer, clip: Rect, x: u16, y: u16, n: &str, text: &str, t: &Theme) -> u16 {
    put(buf, clip, x, y, n, bold(t.accent));
    paragraph(buf, clip, x + 4, y, text, fg(t.light))
}

/// A web address you can click to open, with a Copy button after it. Shown in full where it
/// fits, so the terminal's own link finder (Ctrl+Shift+O in foot) can use it too.
#[allow(clippy::too_many_arguments)]
fn link_row(
    buf: &mut Buffer,
    clip: Rect,
    x: u16,
    y: u16,
    url: &str,
    link: Link,
    t: &Theme,
    hit: &mut impl FnMut(Rect, Target),
) -> u16 {
    let room = clip.right().saturating_sub(x + 12);
    let shown = fit(url, room);
    let end = put(
        buf,
        clip,
        x,
        y,
        &shown,
        fg(t.accent).add_modifier(Modifier::UNDERLINED),
    );
    hit(Rect::new(x, y, end - x, 1), Target::Link(link));
    let copy = pill(buf, clip, end + 2, y, "Copy", t);
    hit(copy, Target::Copy(link));
    y + 1
}

pub(super) fn draw_spotify(
    buf: &mut Buffer,
    screen: Rect,
    step: &SpotifyStep,
    t: &Theme,
    hit: &mut impl FnMut(Rect, Target),
) {
    let r = centred(screen, 80, 24);
    let inner = popup(buf, r, "CONNECT SPOTIFY", t, hit);
    let x = inner.x + 2;
    let mut y = inner.y + 1;
    let error = |buf: &mut Buffer, y: u16, e: &Option<String>| match e {
        Some(e) => paragraph(buf, inner, x, y, e, fg(t.red)),
        None => y,
    };
    let pairs: &[(&str, &str)] = match step {
        SpotifyStep::ClientId { input, error: e } => {
            y = numbered(
                buf,
                inner,
                x,
                y,
                "1",
                "Sign in to the Spotify developer dashboard with your Premium account:",
                t,
            );
            y = link_row(buf, inner, x + 4, y, DASHBOARD, Link::Dashboard, t, hit);
            y = numbered(
                buf,
                inner,
                x,
                y + 1,
                "2",
                "Create an app (any name, e.g. OMAESTRO). Tick \"Web API\". As the Redirect URI, add exactly:",
                t,
            );
            y = link_row(
                buf,
                inner,
                x + 4,
                y,
                spotify::REDIRECT_URI,
                Link::Redirect,
                t,
                hit,
            );
            y = numbered(
                buf,
                inner,
                x,
                y + 1,
                "3",
                "Open the app's settings, copy its Client ID, paste it here (Ctrl+Shift+V) and press Enter. Your browser opens to sign in.",
                t,
            );
            field(
                buf,
                Rect::new(x + 4, y + 1, inner.width.saturating_sub(12), 1),
                input,
                true,
                t,
            );
            error(buf, y + 3, e);
            &[
                ("⏎", "connect"),
                ("click", "a link to open it"),
                ("Esc", "cancel"),
            ]
        }
        SpotifyStep::Waiting { url } => {
            y = paragraph(
                buf,
                inner,
                x,
                y,
                "Your browser should be showing Spotify's sign-in page. Agree there, and this screen moves on by itself.",
                fg(t.light),
            );
            y = paragraph(
                buf,
                inner,
                x,
                y + 1,
                "If no browser opened, open the page from here:",
                fg(t.dim),
            );
            link_row(buf, inner, x, y, url, Link::SignIn, t, hit);
            &[("click", "the link to open it"), ("Esc", "cancel")]
        }
        SpotifyStep::Playback { input, error: e } => {
            y = paragraph(
                buf,
                inner,
                x,
                y,
                "To play Spotify on this computer, OMAESTRO uses Spotify Soloist, Spotify's own background player. (Spotify on Sonos doesn't need it.)",
                fg(t.light),
            );
            y = numbered(
                buf,
                inner,
                x,
                y + 1,
                "1",
                "Get an API key here (Premium accounts can):",
                t,
            );
            y = link_row(
                buf,
                inner,
                x + 4,
                y,
                soloist::KEY_PAGE,
                Link::SoloistKeys,
                t,
                hit,
            );
            y = numbered(
                buf,
                inner,
                x,
                y + 1,
                "2",
                "Paste it here and press Enter. OMAESTRO downloads Soloist into ~/.local/share/omaestro/soloist (no sudo needed), then asks you to pick \"OMAESTRO\" in the Spotify app once.",
                t,
            );
            field(
                buf,
                Rect::new(x + 4, y + 1, inner.width.saturating_sub(12), 1),
                input,
                true,
                t,
            );
            error(buf, y + 3, e);
            &[
                ("⏎", "set up"),
                ("click", "a link to open it"),
                ("Esc", "later"),
            ]
        }
        SpotifyStep::SettingUp(message) => {
            y = paragraph(buf, inner, x, y, message, bold(t.bright));
            paragraph(
                buf,
                inner,
                x,
                y + 1,
                "This screen moves on by itself. Pairing waits up to 10 minutes.",
                fg(t.dim),
            );
            &[("Esc", "stop waiting")]
        }
        SpotifyStep::Done(message) => {
            put(buf, inner, x, y, "✓", bold(t.accent));
            paragraph(buf, inner, x + 2, y, message, bold(t.bright));
            &[("any key", "close")]
        }
    };
    hints(buf, inner, x, inner.bottom().saturating_sub(1), pairs, t);
}

pub(super) fn draw_youtube(
    buf: &mut Buffer,
    screen: Rect,
    step: &YouTubeStep,
    t: &Theme,
    hit: &mut impl FnMut(Rect, Target),
) {
    let rows = match step {
        YouTubeStep::Browser { list, .. } => list.len() as u16,
        _ => 0,
    };
    let r = centred(screen, 80, (rows + 16).max(18));
    let inner = popup(buf, r, "CONNECT YOUTUBE MUSIC", t, hit);
    let x = inner.x + 2;
    let mut y = inner.y + 1;
    let pairs: &[(&str, &str)] = match step {
        YouTubeStep::Browser { list, sel, error } => {
            y = paragraph(
                buf,
                inner,
                x,
                y,
                "OMAESTRO signs in with your browser's YouTube login: yt-dlp copies its YouTube cookies into a file only you can read. Sign in to YouTube Music in that browser first:",
                fg(t.light),
            );
            y = link_row(buf, inner, x, y, YOUTUBE, Link::YouTube, t, hit);
            y = paragraph(
                buf,
                inner,
                x,
                y + 1,
                "Which browser, and which profile, are you signed in with?",
                bold(t.dim),
            );
            if list.is_empty() {
                y = paragraph(
                    buf,
                    inner,
                    x,
                    y,
                    "No supported browser found (Chromium, Chrome, Brave, Firefox, Vivaldi, Edge, Opera).",
                    fg(t.red),
                );
            }
            for (i, login) in list.iter().enumerate() {
                let row = Rect::new(x, y, inner.width.saturating_sub(4), 1);
                hit(row, Target::Browser(i));
                if i == *sel {
                    fill(buf, row, Style::new().bg(t.selection));
                }
                let style = if i == *sel {
                    bold(t.bright)
                } else {
                    fg(t.light)
                };
                put(
                    buf,
                    inner,
                    x + 2,
                    y,
                    &fit(&login.label, row.width.saturating_sub(3)),
                    style,
                );
                y += 1;
            }
            if let Some(e) = error {
                paragraph(buf, inner, x, y + 1, e, fg(t.red));
            }
            &[("↑↓", "choose"), ("⏎", "connect"), ("Esc", "cancel")]
        }
        YouTubeStep::Connecting(browser) => {
            y = paragraph(
                buf,
                inner,
                x,
                y,
                &format!("Reading the YouTube login from {browser}…"),
                bold(t.bright),
            );
            paragraph(
                buf,
                inner,
                x,
                y + 1,
                "Your desktop may ask to unlock the keyring, where the browser keeps its cookies.",
                fg(t.dim),
            );
            &[("Esc", "stop waiting")]
        }
        YouTubeStep::Done(user) => {
            put(buf, inner, x, y, "✓", bold(t.accent));
            y = paragraph(
                buf,
                inner,
                x + 2,
                y,
                &format!("Connected as {user}"),
                bold(t.bright),
            );
            paragraph(
                buf,
                inner,
                x,
                y + 1,
                "YouTube asks for a fresh login now and then; if search or playback says so, connect here again.",
                fg(t.dim),
            );
            &[("any key", "close")]
        }
    };
    hints(buf, inner, x, inner.bottom().saturating_sub(1), pairs, t);
}

// ---- Open ----------------------------------------------------------------------------

pub(super) fn draw_open(
    buf: &mut Buffer,
    screen: Rect,
    open: &Open,
    t: &Theme,
    hit: &mut impl FnMut(Rect, Target),
) {
    let r = centred(screen, 70, 24);
    let inner = popup(buf, r, "OPEN PLAYLIST", t, hit);
    let (cx, cr) = (inner.x + 1, inner.right().saturating_sub(2));
    // Tabs: Saved, Spotify, YouTube Music.
    let mut x = cx;
    for source in crate::search::SOURCES {
        let label = if source == Source::Local {
            "Saved"
        } else {
            source.name()
        };
        let (icon, color) = source_icon(source, t);
        let start = x;
        if source == open.tab {
            x = put(
                buf,
                inner,
                x,
                inner.y,
                &format!(" {icon} "),
                fg(color).bg(t.selection),
            );
            x = put(
                buf,
                inner,
                x,
                inner.y,
                &format!("{label} "),
                bold(t.bright).bg(t.selection),
            );
        } else {
            x = put(
                buf,
                inner,
                x,
                inner.y,
                &format!(" {icon} {label} "),
                fg(t.dim),
            );
        }
        hit(
            Rect::new(start, inner.y, x - start, 1),
            Target::OpenTab(source),
        );
        x += 1;
    }

    let list = Rect::new(
        inner.x,
        inner.y + 2,
        inner.width,
        inner.height.saturating_sub(4),
    );
    let rows = list.height as usize;
    let note = |buf: &mut Buffer, text: &str| {
        paragraph(
            buf,
            list,
            cx,
            list.y,
            text,
            fg(t.dim).add_modifier(Modifier::ITALIC),
        );
    };
    let entries: Vec<(String, String)> = match open.tab {
        Source::Local => {
            if open.local.is_empty() {
                note(buf, "No saved playlists yet. S saves the queue as one.");
            }
            open.local
                .iter()
                .map(|p| {
                    (
                        p.name.clone(),
                        format!("{} · {}", plural(p.tracks, "track"), age(p.modified)),
                    )
                })
                .collect()
        }
        Source::Spotify | Source::YouTube => {
            let listing = if open.tab == Source::Spotify {
                &open.spotify
            } else {
                &open.youtube
            };
            match listing {
                Listing::NotConnected => {
                    note(
                        buf,
                        &format!(
                            "Connect {} in Settings (gear button) to see its playlists here.",
                            open.tab.name()
                        ),
                    );
                    Vec::new()
                }
                Listing::Loading => {
                    note(buf, "Loading…");
                    Vec::new()
                }
                Listing::Failed(e) => {
                    note(buf, e);
                    Vec::new()
                }
                Listing::Ready(lists) => lists
                    .iter()
                    .map(|l| {
                        (
                            l.name.clone(),
                            l.tracks.map(|n| plural(n, "track")).unwrap_or_default(),
                        )
                    })
                    .collect(),
            }
        }
    };
    let top = scroll(0, open.sel, rows, entries.len());
    for (i, (name, about)) in entries.iter().enumerate().skip(top).take(rows) {
        let y = list.y + (i - top) as u16;
        let row = Rect::new(inner.x, y, inner.width, 1);
        hit(row, Target::Playlist(i));
        if i == open.sel {
            fill(buf, row, Style::new().bg(t.selection));
        }
        let name_w = cr.saturating_sub(cx + width(about) + 2);
        let style = if i == open.sel {
            bold(t.bright)
        } else {
            fg(t.light)
        };
        put(buf, inner, cx, y, &fit(name, name_w), style);
        put_right(buf, inner, cr, y, about, fg(t.dim));
    }
    let pairs = [
        ("⏎", "open (replaces the queue)"),
        ("Tab", "source"),
        ("Esc", "close"),
    ];
    hints(buf, inner, cx, inner.bottom().saturating_sub(1), &pairs, t);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_at_spaces() {
        assert_eq!(wrap("one two three four", 9), ["one two", "three", "four"]);
        assert_eq!(wrap("", 5), Vec::<String>::new());
        assert_eq!(wrap("averylongword x", 4), ["averylongword", "x"]);
    }
}
