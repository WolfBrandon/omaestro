# Changelog

Versions are Release.Version.Fix: the middle number goes up for new features, the last for
fixes only. The first moves when a release is called.

## 0.6.0 — 2026-10-13

- **Settings, redesigned** to the "Settings (new)" OTIS tab, in the style of Search.
  - **Find a setting** (`/` or click the box): only matching settings show, by name, section,
    option or note. Esc clears the search, then closes.
  - The sections are pills with icons; clicking one jumps there, and `Tab` steps through them.
  - Each section is a box, the one you're in with a brighter border.
  - Choices are pills; the selected row sits on a darker bar with a ›.
  - × closes Settings. "OTIS" under About is a link.
- **Theme list.** Appearance lists every installed theme, the chosen one ► between
  arrows ◄; click one to pick it. Light themes are marked ☀.

## 0.5.0 — 2026-10-13

- **Themes.** Settings → Appearance → Theme picks any Omarchy theme installed here (Omarchy's
  own and yours in `~/.config/omarchy/themes`), or keeps following Omarchy's, as before.
  - `←` `→` or ◂ ▸ try the next one; it shows at once and is saved.
  - A picked theme paints its own background, so light themes work in a dark terminal.
- The OTIS design's "Settings (new)" tab is redrawn in the style of Search, with a theme grid.
  The Settings screen itself is still the old layout until that design is built.

## 0.4.0 — 2026-10-13

- **Search, redesigned** to the new OTIS tab.
  - The search box is at the top, with the providers under it.
  - Each provider's results get their own box, with their count and column headings; the
    box you're in has a brighter border.
  - A provider that's off gets a line saying how to switch it on.
  - A × closes Search.
- **Buttons, centred for real.**
  - The trouble: in foot's font (GeistMono Nerd Font), the old Font Awesome icons are wider
    than a cell, and a terminal can only move things by whole cells.
  - The fix: playback buttons and Help now use Material Design icons, which are narrower
    than a cell and centred in it, in buttons 5 cells wide.
  - Settings uses Material's cog. No gear in the font is narrow enough to sit exactly in the
    middle; this one is the closest.
- Quality for Spotify on Sonos still shows nothing: for a song handed over the way OMAESTRO
  does it, the speaker doesn't report the quality.

## 0.3.0 — 2026-10-13

- **Fix: Spotify on Sonos.**
  - The problem: newer Sonos software (18.x) no longer lists the accounts linked in the
    Sonos app, so OMAESTRO wrongly said Spotify wasn't linked.
  - The fix: it now reads the account from Spotify songs the Sonos app already put on the
    speakers (a room's queue, or a Spotify album or playlist in Sonos favourites), and
    remembers it.
  - Spotify links are now made exactly the way the Sonos app makes them.
- **Stream quality in Now Playing.** Spotify and YouTube songs show how they're coming in,
  e.g. "OPUS 141 kbps", "AAC 256 kbps" or "LOSSLESS 16b-44k", instead of "SPOTIFY" or
  "YOUTUBE". Nothing shows until that's known.
- The footer's **OTIS** is a link to its repository (click it); the address itself is gone.
- **Buttons.**
  - The four playback buttons are the same size and look; play/pause is no longer
    highlighted.
  - Icons sit in the middle of their buttons, including Settings and Help.
  - The output button has its icon on the left and the name and ▾ on the right.
  - The volume slider is about 25% longer.

## 0.2.0 — 2026-10-13

- **An app of its own.** `./install.sh` adds OMAESTRO to the app launcher (Super+Space), with
  its own icon. Launching it again while it's running, even minimized, brings the window back
  instead of opening a second one.
- **Albums show their year** in the browser, Search and the queue: "Brothers (2010)".
- **Queue columns.**
  - A new Album (year) column.
  - Drag a song to move it, and drag the `╎` marks in the heading row to resize columns.
  - The list icon at the top left (or `C`) shows or hides columns.
  - Column choices are saved in the settings file.

## 0.1.0 — 2026-10-13

Everything up to here, in four commits:
- the local library, its browser and search;
- Spotify and YouTube Music, with their sign-in screens;
- Sonos;
- album art, the visualizer (on Sonos too), media keys, the tray icon and minimize;
- `Ctrl+Z` undo;
- the browser adding songs without replacing the queue.
