//! The browser panel (OTIS tab "Browse"): the search box, toggles and grouped results on the
//! left, the selected result's songs on the right, both on the theme's lighter background.

use super::*;
use crate::browse::{Browse, Kind, Row};

pub(super) fn draw(
    buf: &mut Buffer,
    r: Rect,
    app: &App,
    player: &Player,
    t: &Theme,
    hit: &mut impl FnMut(Rect, Target),
) {
    let block = pane("BROWSE", t);
    let inner = block.inner(r);
    block.render(r, buf);
    if inner.height == 0 || inner.width < 10 {
        return;
    }
    let b = &app.browse;
    // Side by side when there's room; otherwise only the left panel.
    let both = inner.width >= 90;
    let left_w = if both {
        (inner.width * 2 / 5).max(50)
    } else {
        inner.width - 2
    };
    let left = Rect::new(inner.x + 1, inner.y, left_w, inner.height);
    fill(buf, left, Style::new().bg(t.panel));
    draw_search(buf, left, b, t, hit);
    if both {
        let x = left.right() + 1;
        let right = Rect::new(x, inner.y, inner.right() - 1 - x, inner.height);
        fill(buf, right, Style::new().bg(t.panel));
        draw_songs(buf, right, b, player, t, hit);
    }
}

/// Search box, Artist | Album | Song toggles, then the results.
fn draw_search(
    buf: &mut Buffer,
    p: Rect,
    b: &Browse,
    t: &Theme,
    hit: &mut impl FnMut(Rect, Target),
) {
    let typing = b.pane == Pane::Search;
    let field = Rect::new(p.x + 1, p.y, p.width.saturating_sub(2), 3);
    if p.height >= 3 && field.width > 8 {
        let border = if typing { t.accent } else { t.selection };
        Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(fg(border))
            .render(field, buf);
        hit(field, Target::BrowseSearch);
        let y = field.y + 1;
        let x = put(buf, field, field.x + 2, y, I_SEARCH, fg(t.fg)) + 2;
        let room = field.right().saturating_sub(x + 2);
        if b.query.is_empty() && !typing {
            let style = fg(t.dim).add_modifier(Modifier::ITALIC);
            put(
                buf,
                field,
                x,
                y,
                &fit("Artists, albums, songs", room),
                style,
            );
        } else {
            // The end of a long search, so what's being typed stays in view.
            let mut shown: &str = &b.query;
            while width(shown) > room {
                let mut chars = shown.chars();
                chars.next();
                shown = chars.as_str();
            }
            let x = put(
                buf,
                field,
                x,
                y,
                shown,
                fg(t.bright).add_modifier(Modifier::BOLD),
            );
            if typing {
                put(buf, field, x, y, "▏", fg(t.accent));
            }
        }
    }

    let y = p.y + 4;
    let mut x = p.x + 2;
    for kind in Kind::ALL {
        let label = kind.label();
        let start = x;
        if b.show[kind as usize] {
            x = put(buf, p, x, y, "▐", fg(t.selection));
            let on = fg(t.bright).bg(t.selection).add_modifier(Modifier::BOLD);
            x = put(buf, p, x, y, &format!(" {label} "), on);
            x = put(buf, p, x, y, "▌", fg(t.selection));
        } else {
            x = put(buf, p, x, y, &format!("  {label}  "), fg(t.dim));
        }
        hit(Rect::new(start, y, x - start, 1), Target::Toggle(kind));
        x += 1;
    }
    let note = "1 2 3 toggle";
    if x + 2 + width(note) <= p.right().saturating_sub(2) {
        put_right(buf, p, p.right() - 3, y, note, fg(t.dim));
    }

    let list = Rect::new(p.x, p.y + 6, p.width, p.height.saturating_sub(6));
    hit(list, Target::Results);
    draw_results(buf, list, b, t, hit);
}

fn draw_results(
    buf: &mut Buffer,
    list: Rect,
    b: &Browse,
    t: &Theme,
    hit: &mut impl FnMut(Rect, Target),
) {
    let rows = list.height as usize;
    if rows == 0 {
        return;
    }
    let mut top = scroll(b.results_top.get(), b.sel, rows, b.rows.len());
    // Show a group's heading along with its first row.
    if top == b.sel && top > 0 && rows > 1 && matches!(b.rows[top - 1], Row::Heading { .. }) {
        top -= 1;
    }
    if b.rows.iter().position(Row::selectable) == Some(b.sel) && b.sel < rows {
        top = 0;
    }
    b.results_top.set(top);

    let focused = b.pane == Pane::Results;
    let (heading_x, x) = (list.x + 2, list.x + 3);
    let col2 = list.x + 29;
    let end = list.right().saturating_sub(3);
    let two = |buf: &mut Buffer, y: u16, first: &str, second: &str, style: Style| {
        put(
            buf,
            list,
            x,
            y,
            &fit(first, col2.saturating_sub(x + 2)),
            style,
        );
        let w = (end + 1).saturating_sub(col2);
        put(buf, list, col2, y, &fit(second, w), fg(t.fg));
    };

    for (i, row) in b.rows.iter().enumerate().skip(top).take(rows) {
        let y = list.y + (i - top) as u16;
        if row.selectable() {
            hit(Rect::new(list.x, y, list.width, 1), Target::Result(i));
        }
        let selected = i == b.sel && row.selectable();
        if selected && focused {
            let bar = Rect::new(list.x + 1, y, list.width.saturating_sub(2), 1);
            fill(buf, bar, Style::new().bg(t.selection));
        }
        let name = match (selected, focused) {
            (true, true) => fg(t.bright).add_modifier(Modifier::BOLD),
            (true, false) => fg(t.bright),
            _ => fg(t.light),
        };
        match row {
            Row::Heading { kind, shown, total } => {
                let style = fg(t.dim).add_modifier(Modifier::BOLD);
                let after = put(buf, list, heading_x, y, kind.heading(), style);
                let count = match (*shown, *total) {
                    (_, 0) => String::new(),
                    (s, t) if s == t => group_thousands(t),
                    (s, t) => format!("{s} of {}", group_thousands(t)),
                };
                put(buf, list, after + 2, y, &count, fg(t.dim));
            }
            Row::Note(note) => {
                put(
                    buf,
                    list,
                    x,
                    y,
                    note,
                    fg(t.dim).add_modifier(Modifier::ITALIC),
                );
            }
            Row::Gap => {}
            Row::Artist(n) => {
                let w = (end + 1).saturating_sub(x);
                put(buf, list, x, y, &fit(&b.artists[*n], w), name);
            }
            Row::Album(n) => {
                let album = &b.albums[*n];
                two(
                    buf,
                    y,
                    &album_year(&album.title, album.year),
                    &album.artist,
                    name,
                );
            }
            Row::Song(n) => {
                let song = &b.songs[*n];
                two(buf, y, &song.title, &song.artist, name);
            }
        }
    }
}

/// The selected artist's or album's songs.
fn draw_songs(
    buf: &mut Buffer,
    p: Rect,
    b: &Browse,
    player: &Player,
    t: &Theme,
    hit: &mut impl FnMut(Rect, Target),
) {
    hit(p, Target::Songs);
    let (x0, x1) = (p.x + 2, p.right().saturating_sub(3));
    let label = fg(t.dim).add_modifier(Modifier::BOLD);
    let after = put(buf, p, x0, p.y + 1, "SONGS", label);
    if b.tracks.is_empty() {
        let style = fg(t.dim).add_modifier(Modifier::ITALIC);
        put(buf, p, x0, p.y + 3, "Pick an artist, album or song", style);
        return;
    }
    let about = format!("  {} · {}", b.tracks_title, group_thousands(b.tracks.len()));
    put(
        buf,
        p,
        after,
        p.y + 1,
        &fit(&about, (x1 + 1).saturating_sub(after)),
        fg(t.dim),
    );

    // Columns, as in the design: # 0-1, title 4, album or artist about halfway, format and
    // time on the right.
    let title_x = x0 + 4;
    let format_x = x1.saturating_sub(13);
    let col2 = title_x + format_x.saturating_sub(title_x) * 23 / 39;
    let y = p.y + 3;
    let head = fg(t.dim).add_modifier(Modifier::BOLD);
    put_right(buf, p, x0 + 1, y, "#", head);
    put(buf, p, title_x, y, "TITLE", head);
    let second = if b.tracks_by_artist {
        "ALBUM"
    } else {
        "ARTIST"
    };
    put(buf, p, col2, y, second, head);
    put(buf, p, format_x, y, "FORMAT", head);
    put_right(buf, p, x1, y, "TIME", head);

    let first = p.y + 4;
    let rows = p.bottom().saturating_sub(1).saturating_sub(first) as usize;
    let top = scroll(b.tracks_top.get(), b.track_sel, rows, b.tracks.len());
    b.tracks_top.set(top);
    let focused = b.pane == Pane::Songs;
    let playing = player
        .current()
        .filter(|t| t.source == Source::Local)
        .map(|t| t.id);

    for (i, track) in b.tracks.iter().enumerate().skip(top).take(rows) {
        let y = first + (i - top) as u16;
        hit(Rect::new(p.x, y, p.width, 1), Target::Song(i));
        let selected = i == b.track_sel;
        if selected && focused {
            let bar = Rect::new(p.x + 1, y, p.width.saturating_sub(2), 1);
            fill(buf, bar, Style::new().bg(t.selection));
        }
        if Some(track.id) == playing {
            put(
                buf,
                p,
                x0 + 1,
                y,
                "▶",
                fg(t.accent).add_modifier(Modifier::BOLD),
            );
        } else {
            let number = track.track_no.map(|n| n.to_string()).unwrap_or_default();
            put_right(buf, p, x0 + 1, y, &number, fg(t.dim));
        }
        let title = match (selected, focused) {
            (true, true) => fg(t.bright).add_modifier(Modifier::BOLD),
            (true, false) => fg(t.bright),
            _ => fg(t.light),
        };
        put(
            buf,
            p,
            title_x,
            y,
            &fit(&track.title, col2.saturating_sub(title_x + 2)),
            title,
        );
        let other = if b.tracks_by_artist {
            album_year(&track.album, track.year)
        } else {
            track.artist.clone()
        };
        put(
            buf,
            p,
            col2,
            y,
            &fit(&other, format_x.saturating_sub(col2 + 2)),
            fg(t.fg),
        );
        let format = short_format(track.bit_depth, track.sample_rate);
        let color = if hi_res(track) { t.light } else { t.fg };
        put(buf, p, format_x, y, &format, fg(color));
        put_right(
            buf,
            p,
            x1,
            y,
            &clock(track.duration_ms as f64 / 1000.0),
            fg(t.dim),
        );
    }
}
