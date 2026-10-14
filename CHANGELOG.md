# Changelog

Versions are Release.Version.Fix: the middle number goes up for new features, the last for
fixes only. The first moves when a release is called.

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
