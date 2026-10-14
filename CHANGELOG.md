# Changelog

Versions are Release.Version.Fix: the middle number goes up for new features, the last for
fixes only. The first moves when a release is called.

## 0.3.0 — 2026-10-13

- **Fix: Spotify on Sonos.**
  - The problem: newer Sonos software (18.x) no longer lists the accounts linked in the
    Sonos app, so OMAESTRO wrongly said Spotify wasn't linked.
  - The fix: it now reads the account from Spotify songs the Sonos app already put on the
    speakers (a queue, a Sonos playlist or a favourite), and remembers it.
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
