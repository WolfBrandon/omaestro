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
pub(super) fn field(buf: &mut Buffer, r: Rect, text: &str, focused: bool, t: &Theme) {
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

/// The Search screen, as in the OTIS tab "Search (New)": the search box with the providers
/// under it, then a box of results for each provider that's on. When they don't all fit, the
/// box with the selection gets the room and the others show their first few.
pub(super) fn draw_search(
    buf: &mut Buffer,
    screen: Rect,
    s: &Search,
    player: &Player,
    statuses: &crate::status::Statuses,
    t: &Theme,
    hit: &mut impl FnMut(Rect, Target),
) {
    use crate::search::{SOURCES, index};
    let w = screen.width.saturating_sub(4).min(100);
    let h = screen.height.saturating_sub(4).min(38);
    let inner = popup(buf, centred(screen, w, h), "SEARCH", t, hit);
    if inner.width < 30 || inner.height < 8 {
        return;
    }

    // × closes.
    let close_x = inner.right() - 2;
    put(buf, inner, close_x, inner.y, "×", bold(t.light));
    hit(Rect::new(close_x - 1, inner.y, 3, 1), Target::CloseOverlay);

    // The search box.
    let search = Rect::new(inner.x + 2, inner.y, 60.min(close_x - inner.x - 4), 3);
    let border = if s.typing { t.light } else { t.fg };
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(fg(border))
        .render(search, buf);
    hit(search, Target::SearchBox);
    let line = Rect::new(search.x + 1, search.y + 1, search.width - 2, 1);
    let x = put(
        buf,
        line,
        line.x,
        line.y,
        &format!("{I_SEARCH}  "),
        fg(t.fg),
    );
    if s.query.is_empty() && !s.typing {
        let hint = fg(t.dim).add_modifier(Modifier::ITALIC);
        put(
            buf,
            line,
            x,
            line.y,
            "Songs from every provider that's on",
            hint,
        );
    } else {
        // Show the end of a long search, so what's being typed stays in view.
        let room = line.right().saturating_sub(x + 2);
        let mut shown = s.query.as_str();
        while width(shown) > room {
            let mut chars = shown.chars();
            chars.next();
            shown = chars.as_str();
        }
        let x = put(buf, line, x, line.y, shown, bold(t.bright));
        if s.typing {
            put(buf, line, x, line.y, "▏", fg(t.accent));
        }
    }

    // The providers: on ones filled, off ones struck through, each with a dot for how it's
    // doing (once checked).
    let y = inner.y + 3;
    let mut x = inner.x + 3;
    for source in SOURCES {
        let (icon, color) = source_icon(source, t);
        let health = match source {
            Source::Local => &statuses.local,
            Source::Spotify => &statuses.spotify,
            Source::YouTube => &statuses.youtube,
        }
        .as_ref()
        .map(|st| health_color(st.health, t));
        let start = x;
        let on = s.show[index(source)];
        if on {
            x = put(
                buf,
                inner,
                x,
                y,
                &format!(" {icon}"),
                fg(color).bg(t.selection),
            );
            let name = format!(" {} ", source.name());
            x = put(buf, inner, x, y, &name, bold(t.bright).bg(t.selection));
        } else {
            x = put(buf, inner, x, y, &format!(" {icon}"), fg(t.dim));
            let off = fg(t.dim).add_modifier(Modifier::CROSSED_OUT);
            x = put(buf, inner, x, y, &format!(" {} ", source.name()), off);
        }
        if let Some(dot) = health {
            let bg = if on { t.selection } else { t.panel };
            x = put(buf, inner, x, y, "● ", fg(dot).bg(bg));
        }
        hit(
            Rect::new(start, y, x - start, 1),
            Target::SearchChip(source),
        );
        x += 2;
    }
    put(buf, inner, x, y, "1 2 3 toggle providers", fg(t.dim));

    // The result boxes, between the providers and the key hints.
    let rows = s.rows();
    let selected = rows
        .get(s.sel)
        .filter(|_| !s.typing)
        .and_then(|row| match row {
            SearchRow::Song(source, n) => Some((*source, *n)),
            _ => None,
        });
    let focus = selected
        .map(|(source, _)| source)
        .or_else(|| SOURCES.into_iter().find(|src| s.show[index(*src)]));
    let songs = |source: Source| match &s.found[index(source)] {
        Found::Songs(songs, _) => songs.as_slice(),
        _ => &[],
    };
    // Boxes take their borders, a heading row when there are songs, and a gap before them;
    // a provider that's off takes one line.
    let (top, bottom) = (inner.y + 5, inner.bottom().saturating_sub(2));
    let mut fixed = 0;
    for (i, source) in SOURCES.into_iter().enumerate() {
        fixed += if !s.show[index(source)] {
            1
        } else {
            2 + u16::from(!songs(source).is_empty()) + u16::from(i > 0)
        };
    }
    let room = bottom.saturating_sub(top).saturating_sub(fixed) as usize;
    let mut give = [0usize; 3];
    for source in SOURCES.into_iter().filter(|src| s.show[index(*src)]) {
        give[index(source)] = songs(source).len().max(1);
    }
    if give.iter().sum::<usize>() > room {
        for source in SOURCES.into_iter().filter(|src| Some(*src) != focus) {
            give[index(source)] = give[index(source)].min(3);
        }
        let others: usize = SOURCES
            .into_iter()
            .filter(|src| Some(*src) != focus)
            .map(|src| give[index(src)])
            .sum();
        if let Some(focus) = focus {
            let wanted = give[index(focus)];
            give[index(focus)] = room.saturating_sub(others).clamp(1, wanted.max(1));
        }
    }

    let left = inner.x + 2;
    let box_w = inner.width.saturating_sub(5);
    let mut y = top;
    for (i, source) in SOURCES.into_iter().enumerate() {
        if y >= bottom {
            break;
        }
        let n = index(source);
        let (icon, color) = source_icon(source, t);
        if !s.show[n] {
            put(buf, inner, left + 1, y, icon, fg(t.dim));
            let note = format!("  {} is off · press {} to include it", source.name(), n + 1);
            put(
                buf,
                inner,
                left + 2,
                y,
                &note,
                fg(t.dim).add_modifier(Modifier::ITALIC),
            );
            y += 1;
            continue;
        }
        if i > 0 {
            y += 1;
        }
        let list = songs(source);
        let heading = !list.is_empty();
        let height = (2 + u16::from(heading) + give[n] as u16).min(bottom.saturating_sub(y));
        if height < 3 {
            break;
        }
        let area = Rect::new(left, y, box_w, height);
        y += height;

        let focused = Some(source) == focus;
        let name = source.name().to_uppercase();
        let title = match &s.found[n] {
            Found::Songs(songs, total) if *total > songs.len() => {
                format!("{name} ({} of {})", songs.len(), group_thousands(*total))
            }
            Found::Songs(songs, _) => format!("{name} ({})", songs.len()),
            Found::Searching => format!("{name} · searching…"),
            _ => name,
        };
        let (border, title_style) = if focused {
            (t.bright, bold(t.bright))
        } else {
            (t.fg, fg(t.light))
        };
        Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(fg(border))
            .title(Line::from(vec![
                Span::styled("─", fg(border)),
                Span::styled(format!(" {title} "), title_style),
            ]))
            .render(area, buf);
        let body = Rect::new(area.x + 1, area.y + 1, area.width - 2, area.height - 2);

        // Columns as in the design: title, artist, album, format, time.
        let icon_x = body.x + 1;
        let title_x = body.x + 4;
        let time_end = body.right().saturating_sub(3);
        let format_x = time_end.saturating_sub(14);
        let span = format_x.saturating_sub(title_x + 2);
        // The album gets a little more room than in the design, for its year.
        let artist_x = title_x + span * 44 / 100;
        let album_x = artist_x + span * 23 / 100;

        if !heading {
            let note = match &s.found[n] {
                Found::Songs(..) => "No matches".to_string(),
                Found::Searching => "Searching…".to_string(),
                Found::Failed(e) => e.clone(),
                Found::Unavailable(why) => why.clone(),
                Found::Idle => "Type above to search".to_string(),
            };
            let style = fg(t.dim).add_modifier(Modifier::ITALIC);
            put(
                buf,
                body,
                title_x,
                body.y,
                &fit(&note, body.right().saturating_sub(title_x + 1)),
                style,
            );
            continue;
        }
        let head = bold(t.dim);
        fill(
            buf,
            Rect::new(body.x, body.y, body.width, 1),
            Style::new().bg(t.darker),
        );
        put(buf, body, title_x, body.y, "TITLE", head);
        put(buf, body, artist_x, body.y, "ARTIST", head);
        put(buf, body, album_x, body.y, "ALBUM", head);
        put(buf, body, format_x, body.y, "FORMAT", head);
        put_right(buf, body, time_end, body.y, "TIME", head);

        let shown = body.height.saturating_sub(1) as usize;
        let mine = selected.filter(|(src, _)| *src == source).map(|(_, i)| i);
        let offset = if focused {
            let offset = scroll(s.top.get(), mine.unwrap_or(0), shown, list.len());
            s.top.set(offset);
            offset
        } else {
            0
        };
        let first_row = rows
            .iter()
            .position(|r| *r == SearchRow::Song(source, 0))
            .unwrap_or(0);
        let playing = player.current();
        for (k, track) in list.iter().enumerate().skip(offset).take(shown) {
            let ry = body.y + 1 + (k - offset) as u16;
            let rowr = Rect::new(body.x, ry, body.width, 1);
            hit(rowr, Target::SearchRow(first_row + k));
            let chosen = mine == Some(k);
            if chosen {
                fill(buf, rowr, Style::new().bg(t.selection));
            }
            let is_playing = playing.is_some_and(|p| {
                p.source == track.source && p.uri == track.uri && p.path == track.path
            });
            if is_playing {
                put(buf, body, icon_x, ry, "▶", bold(t.accent));
            } else {
                put(buf, body, icon_x, ry, icon, fg(color));
            }
            let style = if chosen { bold(t.bright) } else { fg(t.light) };
            put(
                buf,
                body,
                title_x,
                ry,
                &fit(&track.title, artist_x - title_x - 2),
                style,
            );
            put(
                buf,
                body,
                artist_x,
                ry,
                &fit(&track.artist, album_x - artist_x - 2),
                fg(t.fg),
            );
            let album = album_year(&track.album, track.year);
            put(
                buf,
                body,
                album_x,
                ry,
                &fit(&album, format_x.saturating_sub(album_x + 2)),
                fg(t.dim),
            );
            if track.source == Source::Local {
                let color = if hi_res(track) { t.light } else { t.fg };
                put(buf, body, format_x, ry, &format_of(track), fg(color));
            }
            let time = clock(track.duration_ms as f64 / 1000.0);
            put_right(buf, body, time_end, ry, &time, fg(t.dim));
        }
    }

    let pairs: &[(&str, &str)] = if s.typing {
        &[("type", "to search"), ("⏎ ↓", "results"), ("Esc", "close")]
    } else {
        &[
            ("⏎", "play"),
            ("a", "add to queue"),
            ("A", "add all"),
            ("Tab", "provider"),
            ("Esc", "close"),
        ]
    };
    hints(buf, inner, inner.x + 1, inner.bottom() - 1, pairs, t);
}

// ---- File Info -----------------------------------------------------------------------

/// The end of `text` in `w` columns, starting "…" when cut: for paths, whose end matters.
fn fit_end(text: &str, w: u16) -> String {
    if width(text) <= w {
        return text.to_string();
    }
    let mut shown = text;
    while width(shown) + 1 > w && !shown.is_empty() {
        let mut chars = shown.chars();
        chars.next();
        shown = chars.as_str();
    }
    format!("…{shown}")
}

/// File Info: everything about one song.
pub(super) fn draw_file_info(
    buf: &mut Buffer,
    screen: Rect,
    info: &crate::app::FileInfo,
    t: &Theme,
    hit: &mut impl FnMut(Rect, Target),
) {
    use crate::fileinfo;
    let track = &info.track;
    let local = track.source == Source::Local;
    let w = screen.width.saturating_sub(4).min(80);
    let h = screen.height.saturating_sub(2).min(24);
    let inner = popup(buf, centred(screen, w, h), "FILE INFO", t, hit);
    if inner.width < 30 || inner.height < 8 {
        return;
    }
    let x = inner.x + 2;
    let value_x = x + 15;
    let room = inner.right().saturating_sub(value_x + 2);
    let mut y = inner.y + 1;

    let (icon, color) = source_icon(track.source, t);
    let after = put(buf, inner, x, y, icon, fg(color));
    put(
        buf,
        inner,
        after + 2,
        y,
        &fit(&track.title, room + 12),
        bold(t.bright),
    );
    y += 1;
    put(
        buf,
        inner,
        x + 3,
        y,
        &fit(&track.artist, room + 12),
        fg(t.light),
    );
    y += 2;

    let row = |buf: &mut Buffer, label: &str, value: &str, style: Style, y: &mut u16| {
        if value.is_empty() || *y + 2 >= inner.bottom() {
            return;
        }
        put(buf, inner, x, *y, label, fg(t.dim));
        put(buf, inner, value_x, *y, &fit(value, room), style);
        *y += 1;
    };
    let plain = fg(t.fg);
    put(buf, inner, x, y, "SONG", bold(t.dim));
    y += 1;
    row(
        buf,
        "Album",
        &album_year(&track.album, track.year),
        plain,
        &mut y,
    );
    if track.album_artist != track.artist {
        row(buf, "Album artist", &track.album_artist, plain, &mut y);
    }
    let number = match (track.track_no, track.disc_no) {
        (Some(n), Some(d)) if d > 1 => format!("{n} · disc {d}"),
        (Some(n), _) => n.to_string(),
        _ => String::new(),
    };
    row(buf, "Track", &number, plain, &mut y);
    row(buf, "Genre", &track.genre, plain, &mut y);
    row(
        buf,
        "Length",
        &clock(track.duration_ms as f64 / 1000.0),
        plain,
        &mut y,
    );
    row(buf, "ISRC", &track.isrc, fg(t.dim), &mut y);
    row(buf, "MusicBrainz", &track.mb_track_id, fg(t.dim), &mut y);
    y += 1;

    if local {
        put(buf, inner, x, y, "FILE", bold(t.dim));
        y += 1;
        put(buf, inner, x, y, "Path", fg(t.dim));
        let pill_w = width("Copy") + 4;
        let path_room = room.saturating_sub(pill_w + 1);
        let end = put(
            buf,
            inner,
            value_x,
            y,
            &fit_end(&track.path, path_room),
            fg(t.light),
        );
        let area = pill(buf, inner, end + 1, y, "Copy", t);
        hit(area, Target::InfoCopy);
        y += 1;
        let ext = std::path::Path::new(&track.path)
            .extension()
            .map(|e| e.to_string_lossy().to_uppercase())
            .unwrap_or_default();
        let format = [
            ext,
            track
                .bit_depth
                .map(|b| format!("{b}-bit"))
                .unwrap_or_default(),
            track
                .sample_rate
                .map(|r| format!("{} kHz", r as f64 / 1000.0))
                .unwrap_or_default(),
        ]
        .into_iter()
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join(" · ");
        row(buf, "Format", &format, plain, &mut y);
        match &info.details {
            None => {
                let style = fg(t.dim).add_modifier(Modifier::ITALIC);
                put(buf, inner, value_x, y, "Reading the file…", style);
                y += 1;
            }
            Some(Err(e)) => {
                let style = fg(t.yellow).add_modifier(Modifier::ITALIC);
                put(
                    buf,
                    inner,
                    value_x,
                    y,
                    &fit(&format!("Can't read it: {e}"), room),
                    style,
                );
                y += 1;
            }
            Some(Ok(d)) => {
                let bitrate = d
                    .bitrate
                    .map(|b| format!("{} kbps", crate::app::group_thousands(b as usize)))
                    .unwrap_or_default();
                row(buf, "Bitrate", &bitrate, plain, &mut y);
                row(
                    buf,
                    "Channels",
                    &d.channels.map(fileinfo::channels).unwrap_or_default(),
                    plain,
                    &mut y,
                );
                row(
                    buf,
                    "Size",
                    &d.size.map(fileinfo::size).unwrap_or_default(),
                    plain,
                    &mut y,
                );
                let gain = match (&d.track_gain, &d.album_gain) {
                    (None, None) => "none".to_string(),
                    (t, a) => [
                        t.as_ref().map(|g| format!("track {g}")),
                        a.as_ref().map(|g| format!("album {g}")),
                    ]
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>()
                    .join(" · "),
                };
                row(buf, "ReplayGain", &gain, plain, &mut y);
            }
        }
    } else if let Some(link) = fileinfo::link(track) {
        put(buf, inner, x, y, "LINK", bold(t.dim));
        y += 1;
        let pills_w = width("Open") + width("Copy") + 9;
        let style = fg(t.accent).add_modifier(Modifier::UNDERLINED);
        let end = put(
            buf,
            inner,
            x,
            y,
            &fit(&link, inner.width.saturating_sub(pills_w + 5)),
            style,
        );
        let open = pill(buf, inner, end + 2, y, "Open", t);
        hit(open, Target::InfoOpen);
        let copy = pill(buf, inner, open.right() + 1, y, "Copy", t);
        hit(copy, Target::InfoCopy);
    }
    let _ = y;
    let open_what = if local { "open folder" } else { "open" };
    let pairs = [("c", "copy"), ("o", open_what), ("Esc", "close")];
    hints(buf, inner, inner.x + 1, inner.bottom() - 1, &pairs, t);
}

// ---- Settings ------------------------------------------------------------------------

/// A source's health as a colour: good, worrying, broken, or not set up.
pub(super) fn health_color(health: crate::status::Health, t: &Theme) -> Color {
    use crate::status::Health;
    match health {
        Health::Good => t.accent,
        Health::Warning => t.yellow,
        Health::Bad => t.red,
        Health::Off => t.dim,
    }
}

/// One line of the Settings page: the boxes are drawn a line at a time, so scrolling can cut a
/// box anywhere and its sides still show.
#[derive(Debug, Clone, Copy, PartialEq)]
enum PageLine {
    Top(usize),
    Row(usize),
    Blank(usize),
    /// A line of the theme grid: the row it belongs to, and which line.
    Grid(usize, usize),
    Bottom(usize),
    Gap,
}

/// How wide one theme is in the grid.
const THEME_W: u16 = 22;

/// The Settings screen, as in the OTIS tab "Settings (new)": a box to find a setting, the
/// sections as pills, then the picked section's box (or, while finding, every section with a
/// match). The theme row lists every theme, the chosen one ► between arrows ◄.
pub(super) fn draw_settings(
    buf: &mut Buffer,
    screen: Rect,
    s: &Settings,
    config: &crate::config::Config,
    facts: &Facts,
    t: &Theme,
    hit: &mut impl FnMut(Rect, Target),
) {
    let inner = popup(buf, big_box(screen), "SETTINGS", t, hit);
    if inner.width < 40 || inner.height < 10 {
        return;
    }
    let rows = settings::rows(config, facts);
    let current = s.section(&rows);

    // × closes.
    let close_x = inner.right() - 2;
    put(buf, inner, close_x, inner.y, "×", bold(t.light));
    hit(Rect::new(close_x - 1, inner.y, 3, 1), Target::CloseOverlay);

    // The find box.
    let find = Rect::new(inner.x + 2, inner.y, 60.min(close_x - inner.x - 4), 3);
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(fg(if s.finding { t.light } else { t.fg }))
        .render(find, buf);
    hit(find, Target::SettingsFind);
    let line = Rect::new(find.x + 1, find.y + 1, find.width - 2, 1);
    let x = put(
        buf,
        line,
        line.x,
        line.y,
        &format!("{I_SEARCH}  "),
        fg(t.fg),
    );
    if s.find.is_empty() && !s.finding {
        let hint = fg(t.dim).add_modifier(Modifier::ITALIC);
        put(buf, line, x, line.y, "Find a setting", hint);
    } else {
        let x = put(buf, line, x, line.y, &s.find, bold(t.bright));
        if s.finding {
            put(buf, line, x, line.y, "▏", fg(t.accent));
        }
    }

    // The sections as pills. Eight with icons need 96 columns; narrower, the icons go.
    let y = inner.y + 3;
    let names: u16 = SECTIONS.iter().map(|n| width(n) + 3).sum();
    let icons = names + 2 * SECTIONS.len() as u16 <= inner.right().saturating_sub(inner.x + 2);
    let mut x = inner.x + 2;
    for (i, name) in SECTIONS.iter().enumerate() {
        let on = i == current;
        let any = rows.iter().any(|r| r.section == i && s.shown(r));
        let bg = if on { t.selection } else { t.panel };
        let start = x;
        if icons {
            let color = if on { t.accent } else { t.dim };
            x = put(
                buf,
                inner,
                x,
                y,
                &format!(" {}", settings::SECTION_ICONS[i]),
                fg(color).bg(bg),
            );
        }
        let style = match (on, any) {
            (true, _) => bold(t.bright).bg(bg),
            (false, true) => fg(t.light),
            (false, false) => fg(t.dim),
        };
        x = put(buf, inner, x, y, &format!(" {name} "), style);
        hit(
            Rect::new(start, y, x - start, 1),
            Target::SettingsSection(i),
        );
        x += 1;
    }

    // The section picked in the pills; or, while finding, every section with a match.
    let bx = inner.x + 2;
    let bw = inner.width.saturating_sub(5);
    let br = bx + bw - 1;
    let label_x = bx + 3;
    let value_x = bx + 19;
    let grid_x = bx + 3;
    let per = ((br.saturating_sub(grid_x)) / THEME_W).max(1) as usize;
    let mut lines = Vec::new();
    for section in 0..SECTIONS.len() {
        let members: Vec<usize> = (0..rows.len())
            .filter(|&i| rows[i].section == section && s.visible(&rows[i], current))
            .collect();
        if members.is_empty() {
            continue;
        }
        if !lines.is_empty() {
            lines.push(PageLine::Gap);
        }
        lines.push(PageLine::Top(section));
        for (k, &i) in members.iter().enumerate() {
            if let Value::Pick { options, .. } = &rows[i].value {
                if k > 0 {
                    lines.push(PageLine::Blank(section));
                }
                lines.push(PageLine::Row(i));
                lines.push(PageLine::Blank(section));
                lines.extend((0..options.len().div_ceil(per)).map(|g| PageLine::Grid(i, g)));
                lines.push(PageLine::Blank(section));
            } else {
                lines.push(PageLine::Row(i));
            }
        }
        lines.push(PageLine::Bottom(section));
    }
    let top_y = inner.y + 5;
    let view = inner.bottom().saturating_sub(2 + top_y) as usize;
    if lines.is_empty() {
        let note = format!("No setting matches “{}”", s.find);
        let style = fg(t.dim).add_modifier(Modifier::ITALIC);
        put(buf, inner, label_x, top_y, &fit(&note, bw), style);
    }
    // Keep the selection in view; on the theme row, its grid and the box's end too.
    let sel_line = lines
        .iter()
        .position(|l| *l == PageLine::Row(s.sel))
        .unwrap_or(0);
    let has_grid = lines.contains(&PageLine::Grid(s.sel, 0));
    let box_end = lines[sel_line.min(lines.len().saturating_sub(1))..]
        .iter()
        .position(|l| matches!(l, PageLine::Bottom(_)))
        .map_or(sel_line, |k| sel_line + k);
    let anchor = if has_grid { box_end } else { sel_line };
    let top = scroll(s.top.get(), anchor, view, lines.len());
    let top = scroll(top, sel_line, view, lines.len());
    s.top.set(top);

    let page = Rect::new(bx, top_y, bw, view as u16);
    for (n, line) in lines.iter().enumerate().skip(top).take(view) {
        let y = top_y + (n - top) as u16;
        let section = match *line {
            PageLine::Top(sec) | PageLine::Blank(sec) | PageLine::Bottom(sec) => sec,
            PageLine::Row(i) | PageLine::Grid(i, _) => rows[i].section,
            PageLine::Gap => continue,
        };
        let border = if section == current { t.bright } else { t.fg };
        match *line {
            PageLine::Top(sec) => {
                put(
                    buf,
                    page,
                    bx,
                    y,
                    &format!("╭{}╮", "─".repeat(bw as usize - 2)),
                    fg(border),
                );
                let title = if sec == current {
                    bold(t.bright)
                } else {
                    fg(t.light)
                };
                put(
                    buf,
                    page,
                    bx + 2,
                    y,
                    &format!(" {} ", SECTIONS[sec].to_uppercase()),
                    title,
                );
                continue;
            }
            PageLine::Bottom(_) => {
                put(
                    buf,
                    page,
                    bx,
                    y,
                    &format!("╰{}╯", "─".repeat(bw as usize - 2)),
                    fg(border),
                );
                continue;
            }
            _ => {
                put(buf, page, bx, y, "│", fg(border));
                put(buf, page, br, y, "│", fg(border));
            }
        }
        let inside = Rect::new(bx + 1, y, bw - 2, 1);
        match *line {
            PageLine::Row(i) => {
                draw_settings_row(buf, inside, s, &rows[i], i, label_x, value_x, t, hit)
            }
            PageLine::Grid(i, g) => {
                let Value::Pick {
                    options, chosen, ..
                } = &rows[i].value
                else {
                    continue;
                };
                for c in 0..per {
                    let n = g * per + c;
                    let Some(name) = options.get(n) else { break };
                    let cx = grid_x + c as u16 * THEME_W;
                    let name = fit(name, THEME_W - 4);
                    if n == *chosen {
                        put(buf, inside, cx, y, "►", bold(t.accent));
                        let end = put(buf, inside, cx + 2, y, &name, bold(t.bright));
                        put(buf, inside, end + 1, y, "◄", bold(t.accent));
                    } else {
                        put(buf, inside, cx + 2, y, &name, fg(t.light));
                    }
                    hit(
                        Rect::new(cx, y, THEME_W - 1, 1),
                        Target::SettingsChoice(i, n),
                    );
                }
            }
            _ => {}
        }
    }

    let pairs: &[(&str, &str)] = if s.adding.is_some() {
        &[
            ("type", "a folder, e.g. ~/Music"),
            ("⏎", "add"),
            ("Esc", "cancel"),
        ]
    } else if s.finding {
        &[
            ("type", "to find a setting"),
            ("⏎ ↓", "results"),
            ("Esc", "clear"),
        ]
    } else if !s.find.is_empty() {
        &[
            ("↑↓", "move"),
            ("←→", "change"),
            ("⏎", "press"),
            ("/", "find"),
            ("Esc", "clear"),
        ]
    } else {
        &[
            ("↑↓", "move"),
            ("←→", "change"),
            ("⏎", "press"),
            ("Tab", "section"),
            ("/", "find"),
            ("Esc", "close"),
        ]
    };
    hints(buf, inner, inner.x + 1, inner.bottom() - 1, pairs, t);
}

/// One setting inside its section's box: the label, the value (choices as pills), a note, and a
/// button on the right.
#[allow(clippy::too_many_arguments)]
fn draw_settings_row(
    buf: &mut Buffer,
    inside: Rect,
    s: &Settings,
    row: &settings::Row,
    i: usize,
    label_x: u16,
    value_x: u16,
    t: &Theme,
    hit: &mut impl FnMut(Rect, Target),
) {
    let y = inside.y;
    let right = inside.right().saturating_sub(2);
    hit(inside, Target::SettingsRow(i));
    // The selected row sits darker, so a chosen pill on it still shows.
    let selected = i == s.sel && !s.finding;
    if selected {
        fill(buf, inside, Style::new().bg(t.darker));
        put(buf, inside, label_x - 2, y, "›", bold(t.accent));
    }
    let label = if selected {
        bold(t.bright)
    } else {
        fg(t.light)
    };
    put(buf, inside, label_x, y, row.label, label);
    let button_w = row.button.map_or(0, |b| width(b) + 4);
    let room = |x: u16| right.saturating_sub(x + button_w + 1);
    let mut x = value_x;
    match &row.value {
        Value::Choice(options, chosen) => {
            for (n, option) in options.iter().enumerate() {
                let style = if n == *chosen {
                    bold(t.bright).bg(t.selection)
                } else {
                    fg(t.fg)
                };
                let start = x;
                x = put(buf, inside, x, y, &format!(" {option} "), style);
                hit(
                    Rect::new(start, y, x - start, 1),
                    Target::SettingsChoice(i, n),
                );
                x += 1;
            }
        }
        Value::Number(n) => {
            x = put(buf, inside, x, y, &format!("◂ {n} ▸"), bold(t.bright));
        }
        Value::Pick {
            options, chosen, ..
        } => {
            x = put(buf, inside, x, y, "● ", fg(t.accent));
            let name = options.get(*chosen).map_or("", String::as_str);
            x = put(buf, inside, x, y, name, bold(t.bright));
        }
        Value::Text(text) => {
            if row.id == settings::Id::AddFolder {
                if let Some(typed) = &s.adding {
                    field(buf, Rect::new(x, y, room(x).min(50), 1), typed, true, t);
                }
            } else if row.id == settings::Id::Otis {
                let style = bold(t.accent).add_modifier(Modifier::UNDERLINED);
                let end = put(buf, inside, x, y, text, style);
                hit(
                    Rect::new(x, y, end - x, 1),
                    Target::Link(crate::connect::Link::Otis),
                );
                x = end;
            } else {
                x = put(buf, inside, x, y, &fit(text, room(x)), fg(t.fg));
            }
        }
    }
    // How the source is doing: a coloured dot and a word.
    if let Some(status) = &row.status
        && room(x + 2) > width(&status.word) + 2
    {
        let dot = put(
            buf,
            inside,
            x + 2,
            y,
            "● ",
            fg(health_color(status.health, t)),
        );
        x = put(buf, inside, dot, y, &status.word, fg(t.light));
    }
    if !row.note.is_empty() && room(x + 2) > 8 {
        let style = fg(t.dim).add_modifier(Modifier::ITALIC);
        put(buf, inside, x + 2, y, &fit(&row.note, room(x + 2)), style);
    }
    if let Some(label) = row.button {
        let area = pill(buf, inside, right + 1 - button_w, y, label, t);
        hit(area, Target::SettingsButton(i));
    }
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
