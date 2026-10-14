# Changelog

Versions are Release.Version.Fix: the middle number goes up for new features, the last for
fixes only. The first moves when a release is called.

## 0.9.1 — 2026-10-14

- **Ready for a public release.**
  - An MIT licence, and a README for people new to OMAESTRO (requirements, install,
    setup, privacy and safety).
  - CONTRIBUTING and SECURITY notes, issue templates, and tests run by GitHub on every push.
- The Sonos test data no longer holds a real household's speaker ids and addresses.
- The early research notes moved out of the repository.

## 0.9.0 — 2026-10-14

- **Rename a playlist by clicking its name** in the queue's header, or with `S`. You type the
  new name right there; Enter renames, Esc cancels. A playlist that's been saved has its file
  renamed too. "Save as…" is gone.
- **Spotify and YouTube Music playlists keep their own names** in the queue ("Episode 2", not
  "Untitled"); `s` saves a copy under that name.
- The queue's Save, Open and Clear buttons are just icons now.
- Settings has no Behaviour or Keys sections.

## 0.8.0 — 2026-10-13

- **The queue comes back when you reopen OMAESTRO.**
  - What returns: the songs (Spotify and YouTube Music ones too), the playlist's name and
    whether it was saved, the song it was on, and how far in.
  - Nothing plays until you press Space, which carries on from there. Any other song starts
    at its beginning.
  - It's kept whenever the queue or the song changes, so a crash loses at most the place in
    the song.
- **Fix: switching a Spotify song from Sonos back to this computer.**
  - The problem: it jumped back to where it had paused, looped a few seconds and skipped to
    the next song.
  - Cause 1: the same song again counted as "the previous song", so all of Soloist's news
    about it was ignored. OMAESTRO thought it never started, asked again every few seconds,
    and gave up.
  - Cause 2: the seek to where Sonos was went out with "play" and was lost while the song
    loaded.
  - The fix: the same song's news now counts, and the seek waits until Soloist is playing.

## 0.7.4 — 2026-10-13

- **Fix: no sound after switching from Sonos back to this computer.**
  - The cause: while on Sonos, the volume shown is the speaker's, and switching back handed
    that number to this computer. Soloist plays its volume on a cube curve, so 27% came out
    at about 2%: effectively silent.
  - The fix: this computer's volume is now kept separately, and returns when you switch
    back. It's also the one saved for next time, not a Sonos speaker's.

## 0.7.3 — 2026-10-13

- **Fix: the visualizer for Spotify played here, and likely the popping.**
  - The problem: for Spotify on this computer, the visualizer listens to the whole output (the
    G560). It asked PipeWire never to fall back to another device, and with that, PipeWire
    1.6 can't find a busy output by name ("defined target not found"). So each capture died
    at once, and OMAESTRO started a new one twice a second. The bars flickered, and the
    constant rewiring of the G560 is the likely cause of the pops.
  - The fix: output captures no longer ask that. An output capture falls back to the default
    output, never the microphone (checked).
  - A capture that keeps dying is now tried less and less often (½, 1, 2, 4, then 8 s apart)
    instead of twice a second.

## 0.7.2 — 2026-10-13

- **Fix: Spotify songs on this computer skipped without playing.**
  - The likely cause: when Soloist becomes the active device, Spotify can resume what the
    account last played elsewhere. OMAESTRO took that other song as "ours has finished",
    paused Soloist and moved on.
  - Now news about any other song counts only once ours has actually started.
  - Until then, Soloist is asked again every 4 seconds. After 3 tries the song is skipped,
    with "Spotify didn't start …" on the status line.
- `~/.cache/omaestro/soloist.log` records what's sent to Soloist and what it answers (started
  afresh each time Soloist starts), like `sonos.log`.
- Opening a playlist says how to start it: "Opened Episode 2: 13 songs · Space or ⏎ plays".

## 0.7.1 — 2026-10-13

- **Fix: "Spotify: command requires authentication".**
  - The cause: Soloist restarts when the output changes, and takes a moment to sign in to
    Spotify. OMAESTRO sent "play" straight away, and Soloist refused it.
  - The fix: commands now wait until Soloist says it's signed in. A command it refuses for
    that reason is sent again once it is; confirmed ones never are.
  - After 20 seconds without a sign-in, OMAESTRO says Soloist isn't signed in.
- Spotify played on this computer can't show its quality: Soloist doesn't report it (its
  documented interface has no such fields). OMAESTRO no longer keeps a copy of Soloist's
  reports (`~/.cache/omaestro/soloist-last.json`).

## 0.7.0 — 2026-10-13

- **Animations for Spotify on Sonos.**
  - The problem: the speaker streams Spotify itself, so there's no sound here to draw bars
    from.
  - The fix: the visualizer now shows a starfield, a plasma or a kaleidoscope instead, in
    the theme's colours. Pick one in Settings → Appearance → Animation (or Off).
  - All three are written for OMAESTRO: asciiart.eu's versions reserve their code.

## 0.6.1 — 2026-10-13

- **Settings shows one section at a time:** the one picked in the pills (or with `Tab`),
  instead of scrolling down a page of them. `↑` `↓` stay within it. While something is typed in
  the find box, matches from every section show.
- Five ideas added to [BACKLOG.md](BACKLOG.md): sort by popularity, track plays (and
  scrobble), configure the output rate, show each source's connection status, and update
  without closing and reopening.

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
