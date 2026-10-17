//! Draws the screen from `App` and the player's status, following the OTIS design (project
//! "Omaestro", tabs Main and Browse): the logo and controls on top, Now Playing and the
//! visualizer, the queue, and the browser sliding up over the bottom of the queue.
//!
//! The design is a 120×60 grid. Positions here are the same offsets from each pane's edge, so
//! the screen matches it at that size and stretches or drops panes at others.
//!
//! Drawing also lists where every clickable thing ended up (`Hit`s), so a click can be matched
//! to what's under it.

mod browse;
mod screens;

use crate::app::{App, Hit, OutputChoice, Overlay, Target, group_thousands};
use crate::browse::Pane;
use crate::columns::Column;
use crate::library::{Source, Track};
use crate::player::{Player, Quality, Status};
use crate::theme::Theme;
use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Widget};
use std::time::SystemTime;

// Nerd Font icons, as in the design.
// Button icons, measured in GeistMono Nerd Font (foot's font here). A terminal can only move
// things by whole cells, so an icon sits in the middle of a button only if it's centred in its
// own cell. These Material Design ones are under a cell wide and centred in it. Font Awesome's
// spill about half a cell into the next one, which pushed them off centre. No gear in the font
// is that narrow; Material's cog is the closest (0.18 of a cell right of centre).
const I_STOP: &str = "\u{f04db}";
const I_PREV: &str = "\u{f04ae}";
const I_PLAY: &str = "\u{f040a}";
const I_PAUSE: &str = "\u{f03e4}";
const I_NEXT: &str = "\u{f04ad}";
const I_GEAR: &str = "\u{f0493}";
const I_HELP: &str = "\u{f02d6}";
const I_HEADPHONES: &str = "\u{f025}";
const I_VOLUME_OFF: &str = "\u{f026}";
const I_VOLUME_LOW: &str = "\u{f027}";
const I_VOLUME: &str = "\u{f028}";
const I_LOCAL: &str = "\u{f0a0}";
const I_SPOTIFY: &str = "\u{f1bc}";
const I_YOUTUBE: &str = "\u{f16a}";
const I_MUSIC: &str = "\u{f001}";
const I_SEARCH: &str = "\u{f002}";
const I_LIST: &str = "\u{f03a}";
const I_NEW: &str = "\u{f016}";
const I_SAVE: &str = "\u{f0c7}";
const I_OPEN: &str = "\u{f07c}";
const I_CLEAR: &str = "\u{f1f8}";
const I_STAR: &str = "\u{f005}";
const I_STAR_O: &str = "\u{f006}";
const I_TV: &str = "\u{f26c}";
const I_WIFI: &str = "\u{f1eb}";

/// The logo: headphones over the first O. Each row's characters, then each character's colour
/// (M dark_foreground, B bright_foreground, L light_foreground, F foreground).
const LOGO: [(&str, &str); 4] = [
    (" ⣠⣴⠶⠶⣦⣄", " MMMMMM"),
    (
        "⣸⡅▄▀▀▄⢨⣇ █▄ ▄█ ▄▀▀▄ █▀▀▀ ▄▀▀▀ ▀▀█▀▀ █▀▀▄ ▄▀▀▄",
        "MMBBBBMM LL LL LLLL LLLL LLLL LLLLL LLLL LLLL",
    ),
    (
        "⢿⡇█  █⢸⡿ █ ▀ █ █▀▀█ █▀▀   ▀▀▄   █   █▀█  █  █",
        "MMB  BMM F F F FFFF FFF   FFF   F   FFF  F  F",
    ),
    (
        "   ▀▀    ▀   ▀ ▀  ▀ ▀▀▀▀ ▀▀▀    ▀   ▀  ▀  ▀▀",
        "   BB    M   M M  M MMMM MMM    M   M  M  MM",
    ),
];
const LOGO_W: u16 = 45;

/// Volume, output, settings and help, with a column between each.
const CONTROLS_W: u16 = VOLUME_W + 1 + OUTPUT_W + 1 + ICON_BUTTON_W + 1 + ICON_BUTTON_W;
const VOLUME_W: u16 = 27;
/// The volume slider's length inside its box.
const VOLUME_SLIDER: u16 = 16;
const OUTPUT_W: u16 = 22;
/// A button with just an icon: three cells inside, the icon in the middle one. About as wide
/// as it is tall on screen.
const ICON_BUTTON_W: u16 = 5;

/// Where everything goes on a screen this size.
struct Areas {
    logo: Option<Rect>,
    volume: Option<Rect>,
    output: Option<Rect>,
    settings: Option<Rect>,
    help: Option<Rect>,
    now: Option<Rect>,
    visualizer: Option<Rect>,
    queue: Rect,
    browse: Option<Rect>,
    hints: Option<Rect>,
    footer: Option<Rect>,
}

/// `browse` is how far up the browser is, 0 to 1.
fn layout(area: Rect, browse: f32) -> Areas {
    let margin = if area.width >= 60 { 2 } else { 0 };
    let x = area.x + margin;
    let w = area.width.saturating_sub(2 * margin);
    let mut top = area.y;
    let mut bottom = area.bottom();
    let mut a = Areas {
        logo: None,
        volume: None,
        output: None,
        settings: None,
        help: None,
        now: None,
        visualizer: None,
        queue: Rect::default(),
        browse: None,
        hints: None,
        footer: None,
    };

    if area.height >= 3 {
        bottom -= 1;
        a.footer = Some(Rect::new(x, bottom, w, 1));
    }
    if area.height >= 8 {
        bottom -= 1;
        a.hints = Some(Rect::new(x, bottom, w, 1));
    }
    let header = area.height >= 34 && w >= CONTROLS_W;
    if header {
        let help = Rect::new(x + w - ICON_BUTTON_W, top, ICON_BUTTON_W, 3);
        let settings = Rect::new(help.x - ICON_BUTTON_W - 1, top, ICON_BUTTON_W, 3);
        let output = Rect::new(settings.x - OUTPUT_W - 1, top, OUTPUT_W, 3);
        a.volume = Some(Rect::new(output.x - VOLUME_W - 1, top, VOLUME_W, 3));
        a.output = Some(output);
        a.settings = Some(settings);
        a.help = Some(help);
        if w >= LOGO_W + 2 + CONTROLS_W {
            a.logo = Some(Rect::new(x, top, LOGO_W, 4));
        }
        top += 4;
    }

    let left = bottom.saturating_sub(top);
    let now_h = if left >= 26 {
        11
    } else if left >= 12 {
        5
    } else {
        0
    };
    if now_h > 0 {
        if w >= 100 {
            a.now = Some(Rect::new(x, top, 64, now_h));
            a.visualizer = Some(Rect::new(x + 66, top, w - 66, now_h));
        } else {
            a.now = Some(Rect::new(x, top, w, now_h));
        }
        top += now_h + u16::from(header);
    }

    let lower = Rect::new(x, top, w, bottom.saturating_sub(top));
    a.queue = lower;
    if browse > 0.0 && lower.height > 0 {
        // About half of the space under Now Playing, leaving the queue a few rows.
        let target = if lower.height < 17 {
            lower.height
        } else {
            (lower.height * 53 / 100).max(12).min(lower.height - 5)
        };
        let h = ((target as f32 * browse).round() as u16).min(lower.height);
        if h > 0 {
            a.browse = Some(Rect::new(x, lower.bottom() - h, w, h));
            a.queue = Rect::new(x, top, w, lower.height - h);
        }
    }
    a
}

/// Draws one frame and returns where the clickable things are. `browse` is how far up the
/// browser is, 0 to 1 (it slides).
pub fn draw(frame: &mut Frame, app: &App, player: &Player, browse: f32) -> Vec<Hit> {
    let t = &app.theme;
    let area = frame.area();
    let a = layout(area, browse);
    let buf = frame.buffer_mut();
    // A theme picked in Settings brings its own background; Omarchy's is the terminal's.
    if app.theme_fixed {
        fill(buf, area, Style::new().bg(t.background).fg(t.fg));
    }
    let mut hits = Vec::new();
    let mut hit = |area: Rect, target: Target| hits.push(Hit { area, target });

    if let Some(r) = a.logo {
        draw_logo(buf, r, t);
    }
    if let Some(r) = a.volume {
        draw_volume(buf, r, player.status.volume, t);
        hit(
            Rect::new(r.x + 4, r.y, VOLUME_SLIDER, r.height),
            Target::Volume,
        );
    }
    if let Some(r) = a.output {
        // The icon on the left; the output's name and the ▾ against the right.
        let (icon, name) = match &player.output {
            crate::player::Output::Sonos(group) => (I_VOLUME, group.name()),
            crate::player::Output::Here(_) => (I_HEADPHONES, device_name(&player.status)),
        };
        button(buf, r, "", t.selection, t.light);
        let y = r.y + 1;
        put(buf, r, r.x + 2, y, icon, fg(t.light));
        let room = r.width.saturating_sub(10);
        let label = format!("{} ▾", fit(&name, room));
        put_right(buf, r, r.right() - 3, y, &label, fg(t.light));
        hit(r, Target::Output);
    }
    if let Some(r) = a.settings {
        button(buf, r, I_GEAR, t.selection, t.light);
        hit(r, Target::Settings);
    }
    if let Some(r) = a.help {
        button(buf, r, I_HELP, t.selection, t.light);
        hit(r, Target::Help);
    }
    if let Some(r) = a.now {
        let cover = app.cover.as_ref().filter(|_| !app.cover_blink);
        draw_now_playing(buf, r, player, cover, t, &mut hit);
    }
    if let Some(r) = a.visualizer {
        draw_visualizer(buf, r, app, player, t);
    }
    draw_queue(buf, a.queue, app, player, t, &mut hit);
    if let Some(r) = a.browse {
        browse::draw(buf, r, app, player, t, &mut hit);
    }
    if let Some(r) = a.hints {
        draw_hints(buf, r, app, t);
    }
    if let Some(r) = a.footer {
        draw_footer(buf, r, app, t, &mut hit);
    }
    match &app.overlay {
        Some(Overlay::Help) => draw_help(buf, area, t, &mut hit),
        Some(Overlay::FileInfo(info)) => screens::draw_file_info(buf, area, info, t, &mut hit),
        Some(Overlay::LastFm(step)) => screens::draw_lastfm(buf, area, step, t, &mut hit),
        Some(Overlay::ListenBrainz(step)) => {
            screens::draw_listenbrainz(buf, area, step, t, &mut hit)
        }
        Some(Overlay::Columns(sel)) => draw_columns_menu(buf, area, app, *sel, t, &mut hit),
        Some(Overlay::Picker { sel, all }) => {
            draw_picker(buf, area, a.output, app, player, *sel, *all, t, &mut hit)
        }
        // Renaming happens in the queue's header.
        Some(Overlay::Rename { .. }) => {}
        Some(Overlay::Open(open)) => screens::draw_open(buf, area, open, t, &mut hit),
        Some(Overlay::Search) => {
            screens::draw_search(buf, area, &app.search, player, &app.statuses, t, &mut hit)
        }
        Some(Overlay::Settings(s)) => {
            screens::draw_settings(buf, area, s, &app.config, &app.facts(), t, &mut hit)
        }
        Some(Overlay::Spotify(step)) => screens::draw_spotify(buf, area, step, t, &mut hit),
        Some(Overlay::YouTube(step)) => screens::draw_youtube(buf, area, step, t, &mut hit),
        None => {}
    }
    hits
}

// ---- drawing helpers ---------------------------------------------------------------------

/// Writes `text` from column `x` of row `y`, cut off at the edge of `clip`. Returns the
/// column after the last one written.
fn put(buf: &mut Buffer, clip: Rect, x: u16, y: u16, text: &str, style: Style) -> u16 {
    let clip = clip.intersection(buf.area);
    if y < clip.top() || y >= clip.bottom() || x < clip.left() || x >= clip.right() {
        return x;
    }
    buf.set_stringn(x, y, text, (clip.right() - x) as usize, style)
        .0
}

/// Like `put`, with the text's last column at `right`.
fn put_right(buf: &mut Buffer, clip: Rect, right: u16, y: u16, text: &str, style: Style) {
    let x = (right + 1).saturating_sub(width(text));
    put(buf, clip, x, y, text, style);
}

fn fill(buf: &mut Buffer, r: Rect, style: Style) {
    buf.set_style(r.intersection(buf.area), style);
}

fn width(s: &str) -> u16 {
    Line::from(s).width() as u16
}

/// `s` cut to `w` columns, ending in "…" when something was cut.
fn fit(s: &str, w: u16) -> String {
    if width(s) <= w {
        return s.to_string();
    }
    let mut out = String::new();
    let mut used = 0;
    for c in s.chars() {
        let cw = width(c.encode_utf8(&mut [0; 4]));
        if used + cw + 1 > w {
            break;
        }
        out.push(c);
        used += cw;
    }
    if w > 0 {
        out.push('…');
    }
    out
}

fn fg(color: Color) -> Style {
    Style::new().fg(color)
}

fn bold(color: Color) -> Style {
    fg(color).add_modifier(Modifier::BOLD)
}

/// A pane with a rounded border and its title in the top edge: "╭─ QUEUE ───".
fn pane<'a>(title: &'a str, t: &Theme) -> Block<'a> {
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(fg(t.selection))
        .title(Line::from(vec![
            Span::styled("─", fg(t.selection)),
            Span::styled(format!(" {title} "), fg(t.dim)),
        ]))
}

/// A rounded box with a centred label, the design's button.
fn button(buf: &mut Buffer, r: Rect, label: &str, border: Color, color: Color) {
    let r = r.intersection(buf.area);
    if r.width < 3 || r.height < 3 {
        return;
    }
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(fg(border))
        .render(r, buf);
    let inner = Rect::new(r.x + 1, r.y + 1, r.width - 2, 1);
    let x = inner.x + inner.width.saturating_sub(width(label)) / 2;
    put(buf, inner, x, inner.y, label, fg(color));
}

/// A small filled label, "▐ Save ▌", as in the queue header. Returns its area.
fn pill(buf: &mut Buffer, clip: Rect, x: u16, y: u16, label: &str, t: &Theme) -> Rect {
    let after = put(buf, clip, x, y, "▐", fg(t.selection));
    let after = put(
        buf,
        clip,
        after,
        y,
        &format!(" {label} "),
        fg(t.light).bg(t.selection),
    );
    let end = put(buf, clip, after, y, "▌", fg(t.selection));
    Rect::new(x, y, end - x, 1)
}

/// The first row to show so that `sel` is on screen, moving as little as possible.
fn scroll(top: usize, sel: usize, rows: usize, len: usize) -> usize {
    if rows == 0 {
        return 0;
    }
    let top = top.min(len.saturating_sub(rows));
    if sel < top {
        sel
    } else if sel >= top + rows {
        sel + 1 - rows
    } else {
        top
    }
}

/// 225.4 → "3:45".
fn clock(seconds: f64) -> String {
    let s = seconds.max(0.0) as u64;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

/// 24 bits at 96000 → "24b-96k", as in the design's FORMAT column.
fn short_format(bits: Option<u8>, rate: Option<u32>) -> String {
    match (bits, rate) {
        (Some(b), Some(r)) => format!("{b}b-{}k", r / 1000),
        (None, Some(r)) => format!("{}k", r / 1000),
        _ => String::new(),
    }
}

/// What the FORMAT column says about a song.
fn format_of(t: &Track) -> String {
    match t.source {
        Source::Local => short_format(t.bit_depth, t.sample_rate),
        Source::Spotify => "Spotify".into(),
        Source::YouTube => "YouTube".into(),
    }
}

/// "Brothers (2010)", or just the album when the year isn't known.
fn album_year(album: &str, year: Option<i32>) -> String {
    match year {
        Some(y) if !album.is_empty() => format!("{album} ({y})"),
        _ => album.to_string(),
    }
}

/// More than CD quality: shown a little brighter.
fn hi_res(t: &Track) -> bool {
    t.bit_depth.is_some_and(|b| b > 16) || t.sample_rate.is_some_and(|r| r > 48000)
}

/// The source's icon and colour, for the queue and results.
fn source_icon(source: Source, t: &Theme) -> (&'static str, Color) {
    match source {
        Source::Local => (I_LOCAL, t.light),
        Source::Spotify => (I_SPOTIFY, t.accent),
        Source::YouTube => (I_YOUTUBE, t.red),
    }
}

/// "FLAC" from "/music/…/01 Song.flac".
fn codec(t: &Track) -> String {
    std::path::Path::new(&t.path)
        .extension()
        .map(|e| e.to_string_lossy().to_uppercase())
        .unwrap_or_default()
}

/// The output's short name: "Schiit Modi+" rather than "Schiit Modi+ Analog Stereo".
fn device_name(status: &Status) -> String {
    if status.device.is_empty() || status.device == "pipewire" || status.device == "auto" {
        return "Default".into();
    }
    let name = status
        .devices
        .iter()
        .find(|d| d.name == status.device)
        .map(|d| d.description.as_str())
        .unwrap_or(status.device.as_str());
    [" Analog Stereo", " Digital Stereo", " Stereo"]
        .iter()
        .find_map(|suffix| name.find(suffix).map(|i| &name[..i]))
        .unwrap_or(name)
        .to_string()
}

/// "today", "yesterday", "3 days ago", "5 weeks ago".
fn age(when: SystemTime) -> String {
    let days = SystemTime::now()
        .duration_since(when)
        .map(|d| d.as_secs() / 86_400)
        .unwrap_or(0);
    match days {
        0 => "today".into(),
        1 => "yesterday".into(),
        2..=13 => format!("{days} days ago"),
        14..=60 => format!("{} weeks ago", days / 7),
        _ => format!("{} months ago", days / 30),
    }
}

/// Key hints: keys bright, words dim, stopping before they run off the edge.
fn hints(buf: &mut Buffer, r: Rect, x: u16, y: u16, pairs: &[(&str, &str)], t: &Theme) {
    let mut x = x;
    for (i, (key, word)) in pairs.iter().enumerate() {
        let gap = if i == 0 { 0 } else { 2 };
        if x + gap + width(key) + 1 + width(word) > r.right() {
            break;
        }
        x = put(buf, r, x + gap, y, key, bold(t.light));
        x = put(buf, r, x, y, &format!(" {word}"), fg(t.dim));
    }
}

// ---- the panes ---------------------------------------------------------------------------

fn draw_logo(buf: &mut Buffer, r: Rect, t: &Theme) {
    for (dy, (chars, colours)) in LOGO.iter().enumerate() {
        for (dx, (c, colour)) in chars.chars().zip(colours.chars()).enumerate() {
            if c == ' ' {
                continue;
            }
            let color = match colour {
                'B' => t.bright,
                'L' => t.light,
                'F' => t.fg,
                _ => t.dim,
            };
            let s = c.to_string();
            put(buf, r, r.x + dx as u16, r.y + dy as u16, &s, fg(color));
        }
    }
}

/// "🔊 ━━━━━━━━━●──── 70%" in a rounded box.
fn draw_volume(buf: &mut Buffer, r: Rect, volume: f64, t: &Theme) {
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(fg(t.selection))
        .render(r, buf);
    let (x, y) = (r.x + 2, r.y + 1);
    let icon = if volume <= 0.0 {
        I_VOLUME_OFF
    } else if volume < 50.0 {
        I_VOLUME_LOW
    } else {
        I_VOLUME
    };
    put(buf, r, x, y, icon, fg(t.fg));
    slider(buf, r, x + 2, y, VOLUME_SLIDER, volume / 100.0, t);
    put_right(
        buf,
        r,
        r.right() - 3,
        y,
        &format!("{volume:.0}%"),
        fg(t.dim),
    );
}

/// A line with a knob: "━━━━●────". Used for the volume and the song position.
fn slider(buf: &mut Buffer, clip: Rect, x: u16, y: u16, w: u16, ratio: f64, t: &Theme) {
    if w == 0 {
        return;
    }
    let knob = (ratio.clamp(0.0, 1.0) * (w - 1) as f64).round() as u16;
    for i in 0..w {
        let (c, color) = match i.cmp(&knob) {
            std::cmp::Ordering::Less => ("━", t.fg),
            std::cmp::Ordering::Equal => ("●", t.bright),
            std::cmp::Ordering::Greater => ("─", t.selection),
        };
        put(buf, clip, x + i, y, c, fg(color));
    }
}

fn draw_now_playing(
    buf: &mut Buffer,
    r: Rect,
    player: &Player,
    cover: Option<&ratatui_image::protocol::Protocol>,
    t: &Theme,
    hit: &mut impl FnMut(Rect, Target),
) {
    let block = pane("NOW PLAYING", t);
    let inner = block.inner(r);
    block.render(r, buf);
    let st = &player.status;
    let track = player.current();
    let playing = !st.idle && !st.paused;
    let duration = match (st.duration, track) {
        (d, _) if d > 0.0 => d,
        (_, Some(track)) => track.duration_ms as f64 / 1000.0,
        _ => 0.0,
    };
    let right = inner.right().saturating_sub(2);

    if inner.height >= 9 {
        // Album art on the left (a placeholder until covers are built), text beside it.
        let art = inner.width >= 19 + 30;
        let x = if art { inner.x + 19 } else { inner.x + 1 };
        let w = (right + 1).saturating_sub(x);
        if art {
            let area = Rect::new(inner.x + 1, inner.y + 1, crate::art::COLS, crate::art::ROWS);
            match cover.filter(|_| track.is_some()) {
                Some(cover) => ratatui_image::Image::new(cover).render(area, buf),
                None => draw_cover_placeholder(buf, area, t),
            }
        }
        match track {
            Some(track) => {
                draw_badge(buf, inner, right, inner.y, track, st, t);
                put(
                    buf,
                    inner,
                    x,
                    inner.y + 1,
                    &fit(&track.title, w),
                    bold(t.bright),
                );
                put(
                    buf,
                    inner,
                    x,
                    inner.y + 2,
                    &fit(&track.artist, w),
                    fg(t.light),
                );
                let album = match track.year {
                    Some(year) => format!("{} · {year}", track.album),
                    None => track.album.clone(),
                };
                put(buf, inner, x, inner.y + 3, &fit(&album, w), fg(t.fg));
            }
            None => {
                put(buf, inner, x, inner.y + 1, "Nothing playing", fg(t.light));
                let tip = "Press b to browse your library";
                put(buf, inner, x, inner.y + 2, tip, fg(t.dim));
            }
        }
        // Four buttons the same size and look.
        let step = ICON_BUTTON_W + 1;
        if w >= 4 * step - 1 {
            let y = inner.y + 5;
            let play = if playing { I_PAUSE } else { I_PLAY };
            let buttons = [
                (I_STOP, Target::Stop),
                (I_PREV, Target::Previous),
                (play, Target::PlayPause),
                (I_NEXT, Target::Next),
            ];
            for (i, (icon, target)) in buttons.into_iter().enumerate() {
                let area = Rect::new(x + i as u16 * step, y, ICON_BUTTON_W, 3);
                button(buf, area, icon, t.selection, t.light);
                hit(area, target);
            }
        }
        if let Some(note) = &st.preparing {
            let style = fg(t.yellow).add_modifier(Modifier::ITALIC);
            put(buf, inner, x, inner.y + 8, &fit(note, w), style);
        } else if track.is_some()
            && let Some(bar) = draw_jogger(
                buf,
                inner,
                x,
                inner.y + 8,
                w.min(40),
                st.position,
                duration,
                t,
            )
        {
            hit(bar, Target::Seek);
        }
    } else {
        // Three rows: title and artist, album and format, position.
        let x = inner.x + 1;
        let w = (right + 1).saturating_sub(x);
        let Some(track) = track else {
            let note = "Nothing playing · press b to browse";
            put(buf, inner, x, inner.y, note, fg(t.dim));
            return;
        };
        let icon = if playing { I_PLAY } else { I_PAUSE };
        let after = put(buf, inner, x, inner.y, &format!("{icon}  "), fg(t.accent));
        hit(Rect::new(x, inner.y, 2, 1), Target::PlayPause);
        let after = put(buf, inner, after, inner.y, &track.title, bold(t.bright));
        put(
            buf,
            inner,
            after,
            inner.y,
            &format!("  {}", track.artist),
            fg(t.light),
        );
        put(
            buf,
            inner,
            x,
            inner.y + 1,
            &fit(&track.album, w / 2),
            fg(t.fg),
        );
        draw_badge(buf, inner, right, inner.y + 1, track, st, t);
        if let Some(note) = &st.preparing {
            let style = fg(t.yellow).add_modifier(Modifier::ITALIC);
            put(buf, inner, x, inner.y + 2, &fit(note, w), style);
        } else if let Some(bar) =
            draw_jogger(buf, inner, x, inner.y + 2, w, st.position, duration, t)
        {
            hit(bar, Target::Seek);
        }
    }
}

/// What a stream is coming in as: "LOSSLESS 16b-44k", "OPUS 141 kbps", "AAC 256 kbps".
fn quality_label(q: &Quality) -> String {
    let format = short_format(q.bit_depth, q.sample_rate);
    let label = if q.lossless {
        format!("LOSSLESS {format}")
    } else if let Some(kbps) = q.kbps {
        format!("{} {kbps} kbps", q.codec)
    } else if !q.codec.is_empty() {
        format!("{} {format}", q.codec)
    } else {
        q.level.clone()
    };
    label.trim().to_string()
}

/// " FLAC 24b-96k " at the pane's top right. When the output runs at another rate (PipeWire
/// resampling), that rate follows in yellow: " → 48k ". For a stream, what it's coming in as
/// (" OPUS 141 kbps "), or nothing until that's known.
fn draw_badge(
    buf: &mut Buffer,
    clip: Rect,
    right: u16,
    y: u16,
    track: &Track,
    st: &Status,
    t: &Theme,
) {
    let main = if track.source == Source::Local {
        let format = short_format(track.bit_depth, track.sample_rate);
        format!(" {} ", [codec(track), format].join(" ").trim())
    } else {
        match st.quality.as_ref().map(quality_label) {
            Some(label) if !label.is_empty() => format!(" {label} "),
            _ => return,
        }
    };
    let resampled = match (track.sample_rate, st.sample_rate) {
        (Some(file), Some(out)) if file != out && !st.idle && track.source == Source::Local => {
            format!("→ {}k ", out / 1000)
        }
        _ => String::new(),
    };
    let x = (right + 1).saturating_sub(width(&main) + width(&resampled));
    let x = put(buf, clip, x, y, &main, fg(t.light).bg(t.selection));
    put(buf, clip, x, y, &resampled, fg(t.yellow).bg(t.selection));
}

/// "1:18 ━━━━━━●──────── 2:44". Returns where the bar is.
#[allow(clippy::too_many_arguments)]
fn draw_jogger(
    buf: &mut Buffer,
    clip: Rect,
    x: u16,
    y: u16,
    w: u16,
    pos: f64,
    dur: f64,
    t: &Theme,
) -> Option<Rect> {
    let (left, right) = (clock(pos), clock(dur));
    let (lw, rw) = (width(&left), width(&right));
    if w < lw + rw + 6 {
        return None;
    }
    put(buf, clip, x, y, &left, fg(t.dim));
    let bar = w - lw - rw - 2;
    let ratio = if dur > 0.0 { pos / dur } else { 0.0 };
    slider(buf, clip, x + lw + 1, y, bar, ratio, t);
    put(buf, clip, x + lw + bar + 2, y, &right, fg(t.dim));
    Some(Rect::new(x + lw + 1, y, bar, 1))
}

fn draw_cover_placeholder(buf: &mut Buffer, r: Rect, t: &Theme) {
    fill(buf, r, Style::new().bg(t.panel));
    let (x, y) = (r.x + r.width / 2, r.y + r.height / 2);
    put(buf, r, x, y, I_MUSIC, fg(t.selection));
}

/// The animation the visualizer shows instead of bars, when there's no sound to read: Spotify
/// playing on Sonos, which streams it straight from Spotify.
pub fn fill_in(app: &App, player: &Player) -> Option<crate::animation::Kind> {
    let kind = app.config.appearance.animation;
    let st = &player.status;
    (app.config.appearance.visualizer
        && kind != crate::animation::Kind::Off
        && app.bands.is_empty()
        && !st.idle
        && !st.paused
        && matches!(player.output, crate::player::Output::Sonos(_))
        && player
            .current()
            .is_some_and(|t| t.source == crate::library::Source::Spotify))
    .then_some(kind)
}

fn draw_visualizer(buf: &mut Buffer, r: Rect, app: &App, player: &Player, t: &Theme) {
    let block = pane("VISUALIZER", t);
    let inner = block.inner(r);
    block.render(r, buf);
    if !app.bands.is_empty() {
        draw_bars(buf, inner, &app.bands, t);
        return;
    }
    if let Some(kind) = fill_in(app, player) {
        draw_animation(
            buf,
            inner,
            kind,
            app.animation_clock.elapsed().as_secs_f64(),
            t,
        );
        return;
    }
    let note = if !app.config.appearance.visualizer {
        "Off (Settings → Appearance)"
    } else if matches!(player.output, crate::player::Output::Sonos(_))
        && player
            .current()
            .is_some_and(|t| t.source == crate::library::Source::Spotify)
    {
        "Spotify plays on the speaker itself: nothing to show"
    } else if matches!(player.output, crate::player::Output::Sonos(_)) {
        "Playing on Sonos"
    } else {
        ""
    };
    let x = inner.x + inner.width.saturating_sub(width(note)) / 2;
    put(
        buf,
        inner,
        x,
        inner.y + inner.height / 2,
        note,
        fg(t.selection),
    );
}

/// A frame of an animation, its brightness turned into the theme's colours, faint to bright.
fn draw_animation(
    buf: &mut Buffer,
    area: Rect,
    kind: crate::animation::Kind,
    secs: f64,
    t: &Theme,
) {
    let ramp = [t.selection, t.dim, t.fg, t.accent, t.light, t.bright];
    let frame = crate::animation::frame(kind, area.width as usize, area.height as usize, secs);
    for (y, row) in frame.iter().enumerate() {
        for (x, cell) in row.iter().enumerate() {
            if cell.ch == ' ' {
                continue;
            }
            let i = ((cell.level * (ramp.len() - 1) as f32).round() as usize).min(ramp.len() - 1);
            let at = (area.x + x as u16, area.y + y as u16);
            put(buf, area, at.0, at.1, &cell.ch.to_string(), fg(ramp[i]));
        }
    }
}

/// Bars drawn with braille dots, as in the design: each bar two dots wide with a one-dot gap,
/// four dots to a row, so they rise smoothly.
fn draw_bars(buf: &mut Buffer, area: Rect, bands: &[f32], t: &Theme) {
    // Braille dot bits: [left column, right column] from the top row down.
    const DOTS: [[u32; 4]; 2] = [[0x01, 0x02, 0x04, 0x40], [0x08, 0x10, 0x20, 0x80]];
    let (w, h) = (area.width as usize, area.height as usize);
    let bars = (w * 2) / 3;
    if bars == 0 || h == 0 || bands.is_empty() {
        return;
    }
    let tall = h * 4;
    let heights: Vec<usize> = (0..bars)
        .map(|b| (bands[b * bands.len() / bars].clamp(0.0, 1.0) * tall as f32).round() as usize)
        .collect();
    for cy in 0..h {
        // Lighter towards the top.
        let color = match cy * 3 / h {
            0 => t.accent,
            1 => t.light,
            _ => t.fg,
        };
        for cx in 0..w {
            let mut bits = 0;
            for (dx, column) in DOTS.iter().enumerate() {
                let x = cx * 2 + dx;
                if x % 3 == 2 || x / 3 >= bars {
                    continue;
                }
                for (dy, bit) in column.iter().enumerate() {
                    let from_bottom = (h - 1 - cy) * 4 + (3 - dy);
                    if from_bottom < heights[x / 3] {
                        bits |= bit;
                    }
                }
            }
            if let Some(c) = char::from_u32(0x2800 + bits).filter(|_| bits != 0) {
                let s = c.to_string();
                put(
                    buf,
                    area,
                    area.x + cx as u16,
                    area.y + cy as u16,
                    &s,
                    fg(color),
                );
            }
        }
    }
}

fn draw_queue(
    buf: &mut Buffer,
    r: Rect,
    app: &App,
    player: &Player,
    t: &Theme,
    hit: &mut impl FnMut(Rect, Target),
) {
    if r.height < 2 {
        return;
    }
    let block = pane("QUEUE", t);
    let inner = block.inner(r);
    block.render(r, buf);
    hit(inner, Target::QueueList);
    let queue = player.queue();
    let current = player.current_index();
    let focused = !app.browse_open && app.overlay.is_none();
    let picked: std::collections::BTreeSet<usize> = app
        .picked(player.revision, queue.len())
        .into_iter()
        .collect();
    let (cx, cr) = (inner.x + 1, inner.right().saturating_sub(2));
    let full = inner.height >= 6;
    let mut y = inner.y;

    if full {
        // The list icon opens the show/hide-columns menu; then the playlist's name (click it to
        // rename it, which happens right here) and its buttons, as in the design.
        let icon = Rect::new(cx, y, 3, 1);
        put(buf, inner, cx, y, &format!("{I_LIST} ▾"), fg(t.fg));
        hit(icon, Target::ColumnsMenu);
        app.columns_anchor.set(Some(icon));
        let x = cx + 5;
        let x = match &app.overlay {
            Some(Overlay::Rename {
                name: typed,
                then_save,
            }) => {
                let w = 30.min(cr.saturating_sub(x + 2));
                screens::field(buf, Rect::new(x, y, w, 1), typed, true, t);
                // Renaming onto another playlist's name replaces it: say so first.
                let path = crate::playlist::file_for(&app.playlist_dir, typed);
                let mine = player.playlist.as_ref().is_some_and(|p| p.path == path);
                let (note, color) = if !typed.trim().is_empty() && path.exists() && !mine {
                    ("⏎ replaces the playlist with that name", t.yellow)
                } else if *then_save {
                    ("⏎ save · Esc cancel", t.dim)
                } else {
                    ("⏎ rename · Esc cancel", t.dim)
                };
                put(buf, inner, x + w + 2, y, note, fg(color))
            }
            _ => {
                let name = player
                    .playlist
                    .as_ref()
                    .map_or("Untitled", |p| p.name.as_str());
                let end = put(buf, inner, x, y, &fit(name, 30), bold(t.bright));
                hit(Rect::new(x, y, end - x, 1), Target::PlaylistName);
                let unsaved = !queue.is_empty()
                    && player
                        .playlist
                        .as_ref()
                        .is_none_or(|p| p.saved != player.revision);
                if unsaved {
                    put(buf, inner, end, y, "  ● unsaved", fg(t.orange))
                } else {
                    end
                }
            }
        };
        let mut x = x.max(cx + 26) + 4;
        let summary = if queue.is_empty() {
            String::new()
        } else {
            let total: i64 = queue.iter().map(|t| t.duration_ms).sum();
            let tracks = if queue.len() == 1 { "track" } else { "tracks" };
            let all = format!(
                "{} {tracks} · {}",
                queue.len(),
                clock(total as f64 / 1000.0)
            );
            match picked.len() {
                0 => all,
                n => format!("{n} selected · {all}"),
            }
        };
        let room = cr.saturating_sub(width(&summary) + 2);
        for (icon, target) in [
            (I_NEW, Target::New),
            (I_SAVE, Target::Save),
            (I_OPEN, Target::Open),
            (I_CLEAR, Target::Clear),
        ] {
            if x + width(icon) + 4 > room {
                break;
            }
            let area = pill(buf, inner, x, y, icon, t);
            hit(area, target);
            x = area.right() + 1;
        }
        put_right(buf, inner, cr, y, &summary, fg(t.dim));
        y += 2;
    }

    if queue.is_empty() {
        let note = "The queue is empty. Press b to browse your library, or O to open a playlist.";
        put(
            buf,
            inner,
            cx,
            y,
            note,
            fg(t.dim).add_modifier(Modifier::ITALIC),
        );
        return;
    }

    // Columns: shown and sized as set (the list icon's menu, dragging the heading edges).
    let (marker, placed) = crate::columns::layout(cx, cr, &app.config.queue);
    let flexible: Vec<&crate::columns::Placed> =
        placed.iter().filter(|p| p.column.flexible()).collect();

    if full {
        let head = bold(t.dim);
        for p in &placed {
            match p.column {
                Column::Number => put_right(buf, inner, p.x + p.width - 1, y, "#", head),
                Column::Time => put_right(buf, inner, p.x + p.width - 1, y, "TIME", head),
                c => {
                    put(buf, inner, p.x, y, c.heading(), head);
                }
            }
        }
        // A handle between text columns: drag it to resize the one on its left. It can grow
        // as far as the last text column can shrink.
        let give = flexible.last().map_or(0, |last| {
            last.width.saturating_sub(crate::columns::MIN_WIDTH)
        });
        for p in flexible.iter().take(flexible.len().saturating_sub(1)) {
            let edge = p.x + p.width;
            let dragging = matches!(app.drag, Some(crate::app::Drag::Column { column, .. }) if column == p.column);
            let color = if dragging { t.accent } else { t.selection };
            put(buf, inner, edge, y, "╎", fg(color));
            hit(
                Rect::new(edge, y, 2, 1),
                Target::ColumnEdge(p.column, p.x, p.width + give),
            );
        }
        y += 1;
    }

    let rows = inner.bottom().saturating_sub(y) as usize;
    // While the browser has the keys, keep the playing song in view instead of the highlight.
    let anchor = if focused {
        app.queue_sel
    } else {
        current.unwrap_or(app.queue_sel)
    };
    let top = scroll(app.queue_top.get(), anchor, rows, queue.len());
    app.queue_top.set(top);
    let dragging = match app.drag {
        Some(crate::app::Drag::Row { from, over }) if from != over => Some((from, over)),
        _ => None,
    };

    for (i, track) in queue.iter().enumerate().skip(top).take(rows) {
        let ry = y + (i - top) as u16;
        let row = Rect::new(inner.x, ry, inner.width, 1);
        hit(row, Target::Queue(i));
        let playing = Some(i) == current;
        let played = current.is_some_and(|c| i < c);
        let selected = focused && i == app.queue_sel;
        // With songs selected, they're the ones lit up; otherwise the highlight is.
        let lit = if picked.is_empty() {
            selected
        } else {
            picked.contains(&i)
        };
        if lit {
            fill(buf, row, Style::new().bg(t.selection));
        }
        // A dragged song fades where it was; so do the others selected with it.
        let moving = dragging
            .is_some_and(|(from, _)| from == i || (picked.contains(&from) && picked.contains(&i)));
        let faded = |color: Color| {
            if played || moving {
                fg(t.dim).add_modifier(Modifier::DIM)
            } else {
                fg(color)
            }
        };
        if playing {
            put(buf, inner, marker, ry, "▶", bold(t.accent));
        }
        for p in &placed {
            match p.column {
                Column::Number => put_right(
                    buf,
                    inner,
                    p.x + p.width - 1,
                    ry,
                    &(i + 1).to_string(),
                    faded(t.dim),
                ),
                Column::Source => {
                    let (icon, icon_color) = source_icon(track.source, t);
                    put(buf, inner, p.x, ry, icon, faded(icon_color));
                }
                Column::Title => {
                    let style = if playing {
                        bold(t.bright)
                    } else if selected {
                        fg(t.bright)
                    } else {
                        faded(t.light)
                    };
                    put(buf, inner, p.x, ry, &fit(&track.title, p.width), style);
                }
                Column::Artist => {
                    put(
                        buf,
                        inner,
                        p.x,
                        ry,
                        &fit(&track.artist, p.width),
                        faded(t.fg),
                    );
                }
                Column::Album => {
                    let album = album_year(&track.album, track.year);
                    put(buf, inner, p.x, ry, &fit(&album, p.width), faded(t.dim));
                }
                // Stars a space apart, clickable; an unrated song shows faint ones only where
                // the highlight is. Only stars on screen take clicks, so a click on a blank one
                // picks the row. Each star takes the space after it too, as a star can spill
                // into it.
                Column::SourceName => {
                    let (_, color) = source_icon(track.source, t);
                    put(buf, inner, p.x, ry, track.source.name(), faded(color));
                }
                Column::Rating => {
                    let stars = app.rating(track);
                    if stars > 0 || selected {
                        for n in 1..=5u8 {
                            let x = p.x + 2 * (n as u16 - 1);
                            let (star, color) = if n <= stars {
                                ("★", t.yellow)
                            } else {
                                ("☆", t.dim)
                            };
                            put(buf, inner, x, ry, star, faded(color));
                            let w = if n < 5 { 2 } else { 1 };
                            hit(Rect::new(x, ry, w, 1), Target::Star(i, n));
                        }
                    }
                }
                Column::Format => {
                    let color = if hi_res(track) { t.light } else { t.fg };
                    put(
                        buf,
                        inner,
                        p.x,
                        ry,
                        &fit(&format_of(track), p.width),
                        faded(color),
                    );
                }
                Column::Time => {
                    let time = clock(track.duration_ms as f64 / 1000.0);
                    put_right(buf, inner, p.x + p.width - 1, ry, &time, faded(t.dim));
                }
            }
        }
        // Where a dragged row will land: a line on the row it will push down (or up).
        if let Some((from, over)) = dragging
            && over == i
        {
            let mark = if over > from { "▁" } else { "▔" };
            for x in inner.x..inner.right() {
                put(buf, inner, x, ry, mark, fg(t.accent));
            }
            let title = if picked.contains(&from) && picked.len() > 1 {
                format!("{} songs", picked.len())
            } else {
                fit(&queue[from].title, inner.width.saturating_sub(8))
            };
            put(
                buf,
                inner,
                cx + 2,
                ry,
                &format!(" {title} "),
                bold(t.bright).bg(t.selection),
            );
        }
    }
}

/// The keys for whatever has focus, under the panes.
fn draw_hints(buf: &mut Buffer, r: Rect, app: &App, t: &Theme) {
    let pairs: &[(&str, &str)] = if app.browse_open {
        match app.browse.pane {
            Pane::Search => &[
                ("type", "to search"),
                ("↓", "results"),
                ("Tab", "next pane"),
                ("Esc", "close"),
            ],
            Pane::Results => &[
                ("⏎", "add to queue"),
                ("⇧↑↓", "select"),
                ("^Z", "undo"),
                ("1 2 3", "filters"),
                ("/", "search"),
                ("Tab", "next pane"),
                ("b", "close"),
            ],
            Pane::Songs => &[
                ("⏎", "add to queue"),
                ("⇧↑↓", "select"),
                ("^Z", "undo"),
                ("Tab", "next pane"),
                ("b", "close"),
            ],
        }
    } else {
        &[
            ("⏎", "play"),
            ("b", "browse"),
            ("/", "search"),
            ("d", "remove"),
            ("J/K", "move"),
            ("⇧↑↓", "select"),
            ("1–5", "rate"),
            ("^Z", "undo"),
            ("s", "save"),
            ("O", "open"),
            // The rest fall off the end first on a narrow screen, so all keys comes early.
            ("?", "all keys"),
            ("␣", "pause"),
            ("o", "output"),
            ("m", "minimize"),
            ("q", "quit"),
        ]
    };
    hints(buf, r, r.x + 2, r.y, pairs, t);
}

/// News on the left; the version on the right, green when up to date and yellow when a newly
/// installed one is waiting. Clicking it opens Settings → About.
fn draw_footer(
    buf: &mut Buffer,
    r: Rect,
    app: &App,
    t: &Theme,
    hit: &mut impl FnMut(Rect, Target),
) {
    let version = format!("[v{}]", env!("CARGO_PKG_VERSION"));
    let w = width(&version);
    if r.width >= width(&app.status) + 4 + w {
        let x = r.right() - w;
        let color = if app.update_ready.is_some() {
            t.yellow
        } else {
            t.accent
        };
        put(buf, r, x, r.y, &version, bold(color));
        hit(Rect::new(x, r.y, w, 1), Target::Version);
    }
    put(buf, r, r.x + 2, r.y, &app.status, fg(t.accent));
}

// ---- popups ------------------------------------------------------------------------------

/// A box drawn over everything else, on the lighter background. Returns its inside.
fn popup(
    buf: &mut Buffer,
    r: Rect,
    title: &str,
    t: &Theme,
    hit: &mut impl FnMut(Rect, Target),
) -> Rect {
    let r = r.intersection(buf.area);
    Clear.render(r, buf);
    fill(buf, r, Style::new().bg(t.panel));
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(fg(t.fg))
        .title(Line::from(vec![
            Span::styled("─", fg(t.fg)),
            Span::styled(format!(" {title} "), fg(t.dim)),
        ]));
    let inner = block.inner(r);
    block.render(r, buf);
    hit(r, Target::Popup);
    inner
}

/// A popup's area, centred on the screen.
fn centred(screen: Rect, w: u16, h: u16) -> Rect {
    let (w, h) = (w.min(screen.width), h.min(screen.height));
    Rect::new(
        screen.x + (screen.width - w) / 2,
        screen.y + (screen.height - h) / 2,
        w,
        h,
    )
}

/// The "Play to" list, hanging from the output button: one list of this computer's outputs
/// and the Sonos groups, the Sonos ones marked [SONOS].
#[allow(clippy::too_many_arguments)]
fn draw_picker(
    buf: &mut Buffer,
    screen: Rect,
    anchor: Option<Rect>,
    app: &App,
    player: &Player,
    selected: usize,
    all: bool,
    t: &Theme,
    hit: &mut impl FnMut(Rect, Target),
) {
    let ctx = crate::app::Ctx {
        queue: player.queue(),
        devices: &player.status.devices,
        output: &player.output,
        playlist: player.playlist.as_ref(),
        revision: player.revision,
    };
    let rows = app.picker_rows(&ctx, all);
    let hidden = app.picker_rows(&ctx, true).len() - rows.len();
    let w = 60.min(screen.width);
    let h = (rows.len() as u16 + 8).min(screen.height);
    let r = match anchor {
        Some(b) => Rect::new(b.right().saturating_sub(w).max(screen.x), b.bottom(), w, h)
            .intersection(screen),
        None => centred(screen, w, h),
    };
    let inner = popup(buf, r, "PLAY TO", t, hit);
    let (cx, cr) = (inner.x + 1, inner.right().saturating_sub(2));
    let mut y = inner.y;
    for (i, row) in rows.iter().enumerate() {
        let sonos = matches!(row.choice, OutputChoice::Sonos(_));
        let line = Rect::new(inner.x, y, inner.width, 1).intersection(inner);
        hit(line, Target::Device(i));
        if i == selected {
            fill(buf, line, Style::new().bg(t.selection));
        }
        if row.current {
            put(buf, inner, cx, y, "●", fg(t.accent));
        }
        let icon = if sonos {
            I_WIFI
        } else if row.name == "Default output" {
            I_VOLUME
        } else {
            I_HEADPHONES
        };
        let x = put(buf, inner, cx + 2, y, &format!("{icon}  "), fg(t.light));
        let x = if sonos {
            put(buf, inner, x, y, "[SONOS] ", bold(t.dim))
        } else {
            x
        };
        let style = if row.current {
            bold(t.bright)
        } else {
            fg(t.light)
        };
        let room = cr.saturating_sub(x + 12);
        let x = put(buf, inner, x, y, &fit(&row.name, room), style);
        let x = if row.tv {
            put(buf, inner, x + 2, y, I_TV, fg(t.orange))
        } else {
            x
        };
        if !row.note.is_empty() {
            let w = cr.saturating_sub(x + 4);
            put(buf, inner, x + 2, y, &fit(&row.note, w), fg(t.dim));
        }
        if row.hidden {
            put_right(buf, inner, cr.saturating_sub(2), y, "hidden", fg(t.dim));
        }
        let (star, color) = if row.favourite {
            (I_STAR, t.yellow)
        } else {
            (I_STAR_O, t.dim)
        };
        put(buf, inner, cr, y, star, fg(color));
        y += 1;
    }
    let italic = fg(t.dim).add_modifier(Modifier::ITALIC);
    if hidden > 0 {
        put(
            buf,
            inner,
            cx + 2,
            y,
            &format!("+ {hidden} hidden  (H to show)"),
            italic,
        );
        y += 1;
    }
    let note = if app.sonos.is_some() {
        "OMAESTRO never regroups Sonos speakers."
    } else {
        "Looking for Sonos speakers…"
    };
    put(buf, inner, cx, y + 1, note, italic);
    let pairs = [
        ("⏎", "play"),
        ("f", "fav"),
        ("h", "hide"),
        ("H", "all"),
        ("Esc", "close"),
    ];
    hints(buf, inner, cx, inner.bottom().saturating_sub(1), &pairs, t);
}

/// The queue's show/hide-columns menu, hanging from the list icon.
fn draw_columns_menu(
    buf: &mut Buffer,
    screen: Rect,
    app: &App,
    selected: usize,
    t: &Theme,
    hit: &mut impl FnMut(Rect, Target),
) {
    let w = 34.min(screen.width);
    let h = (Column::ALL.len() as u16 + 4).min(screen.height);
    let r = match app.columns_anchor.get() {
        Some(icon) => Rect::new(icon.x.saturating_sub(1), icon.bottom(), w, h).intersection(screen),
        None => centred(screen, w, h),
    };
    let inner = popup(buf, r, "COLUMNS", t, hit);
    for (i, column) in Column::ALL.iter().enumerate() {
        let y = inner.y + i as u16;
        let row = Rect::new(inner.x, y, inner.width, 1).intersection(inner);
        hit(row, Target::ColumnItem(i));
        if i == selected {
            fill(buf, row, Style::new().bg(t.selection));
        }
        let on = app.config.queue.shown(*column);
        let (mark, color) = if on {
            ("■", t.accent)
        } else {
            ("□", t.dim)
        };
        put(buf, inner, inner.x + 1, y, mark, fg(color));
        let style = if on { fg(t.bright) } else { fg(t.fg) };
        let x = put(buf, inner, inner.x + 3, y, column.label(), style);
        if *column == Column::Title {
            put(buf, inner, x + 2, y, "always", fg(t.dim));
        }
    }
    let pairs = [("⏎", "show/hide"), ("Esc", "close")];
    hints(
        buf,
        inner,
        inner.x + 1,
        inner.bottom().saturating_sub(1),
        &pairs,
        t,
    );
}

fn draw_help(buf: &mut Buffer, screen: Rect, t: &Theme, hit: &mut impl FnMut(Rect, Target)) {
    let keys = [
        ("b", "Browse the library"),
        ("/", "Search everywhere: local, Spotify, YouTube Music"),
        ("␣", "Play or pause"),
        ("x", "Stop"),
        ("n  p", "Next or previous song"),
        (",  .", "Back or forward 10 seconds"),
        ("-  +", "Volume down or up"),
        ("o", "Choose where to play (this computer or Sonos)"),
        ("m", "Minimize to the tray (Hyprland)"),
        ("F2", "Settings (the gear button too; / finds a setting)"),
        (
            "i",
            "File Info for the highlighted song (or right-click it)",
        ),
        (
            "U",
            "Switch to a newly installed version now (it also does at the next pause)",
        ),
        ("r", "Check the music folders for changes"),
        ("q", "Quit (the music stops)"),
        ("", ""),
        ("In the queue", ""),
        ("↑↓  j k", "Move the highlight; ⏎ plays it"),
        ("d", "Remove it (or all the selected songs)"),
        ("J  K", "Move it (or them) down or up"),
        (
            "⇧↑↓  ^A",
            "Select several; Ctrl+click adds one; Esc lets go",
        ),
        ("1-5  0", "Rate it (or them), or click a star; 0 unrates"),
        ("^Z  ^Y", "Undo, redo the last change (anywhere)"),
        ("^N", "Start a new playlist (saves the named one first)"),
        ("c", "Clear the queue"),
        ("s  S", "Save the playlist; rename it (or click its name)"),
        ("O", "Open a playlist (saved, Spotify, YouTube Music)"),
        ("C", "Show or hide columns (or click the list icon)"),
        ("", ""),
        ("Browser", ""),
        ("Tab", "Next pane: search, results, songs"),
        ("1 2 3", "Show or hide artists, albums, songs"),
        ("⏎  a", "Add to the queue (never replaces it)"),
        ("⇧↑↓  ^A", "Select several to add; in Search (/) too"),
        ("Esc  b", "Close"),
        ("", ""),
        ("Mouse", "Click buttons and rows, double-click plays"),
        ("", "Drag queue rows to reorder, column edges to resize"),
    ];
    let r = centred(screen, 64, keys.len() as u16 + 4);
    let inner = popup(buf, r, "HELP", t, hit);
    for (i, (key, what)) in keys.iter().enumerate() {
        let y = inner.y + i as u16;
        if what.is_empty() {
            put(buf, inner, inner.x + 1, y, key, bold(t.dim));
            continue;
        }
        put(buf, inner, inner.x + 1, y, key, bold(t.light));
        put(buf, inner, inner.x + 12, y, what, fg(t.fg));
    }
    let note = "Any key or click closes this";
    put(
        buf,
        inner,
        inner.x + 1,
        inner.bottom().saturating_sub(1),
        note,
        fg(t.dim),
    );
}

fn plural(n: usize, word: &str) -> String {
    if n == 1 {
        format!("1 {word}")
    } else {
        format!("{} {word}s", group_thousands(n))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Ctx;
    use crate::player::mpv::Options;
    use crate::testutil::{TempDir, have};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::crossterm::event::{KeyCode, KeyEvent};

    fn screen_text(terminal: &Terminal<TestBackend>) -> String {
        let buffer = terminal.backend().buffer();
        let w = buffer.area.width as usize;
        let cells: Vec<&str> = buffer.content.iter().map(|c| c.symbol()).collect();
        cells.chunks(w).map(|row| row.concat() + "\n").collect()
    }

    fn silent_player(tmp: &TempDir) -> Player {
        let (tx, _rx) = std::sync::mpsc::channel();
        let options = Options {
            audio_device: String::new(),
            replaygain: "off".into(),
            gapless: true,
            volume: 100,
            silent: true,
        };
        Player::start(&options, &tmp.0.join("mpv.sock"), tx).unwrap()
    }

    /// A theme picked in Settings paints its own background (a light one too), so its text is
    /// readable whatever the terminal's colours; Omarchy's theme leaves the terminal's alone.
    #[test]
    fn a_picked_theme_paints_its_background() {
        if !have("mpv") {
            eprintln!("skipping: needs mpv");
            return;
        }
        let tmp = TempDir::new("ui-theme");
        let mut app = crate::app::tests::app(&tmp);
        let player = silent_player(&tmp);
        let latte = crate::theme::Theme::from_toml(
            "background = \"#eff1f5\"\nforeground = \"#4c4f69\"\n\
             lighter_background = \"#dce0e8\"\nselection = \"#ccd0da\"\n",
        );
        app.theme = latte;
        let mut terminal = Terminal::new(TestBackend::new(120, 60)).unwrap();
        let mut corner = |app: &App| {
            terminal
                .draw(|f| {
                    draw(f, app, &player, 0.0);
                })
                .unwrap();
            terminal.backend().buffer()[(0, 59)].bg
        };
        assert_eq!(corner(&app), Color::Reset, "Omarchy's: the terminal's own");
        app.theme_fixed = true;
        assert_eq!(corner(&app), latte.background);
    }

    /// Draws every state at many sizes, with the browser closed, halfway up and open, so layout
    /// mistakes show up as failures here instead of crashes on screen. Uses a silent mpv.
    #[test]
    fn draws_every_state_at_any_size() {
        if !have("mpv") {
            eprintln!("skipping: needs mpv");
            return;
        }
        let tmp = TempDir::new("ui");
        let mut app = crate::app::tests::app(&tmp);
        let mut player = silent_player(&tmp);
        // The sample library has no audio files; mpv's complaints aren't read here.
        player.play(app.browse.tracks.clone(), 1).unwrap();
        let queue = player.queue().to_vec();
        crate::playlist::save(&app.playlist_dir.join("Mix.jspf"), "Mix", &queue).unwrap();
        let ctx = Ctx {
            queue: &queue,
            devices: &[],
            output: &crate::player::Output::Here(String::new()),
            playlist: None,
            revision: 0,
        };
        let key = |app: &mut App, code| {
            app.on_key(KeyEvent::from(code), &ctx).unwrap();
        };

        let sizes = [
            (120, 60),
            (200, 60),
            (160, 45),
            (100, 30),
            (80, 24),
            (60, 12),
            (20, 6),
            (1, 1),
        ];
        for (w, h) in sizes {
            let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
            let mut frame = |app: &App, browse: f32| {
                let mut hits = Vec::new();
                terminal
                    .draw(|f| hits = draw(f, app, &player, browse))
                    .unwrap();
                (screen_text(&terminal), hits)
            };
            let (text, hits) = frame(&app, 0.0);
            if (w, h) == (120, 60) {
                for expected in [
                    "NOW PLAYING",
                    "QUEUE",
                    "Knowing Me",
                    "2 tracks · 6:00",
                    "● unsaved",
                    "ALBUM",
                    concat!("[v", env!("CARGO_PKG_VERSION"), "]"),
                ] {
                    assert!(text.contains(expected), "missing {expected:?}\n{text}");
                }
                for target in [
                    Target::PlayPause,
                    Target::Volume,
                    Target::Seek,
                    Target::Queue(1),
                    Target::New,
                    Target::Save,
                    Target::PlaylistName,
                    Target::Version,
                ] {
                    assert!(hits.iter().any(|h| h.target == target), "no {target:?}");
                }
                assert!(
                    !text.contains("Designed with"),
                    "just the version in the corner\n{text}"
                );
                let area = |target: Target| hits.iter().find(|h| h.target == target).unwrap().area;
                // The four playback buttons are the same size; so are Settings and Help.
                let sizes: Vec<(u16, u16)> = [
                    Target::Stop,
                    Target::Previous,
                    Target::PlayPause,
                    Target::Next,
                ]
                .map(|t| (area(t).width, area(t).height))
                .to_vec();
                assert!(sizes.iter().all(|s| *s == (ICON_BUTTON_W, 3)), "{sizes:?}");
                assert_eq!(area(Target::Settings).width, area(Target::Help).width);
                // Clicking the volume maps onto the bar as drawn.
                assert_eq!(area(Target::Volume).width, VOLUME_SLIDER);
            }
            key(&mut app, KeyCode::Char('b'));
            frame(&app, 0.5);
            for c in "zap".chars() {
                key(&mut app, KeyCode::Char(c));
            }
            key(&mut app, KeyCode::Tab);
            let (text, hits) = frame(&app, 1.0);
            if (w, h) == (120, 60) {
                for expected in [
                    "BROWSE",
                    "ARTISTS",
                    "Zappa",
                    "Apostrophe",
                    "Cosmik Debris",
                    "SONGS",
                ] {
                    assert!(text.contains(expected), "missing {expected:?}\n{text}");
                }
                assert!(hits.iter().any(|h| matches!(h.target, Target::Result(_))));
                assert!(hits.iter().any(|h| matches!(h.target, Target::Toggle(_))));
            }
            key(&mut app, KeyCode::Tab);
            frame(&app, 1.0);
            key(&mut app, KeyCode::Char('o'));
            frame(&app, 1.0);
            key(&mut app, KeyCode::Esc);
            key(&mut app, KeyCode::Char('?'));
            frame(&app, 1.0);
            key(&mut app, KeyCode::Esc);
            key(&mut app, KeyCode::Char('b'));
            key(&mut app, KeyCode::Char('S'));
            key(&mut app, KeyCode::Char('M'));
            let (text, _) = frame(&app, 0.0);
            if (w, h) == (120, 60) {
                assert!(text.contains("⏎ rename · Esc cancel"), "{text}");
            }
            key(&mut app, KeyCode::Esc);
            key(&mut app, KeyCode::Char('O'));
            let (text, _) = frame(&app, 0.0);
            if (w, h) == (120, 60) {
                assert!(
                    text.contains("OPEN PLAYLIST") && text.contains("Mix"),
                    "{text}"
                );
            }
            key(&mut app, KeyCode::Tab);
            let (text, _) = frame(&app, 0.0);
            if (w, h) == (120, 60) {
                assert!(text.contains("Connect Spotify in Settings"), "{text}");
            }
            key(&mut app, KeyCode::Esc);
            key(&mut app, KeyCode::Char('C'));
            let (text, _) = frame(&app, 0.0);
            if (w, h) == (120, 60) {
                assert!(
                    text.contains("COLUMNS") && text.contains("Album (year)"),
                    "{text}"
                );
            }
            key(&mut app, KeyCode::Esc);

            // Search (local only, so the test stays offline), Settings and both sign-ins.
            app.search.show = [true, false, false];
            key(&mut app, KeyCode::Char('/'));
            for c in "dan".chars() {
                key(&mut app, KeyCode::Char(c));
            }
            let (text, hits) = frame(&app, 0.0);
            if (w, h) == (120, 60) {
                for expected in [
                    "SEARCH",
                    "Dancing Queen",
                    "LOCAL (",
                    "1 2 3 toggle providers",
                    "Spotify is off · press 2 to include it",
                    "×",
                ] {
                    assert!(text.contains(expected), "missing {expected:?}\n{text}");
                }
                for target in [Target::SearchBox, Target::CloseOverlay] {
                    assert!(hits.iter().any(|h| h.target == target), "no {target:?}");
                }
                assert!(
                    hits.iter()
                        .any(|h| matches!(h.target, Target::SearchRow(_)))
                );
            }
            key(&mut app, KeyCode::Esc);
            app.open_settings();
            let (text, hits) = frame(&app, 0.0);
            if (w, h) == (120, 60) {
                // One section at a time: Library first, nothing from the others.
                for expected in ["SETTINGS", "Find a setting", "LIBRARY", "Rescan now"] {
                    assert!(text.contains(expected), "missing {expected:?}\n{text}");
                }
                assert!(!text.contains("ReplayGain"), "only Library shows\n{text}");
                assert!(
                    hits.iter()
                        .any(|h| matches!(h.target, Target::SettingsChoice(..)))
                );
            }
            key(&mut app, KeyCode::Tab);
            let (text, _) = frame(&app, 0.0);
            if (w, h) == (120, 60) {
                for expected in ["PLAYBACK", "ReplayGain", " Album "] {
                    assert!(text.contains(expected), "missing {expected:?}\n{text}");
                }
                assert!(!text.contains("Rescan now"), "only Playback shows\n{text}");
            }
            key(&mut app, KeyCode::BackTab);
            // Integrations, and the Last.fm sign-in screen.
            for _ in 0..crate::settings::INTEGRATIONS {
                key(&mut app, KeyCode::Tab);
            }
            key(&mut app, KeyCode::Enter);
            let (text, _) = frame(&app, 0.0);
            if (w, h) == (120, 60) {
                assert!(
                    text.contains("CONNECT LAST.FM") && text.contains("Shared secret"),
                    "{text}"
                );
            }
            key(&mut app, KeyCode::Esc);
            for _ in 0..crate::settings::INTEGRATIONS - crate::settings::SOURCES {
                key(&mut app, KeyCode::BackTab);
            }
            key(&mut app, KeyCode::Enter);
            let (text, _) = frame(&app, 0.0);
            if (w, h) == (120, 60) {
                assert!(
                    text.contains("CONNECT SPOTIFY") && text.contains("127.0.0.1:8898"),
                    "{text}"
                );
            }
            key(&mut app, KeyCode::Esc);
            key(&mut app, KeyCode::Char('j'));
            key(&mut app, KeyCode::Char('j'));
            key(&mut app, KeyCode::Enter);
            let (text, _) = frame(&app, 0.0);
            if (w, h) == (120, 60) {
                assert!(text.contains("CONNECT YOUTUBE MUSIC"), "{text}");
            }
            key(&mut app, KeyCode::Esc);
            key(&mut app, KeyCode::Esc);
            assert!(app.overlay.is_none(), "{w}x{h}");
        }
    }

    #[test]
    fn bars_are_braille_two_dots_wide() {
        let t = Theme::default();
        let mut buf = Buffer::empty(Rect::new(0, 0, 6, 2));
        // Four bars fit in six cells; full, half, empty, full.
        draw_bars(&mut buf, Rect::new(0, 0, 6, 2), &[1.0, 0.5, 0.0, 1.0], &t);
        let row = |y: u16| {
            (0..6)
                .map(|x| buf[(x, y)].symbol().to_string())
                .collect::<String>()
        };
        // Top row: only the two full bars reach it. Bottom row: the half bar too.
        assert_eq!(row(0), "⣿   ⢸⡇");
        assert_eq!(row(1), "⣿⢸⡇ ⢸⡇");
    }

    #[test]
    fn formats_times_rates_and_names() {
        assert_eq!(clock(225.4), "3:45");
        assert_eq!(clock(3725.0), "1:02:05");
        assert_eq!(short_format(Some(24), Some(96000)), "24b-96k");
        assert_eq!(short_format(Some(16), Some(44100)), "16b-44k");
        assert_eq!(short_format(None, None), "");
        assert_eq!(fit("Beautiful People", 9), "Beautifu…");
        assert_eq!(fit("Run", 9), "Run");
        assert_eq!(scroll(0, 30, 10, 50), 21);
        assert_eq!(
            scroll(21, 25, 10, 50),
            21,
            "doesn't move while the row is on screen"
        );
        assert_eq!(
            scroll(40, 5, 10, 12),
            2,
            "no empty rows after the list shrinks"
        );
    }

    /// Prints a screen drawn from the real library index, for checking the layout against
    /// the OTIS design: `OMAESTRO_PREVIEW=search cargo test preview -- --ignored --nocapture`
    /// (browse, search, settings, spotify, youtube, open). Silent; plays nothing, and the
    /// search stays local.
    #[test]
    #[ignore]
    fn preview_with_real_library() {
        let db = crate::library::Db::open(&crate::config::cache_dir().join("library.db")).unwrap();
        let (tx, _rx) = std::sync::mpsc::channel();
        let config = crate::config::Config::load().unwrap();
        let mut app = App::new(db, config, tx).unwrap();
        let tmp = TempDir::new("preview");
        let mut player = silent_player(&tmp);
        let ctx = Ctx {
            queue: &[],
            devices: &[],
            output: &crate::player::Output::Here(String::new()),
            playlist: None,
            revision: 0,
        };
        let keys = |app: &mut App, text: &str| {
            for c in text.chars() {
                app.on_key(KeyEvent::from(KeyCode::Char(c)), &ctx).unwrap();
            }
        };
        let which = std::env::var("OMAESTRO_PREVIEW").unwrap_or_else(|_| "browse".into());
        match which.as_str() {
            "search" | "search-picked" => {
                app.search.show = [true, false, false];
                keys(&mut app, "/black keys");
                app.on_key(KeyEvent::from(KeyCode::Enter), &ctx).unwrap();
            }
            "settings" => app.open_settings(),
            // File Info for the first song of a search, with the file read straight away.
            "info" => {
                keys(&mut app, "bblack keys");
                app.on_key(KeyEvent::from(KeyCode::Tab), &ctx).unwrap();
                let track = app.browse.tracks[0].clone();
                let details = crate::fileinfo::read(std::path::Path::new(&track.path));
                app.on_key(KeyEvent::from(KeyCode::Esc), &ctx).unwrap();
                app.open_file_info(track);
                if let Some(Overlay::FileInfo(info)) = &mut app.overlay {
                    info.details = Some(details);
                }
            }
            // Settings at Integrations, then the Last.fm sign-in.
            "integrations" | "lastfm" | "listenbrainz" => {
                app.open_settings();
                for _ in 0..crate::settings::INTEGRATIONS {
                    app.on_key(KeyEvent::from(KeyCode::Tab), &ctx).unwrap();
                }
                if which == "lastfm" {
                    app.on_key(KeyEvent::from(KeyCode::Enter), &ctx).unwrap();
                }
                if which == "listenbrainz" {
                    app.on_key(KeyEvent::from(KeyCode::Down), &ctx).unwrap();
                    app.on_key(KeyEvent::from(KeyCode::Enter), &ctx).unwrap();
                }
            }
            // Settings at About.
            "about" => {
                app.open_settings();
                for _ in 0..crate::settings::ABOUT {
                    app.on_key(KeyEvent::from(KeyCode::Tab), &ctx).unwrap();
                }
            }
            // Settings at Appearance, on the theme row.
            "themes" => {
                app.open_settings();
                for _ in 0..crate::settings::APPEARANCE {
                    app.on_key(KeyEvent::from(KeyCode::Tab), &ctx).unwrap();
                }
                for _ in 0..2 {
                    app.on_key(KeyEvent::from(KeyCode::Down), &ctx).unwrap();
                }
            }
            "spotify" => {
                app.overlay = Some(crate::app::Overlay::Spotify(
                    crate::connect::SpotifyStep::start(false),
                ))
            }
            "youtube" => {
                app.overlay = Some(crate::app::Overlay::YouTube(
                    crate::connect::YouTubeStep::start(crate::youtube::logins()),
                ))
            }
            "open" => keys(&mut app, "O"),
            // The queue; "picked" with three songs selected.
            "queue" | "picked" => {
                keys(&mut app, "bblack keys");
                app.on_key(KeyEvent::from(KeyCode::Tab), &ctx).unwrap();
                player
                    .enqueue(app.browse.tracks.iter().take(14).cloned().collect())
                    .unwrap();
                keys(&mut app, "b");
                app.on_key(KeyEvent::from(KeyCode::Esc), &ctx).unwrap();
                // A few ratings (only on screen; nothing is saved).
                for (track, stars) in player.queue().iter().zip([5, 0, 3, 4, 0, 1]) {
                    if stars > 0 {
                        app.ratings.insert(crate::plays::key(track), stars);
                    }
                }
                if which == "picked" {
                    app.picks = crate::app::Picks {
                        rows: [2, 3, 4].into(),
                        anchor: 2,
                        revision: player.revision,
                    };
                }
            }
            "help" => app.overlay = Some(Overlay::Help),
            _ => {
                keys(&mut app, "bblack keys");
                app.on_key(KeyEvent::from(KeyCode::Tab), &ctx).unwrap();
            }
        }
        // "search-picked", "browse-picked", "songs-picked": three rows picked with Shift+↓.
        if which.ends_with("-picked") {
            if which == "songs-picked" {
                app.on_key(KeyEvent::from(KeyCode::Tab), &ctx).unwrap();
            }
            for _ in 0..2 {
                let down = KeyEvent::new(
                    KeyCode::Down,
                    ratatui::crossterm::event::KeyModifiers::SHIFT,
                );
                app.on_key(down, &ctx).unwrap();
            }
        }
        let mut terminal = Terminal::new(TestBackend::new(120, 60)).unwrap();
        terminal
            .draw(|f| {
                draw(f, &app, &player, if app.browse_open { 1.0 } else { 0.0 });
            })
            .unwrap();
        print!("{}", screen_text(&terminal));
    }
}
