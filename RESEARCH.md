# OMAESTRO — research and plan

*Planning only, nothing built yet. Researched 2026-10-12.*

OMAESTRO is a terminal (TUI) music player for Omarchy. It plays Spotify, YouTube Music and the
local FLAC library. Each of those can go to a PC audio device or to a Sonos room or group. It also
shows album art, has a visualizer, and keeps playlists that mix all three sources.

Anything marked **UNVERIFIED** came from a single source or couldn't be tested.

---

## 0. Decisions so far (Brandon, 2026-10-12)

- **Build OMAESTRO as the one music app.** The goal is to stop needing several music players.
  Fooyin feels sluggish and isn't very customizable, and cliamp is missing too much (§1a).
- **Outputs are chosen inside the app, never by changing the desktop's sound output.** Each
  playback goes to a Sonos room/group *or* a local device (e.g. the Schiit Modi+ for headphones),
  picked in OMAESTRO. The desktop default output is never touched.
- **Output favourites.** It's fine for the picker to discover every Sonos group and PipeWire
  device, as long as the unused ones can be hidden and the regulars kept as favourites.
- **The local library must be solid and fast.** It changes rarely, so a refresh at launch plus
  a manual refresh is enough. No live file watching.
- **Spotify Premium:** yes, so the Soloist and Web API route is open.
- **YouTube Music is in v1.** Whether Brandon has YouTube Music Premium isn't confirmed; it only
  changes audio quality.
- **OMAESTRO may play on Office.** Playback commands only, which switch the Arc away from the
  TV. It still never sends grouping or bonding commands (§4.4).
- **Terminal: whichever is fastest and lightest**, including ones not yet installed. Decision:
  **foot** (§4.5).
- **Quitting stops the music, but OMAESTRO can "minimize" to a tray icon** with playback
  controls while it keeps playing (§3.1).

---

## 1. Recommendation

**Build it in Rust with ratatui, and don't make OMAESTRO decode audio itself.** OMAESTRO handles
the screens, the library, playlists and the choice of where sound goes. Actual playback is handed
to programs that already do it well:

| Source | → this PC (PipeWire) | → a Sonos room/group |
|---|---|---|
| **Local FLAC** | **mpv**, driven over its JSON socket | OMAESTRO serves the file over HTTP and Sonos pulls it. Files above 48 kHz are converted once with ffmpeg and cached. |
| **Spotify** | **Spotify Soloist** (Spotify's own headless Linux player), driven over its local WebSocket. librespot is the fallback. | Sonos's built-in Spotify, started over the local UPnP API |
| **YouTube Music** *(experimental)* | **yt-dlp** finds the stream, **mpv** plays it | yt-dlp → ffmpeg → a small HTTP relay → Sonos |

```
 ┌────────────────────── omaestro (one Rust binary, ratatui) ───────────────────────┐
 │  screens · library index (SQLite) · playlists (JSPF files) · router · MPRIS      │
 └────┬──────────────┬────────────────┬──────────────────┬──────────────────────────┘
      │ JSON socket  │ WebSocket      │ SOAP, port 1400  │ HTTP, one LAN port
     mpv          Soloist         Sonos groups      file server / YTM relay
 (local + YTM)   (Spotify)        (controls)        (Sonos pulls audio from here)
      └── PipeWire ──┘
```

**Why this shape:**
- **Less code.** mpv already handles gapless playback, ReplayGain, CUE sheets, seeking and choosing
  an output device. Soloist is Spotify's own engine, so it won't break every time Spotify changes
  its login. Your Rust code is mostly UI and glue, which is the fun part.
- **Fewer moving parts per source.** Each source is a small adapter. YouTube Music, the riskiest
  one, can break without taking anything else down.
- **Rust still pays off.** OMAESTRO scans and searches a 100k-track library, draws covers, and
  serves files to Sonos. All of that benefits from a fast single binary. Rust also has the best
  terminal-image library (ratatui-image), and Myx's MIT-licensed code is there to borrow.

**How it differs from what exists.** Omarchy already ships **cliamp**, a Go player bound to
Super+Shift+Alt+M. It plays Spotify, YouTube Music, local files and a dozen more services. It has
**no Sonos output, no tag-based library tree, no CUE support and no album art in the player**.
OMAESTRO should win on exactly those:
- Sonos rooms and groups as first-class outputs, with the Office rules built in
- a foobar/Fooyin-style library for the whole NAS
- album art and album-art colours in the TUI
- playlists matched by MusicBrainz ID and ISRC instead of guessed by title

> **Honest alternative:** contribute those features to cliamp (MIT, very active, about 174k
> lines of Go) instead of starting a new app. I'd still build OMAESTRO, for two reasons. Sonos as
> an *output* is a change to the architecture, not a plugin; cliamp's Lua plugins can't redirect
> audio. And a 100k-track tag library is a different model from cliamp's folder-and-provider
> browsing.

### 1a. Does cliamp already do all this? (checked 2026-10-12, cliamp main `ce8c65f`)

No. Sonos isn't the only gap.

| Feature asked for | cliamp today |
|---|---|
| Spotify | ✅ via go-librespot (Premium). Open bug [#586](https://github.com/bjarneo/cliamp/issues/586) since 2026-09-29: login fails for some users. It's the same Aug 2026 Spotify login change that hits every librespot-based player. |
| YouTube Music | ✅ via yt-dlp and browser cookies |
| Foobar/Fooyin-style local library | ⚠️ Partly. Files, a file browser, and playlists that pull in whole folders. Missing: a tag-based library tree, a library index (folder sources are rescanned on every load), CUE support, and native sample rates (it resamples to one fixed rate; [#601](https://github.com/bjarneo/cliamp/issues/601)). |
| Choose an audio device | ✅ `d` picker |
| Visualizer | ✅ many modes |
| Album art | ❌ The maintainer has declined it twice. In [#306](https://github.com/bjarneo/cliamp/issues/306): "for me it ruins the tui experience and the layout", and "cliamp is getting into a state where it has toggles for anything, not what I envisioned". |
| Cross-source playlists | ⚠️ Partly. TOML playlists can mix local files and provider tracks, and it loads M3U/PLS. There's no export to Spotify or YouTube Music playlists and no cross-source matching. |
| Sonos | ❌ Nothing. Issue #117 ("UPnP/DLNA") was about *browsing* media servers. |

**Decision (§0): not pursued.** Brandon wants one app, with Sonos chosen from inside it rather
than added as a desktop sound output. Notes kept for reference.

**Ways to add Sonos to cliamp:**
- **Upstream PR (Go).** cliamp decodes every source itself, Spotify included, so a Sonos output could
  stream cliamp's own audio to a Sonos group. Outside PRs are merged often (about 20 in the past
  three weeks), but the maintainer is pushing back on scope, so open an issue first.
- **Fork.** No: the project changes daily.
- **Zero code: [pa-dlna](https://gitlab.com/xdegaye/pa-dlna).** It's Python, MIT, and active
  (2.0 on 2026-09-28; AUR has 1.1, flagged out of date). It makes each network speaker appear as
  a normal PipeWire output, so cliamp, Fooyin or anything else could pick a Sonos room. It
  sends per-track titles with `track_metadata`.
  - **It has no way to limit which speakers it uses.** Checked in its source (`build_list` in
    `pa_dlna/pa_dlna.py`, commit `28b8e14`): every MediaRenderer it discovers gets an output, and
    there's no allow or deny setting. So it would create outputs for Office's Arc too, and
    possibly for the bonded satellites. That breaks the Office rules as-is. Adding an allowlist
    upstream would be a small patch.
  - Sonos compatibility: one bug fixed in 2025 ([#50](https://gitlab.com/xdegaye/pa-dlna/-/issues/50));
    otherwise UNVERIFIED.
  - Pause/skip delay: UNVERIFIED.

---

## 2. Facts about this machine that shape the design

| What | Finding | Why it matters |
|---|---|---|
| Library size | ~98.5k FLAC in 8,763 album folders, 4.4 TB, over **NFS** from the Synology | The scanner has to be fast and incremental. File-change watching (inotify) doesn't see NFS changes made from other machines. |
| Formats (from folder names) | 16/44.1: 5,417 albums · 24/44.1–48: 614 · **above 48 kHz: 1,203** (24/88.2: 57, 24/96: 790, 24/176.4: 7, 24/192: 349). 1,495 folders have no format tag, so 1,203 is a minimum. | Sonos plays at most 24-bit/48 kHz and reportedly *refuses* higher rates, so those albums need converting for Sonos |
| CUE sheets | 1,887 `.cue` files, but nearly all are leftover rip sheets pointing at `.wav` files that no longer exist. Only about 4 albums are one big FLAC plus a CUE. | CUE support is "later", not "must-have". mpv handles it anyway. |
| Tags | Sample of 8: MusicBrainz recording ID **8/8**, ISRC **6/8** (some files have several ISRCs, stored two different ways), AcoustID 0/8 | Playlists can match tracks by ID |
| PipeWire | Locked to **48 kHz** (`clock.allowed-rates [48000]`), so everything is resampled | One small, reversible config file allows native rates (see §4.1) |
| Tools present | mpv 0.41, yt-dlp 2026.08.19 plus deno (its JavaScript runtime), ffmpeg | No Rust toolchain yet: `sudo pacman -S rustup` |
| Terminals | Ghostty 1.3.1 is installed, but `~/.config/xdg-terminals.list` points at **foot**, so app launchers open TUIs in foot | Album art must work with kitty graphics (Ghostty) **and** Sixel (foot) |
| Omarchy 4.0.4 | Theme at `~/.local/state/omarchy/current/theme/`; bar is Quickshell, not Waybar; `mpv-mpris` is preinstalled | Covered in §4.5 |
| Sonos | ReSono already speaks the speakers' UPnP/SOAP API, enforces the **Office** protection rules, and has an opt-in AirPlay config (`~/.config/pipewire/sonos-airplay.conf`) | OMAESTRO reuses ReSono's approach and must keep the Office rules |

---

## 3. Key design decisions

1. **One binary. Quit stops the music; minimize hides it to a tray icon.** No daemon needed.
   - **Quit** (`q`, or closing the window): OMAESTRO stops mpv and pauses Soloist.
   - **Minimize** (a key in OMAESTRO, or the tray menu): the terminal window moves to a hidden
     Hyprland *special workspace*, a scratchpad that's off-screen until toggled. The OMAESTRO
     process keeps running, so the music keeps playing.
     - OMAESTRO also stops drawing the visualizer while hidden, so it costs nothing.
     - Default scratchpad: `special:omaestro`. It's configurable, so it can be your existing
       Super+M `special:music` pad instead. Your `music-toggle` script and `music-layout.lua` will
       need their Spotify, Fooyin and YouTube Music slots swapped for OMAESTRO once it's ready.
   - **Tray icon:** OMAESTRO registers one itself with the `ksni` crate.
     - ksni is pure Rust, speaks the standard StatusNotifierItem tray protocol over D-Bus, and is
       0.3.6 (Jul 2026). Avoid `tray-icon`, which needs GTK on Linux.
     - Your bar already has Omarchy's tray widget (`omarchy.tray`, right side).
     - **Left-click:** show or hide the window.
     - **Menu:** what's playing, Play/Pause, Next, Previous, Output ▸ (favourites), Show,
       Quit.
   - **Hyprland detail found on this machine:** Hyprland 0.56 runs a Lua config, and the old
     `hyprctl dispatch exec "[rules] …"` form fails. The commands must use the Lua form,
     `hyprctl dispatch 'hl.dsp.…'`. Keep the show/hide commands in OMAESTRO's config so a future
     Hyprland change is a config edit, not a rebuild.
   - **Optional:** turning on Omarchy's media widget in the bar adds a now-playing display and
     controls. That works through MPRIS, which OMAESTRO publishes anyway.
2. **Colours.** Default to the Omarchy theme: draw with the terminal's 16 colours, and read
   `colors.toml` only for accent and selection. Theme changes then apply live for free. An
   **album-art accent** mode, as in Myx, is a toggle key.
3. **Visualizer.** Run `cava` in the background and read its raw output, the way rmpc does. cava
   listens to PipeWire, so it works for mpv, Soloist and YouTube Music alike. Point cava at the
   *chosen* device's monitor, not `source = auto`, which always follows the default device. It's
   **hidden when output is Sonos**, because then no sound passes through the PC.
4. **Output picker, driven by the app.** One list holding the PipeWire devices and the Sonos
   groups. The desktop's default output is never changed.
   - **Local devices:** mpv is told `audio-device=pipewire/<node.name>`. For Soloist, which has
     no device setting, OMAESTRO moves only Soloist's own stream with `pactl move-sink-input`.
   - **Sonos:** OMAESTRO sends the group coordinator what to play (§4.4). No desktop audio is
     involved.
   - **Favourites:**
     - Discover everything. Star the outputs you use and hide the rest. A "show all" key brings
       hidden ones back. Give each one a friendly name, e.g. "Headphones (Modi+)".
     - Favourites are saved in `~/.config/omaestro/config.toml`. A PipeWire device is keyed by
       its stable `node.name` (`alsa_output.usb-…`). A Sonos room is keyed by its speaker ID
       (`RINCON_…`), because group IDs change whenever groups change.
   - **Sonos groups are used as they are.** Groups are set up in the Sonos app or ReSono, and
     OMAESTRO never regroups.
     - Rooms are listed under their group's coordinator, the speaker that leads the group.
     - If a favourite room is currently part of someone else's group, picking it plays to that
       whole group, through its coordinator.
   - **Hidden always:** AirPlay devices.
   - **Later:** switching output mid-song, carrying on from the same position.
5. **Sonos safety.** Copy ReSono's deny-by-default allowlist and its Office rules into OMAESTRO.
   Never send grouping or bonding commands. Playback commands go only to a group's coordinator.
   See §4.4 for the exact list.
7. **Customizable from one config file** (`~/.config/omaestro/config.toml`). This matters
   because Fooyin felt rigid. It covers:
   - key bindings
   - the library grouping pattern (e.g. `%albumartist% / (%year%) %album%`)
   - track-list columns
   - pane sizes and which panes are shown
   - colours

   A visual layout editor isn't planned for v1.
8. **Mouse and keyboard both work** (decided 2026-10-12). crossterm, the terminal library ratatui
   uses, can capture the mouse (`EnableMouseCapture`), and Myx already supports clicks this way.
   - **What's clickable:** buttons (stop, previous, play/pause, next, settings, help, the
     playlist buttons, the provider chips).
   - **Rows:** click to select, double-click to play.
   - **Dragging:** the jogger seeks, the volume slider sets the level, and the scroll wheel
     scrolls lists.
   - **How it works:** the screen code already knows each widget's position on every frame, so a
     click is matched against those positions.
   - **Safeguards:**
     - A button fires when the mouse is *released* over it, as in a desktop app.
     - Clicks are ignored for a moment after the window gains focus, so the click that focuses
       the window doesn't also press a button.
   - **Side effects:**
     - Selecting text needs **Shift+drag** while the app owns the mouse. That's standard in foot
       and Ghostty.
     - Inside tmux, mouse support needs `set -g mouse on`.
     - Everything clickable keeps a key.
6. **Playlists are plain files.** One JSPF file per playlist in
   `~/.local/share/omaestro/playlists/`, so they're readable and easy to back up. The library
   index is a throwaway cache in `~/.cache/omaestro/`.

---

## 4. Findings by area

### 4.1 Local library

**Playback engine: mpv**, already installed. OMAESTRO runs it in the background with
`--input-ipc-server` and sends JSON commands over that socket, so no Rust bindings are needed.
`libmpv2` (LGPL) is the in-process alternative.
- **Gapless:** `--gapless-audio`. The manual warns of small gaps when a slow network load hits a
  format change.
- **ReplayGain:** `--replaygain=track|album`.
- **CUE:** external CUE files play as chapters, and `loadfile` with `start=`/`end=` options plays a
  single CUE track.
- **Output device:** read `audio-device-list`, then set `audio-device=pipewire/<node>`. Both work
  while playing.
- **Hi-res:** mpv's PipeWire output asks for each file's own rate.
- **Gotchas, verified today:**
  - Omarchy preinstalls `mpv-mpris` (`/etc/mpv/scripts/mpris.so`), so an mpv started with your
    config appears as a **second player** in the bar. Starting it with `--load-scripts=no` (or
    `--no-config`) stops that; I tested both.
  - Also pass `--audio-client-name=omaestro`, so the bar matches the sound to OMAESTRO.
- **Plan B:** MPD as a daemon with rmpc-style ideas. That's a good fit if you later want phone
  control, but it's an extra service to run.

**Library index.** One SQLite file with FTS5 full-text search (`rusqlite`, bundled), laid out like
Fooyin's: path, subsong, cue path, offset, duration, size, mtime, tags.
- **Scan in two passes:**
  1. Walk the folders in parallel (`jwalk`) and only `stat` the files.
  2. Read tags (`lofty`, cover art skipped) only for files whose size or mtime changed.

  Your NFS mount already uses 4 connections and 1 MB reads, so parallel reads help.
- **Rescanning:** rescan at startup and on a key. inotify, Linux's file-change watcher, doesn't
  report changes made on the NAS from other machines.
- **Measured on this NAS (2026-10-12, read-only):**

  | Step | Time |
  |---|---|
  | List every audio file with size and date (119,982 files, 4.78 TB; directory info already cached) | **1.7 s** on 1 thread, **0.8 s** on 8 |
  | Read tags with `metaflac` | ~9–12 ms per file. Parallel threads barely help, because it reads all of a file's metadata, including embedded art of up to 4 MB. |
  | Read only the first 64 KB of each file, where the tags live | ~3 ms per file on 1 thread, **~1 ms per file with 16 threads** |

  - **Not measured:** the very first listing after a reboot, with nothing cached, will be
    slower.
  - **Measured with OMAESTRO itself (2026-10-12, release build, `omaestro scan`):**
    - **First full scan: 69 s** for **117,707 tracks** (8,541 albums, 1,188 album artists):
      1.8 s finding files, 67 s reading tags with 16 readers.
    - **Refresh with nothing changed: 1.3 s** with jwalk, then **0.3 s** with OMAESTRO's own
      parallel folder walk (NAS caches warm). jwalk was dropped because it silently skips
      folders it can't read, which could have looked like deleted albums.
    - **Search: ~2 ms.**
    - 9 files unreadable (still listed by file name).
    - 95% of tracks carry a MusicBrainz recording ID; 62% carry an ISRC.
    - Earlier "~98.5k FLAC" figures came from a `find -maxdepth 3`, which missed multi-disc
      `DISCn/` folders.
    - lofty with `read_cover_art(false)` measured **0.5 ms/file with 16 threads** (2.5 ms on
      one). That's fast enough, so there's no need to hand-roll a FLAC tag reader.
  - **What that means** (the estimate before measuring):
    - The **first full scan** of ~98.5k FLACs should take **about 2 minutes**, if the tag reader
      skips cover art and runs about 16 reads at once. lofty does skip cover art.
    - **A launch refresh is about 1–2 s**, plus tag reads for changed files only.
    - OMAESTRO opens straight from the cached index and refreshes in the background, so the UI
      never waits on the NAS.
    - Album art is loaded only when shown, then cached as small thumbnails.
- **Album art:** use `cover.jpg`/`folder.jpg` first, then embedded art. Keep resized copies in
  `~/.cache/omaestro/covers`.
- **The Fooyin features worth copying:**
  - a grouping tree (Album Artist › (Year) Album › tracks)
  - fast tag search
  - playlist tabs plus a queue
  - ReplayGain
  - CUE tracks as rows, hiding the big FLAC a CUE points at

  Fooyin's own database (115k rows, 318 MB) is a useful reference for the layout.

**Optional fix you can do today, and undo.** PipeWire is Linux's sound server. Right now it
resamples everything to 48 kHz. This file lets it switch to each song's own rate:

```
# ~/.config/pipewire/pipewire.conf.d/10-sample-rates.conf
context.properties = {
  default.clock.allowed-rates = [ 44100 48000 88200 96000 176400 192000 ]
}
```

Then run `systemctl --user restart pipewire`. To undo, delete the file and restart again.
PipeWire only switches rate while the output is idle. Omarchy has a pull request open for the same
change ([omarchy#13834](https://github.com/omacom/omarchy/pull/13834)).

### 4.2 Spotify

**What changed in 2025–2026.** Spotify has been tightening its API, and the tools around it break
every few months.
- **Developer access** ([Feb 2026](https://developer.spotify.com/blog/2026-02-06-update-on-developer-access-and-platform-security)):
  - The free "development mode" now needs the **app owner to have Premium** and allows **5 users
    per client ID**.
  - Logins expire after 6 months ([Jun 2026](https://developer.spotify.com/blog/2026-06-18-refresh-token-expiration)).
  - Search returns at most 10 results.
  - You can read track lists **only for playlists you own or collaborate on**, so "import a
    friend's playlist" won't work.
- **Removed** ([Nov 2024](https://developer.spotify.com/blog/2024-11-27-changes-to-the-web-api)):
  recommendations, related artists, audio features, and Spotify's own editorial and algorithmic
  playlists.
- **ISRC is still returned.** It was removed in February 2026 and
  [restored in March](https://developer.spotify.com/documentation/web-api/references/changes/march-2026).
- **librespot**, the open-source Spotify engine used by Myx, spotify_player and ncspot:
  - Since **10 Aug 2026**, Spotify rejects its login when used with your own client ID
    ([librespot#1737](https://github.com/librespot-org/librespot/issues/1737)). The fix is a
    pairing code, which is merged in librespot but **not released**; crates.io still has 0.8.0
    from Nov 2025.
  - Logins broke three times between July and September 2026.

**Plan:**
- **Browsing, library, playlists:** the official Web API with **your own client ID** and a browser
  login. `rspotify` 0.16 already uses the February 2026 endpoints (`/playlists/{id}/items`,
  `/me/library`); I checked its source. Myx's `api/` module (MIT) is the backup.
- **Playback on the PC: [Spotify Soloist](https://github.com/spotify/soloist)**, announced in
  [Aug 2026](https://developer.spotify.com/blog/2026-08-13-introducing-spotify-soloist). It's
  Spotify's official headless player for Linux.
  - It outputs to PipeWire and plays lossless up to 24-bit/44.1 kHz on Premium.
  - Its local [WebSocket API](https://developer.spotify.com/documentation/soloist/reference/websocket-api)
    can **play any track, album or playlist link**, queue, seek and set volume. It also sends events
    with metadata and cover URLs.
  - **Caveats:**
    - It's a closed binary you download after logging into a Spotify developer account, with a
      credential per device. So OMAESTRO can't bundle it: you install it, and OMAESTRO detects it.
    - It has no output-device setting. OMAESTRO moves its PipeWire stream instead
      (`pactl move-sink-input`).
    - OMAESTRO never gets the raw audio, so there's no built-in EQ like Myx's. The cava visualizer
      still works.
    - It's two months old, and I've only read its docs.
  - **So the first Spotify task is a test:** install it and confirm `play` works with a track link.
- **Fallback:** librespot pinned to a specific git commit, with pairing-code login, borrowing Myx's
  engine code. This gives back the EQ and an in-process visualizer, at the cost of breaking more
  often.
- **Spotify on Sonos:** send Sonos its own Spotify link format (`x-sonos-spotify:…`) over the local
  UPnP API ([proven on current firmware](https://github.com/steipete/sonoscli/pull/6)).
  - Spotify has to be linked in the Sonos app.
  - The link needs two per-household numbers (`sid`, `sn`). Read them from an existing Spotify
    favourite on your Sonos system rather than hard-coding them. So first save any Spotify album
    as a Sonos Favourite in the Sonos app.
  - Don't use Spotify Connect "transfer playback": Spotify marks Sonos as a restricted device, and
    the Web API won't control it.

**What to borrow from Myx** (MIT, Rust, about 10k lines): the 10-band EQ, the visualizer pipeline,
album-art colour extraction (color-thief, kept readable, with a fade), cover rendering, and MPRIS
wiring.

**Don't fork it.** It has no source abstraction: about 34 places call the Spotify engine directly.

**Don't build on spotatui either.** That's LargeModGames/spotatui, 162k lines. As of v0.44 it logs
in through Spotify's *private* APIs using the desktop app's ID. That's likely against Spotify's
terms (UNVERIFIED).

### 4.3 YouTube Music — **high risk, keep it experimental**

There's no official API, and YouTube actively fights scrapers.
- **Every pure-Rust stream extractor is broken or dead:** rustypipe since March 2026, rusty_ytdl
  returns 403 errors, and ytermusic dropped its own. All the working tools now run **yt-dlp**.
- **Streams:** run the installed `yt-dlp` as a separate process.
  - It already has its JavaScript runtime (deno), which it now requires.
  - Fixes often land in nightly builds days or weeks before Arch gets them.
  - mpv 0.41 lacks the fix for YouTube's forced pre-roll delay
    ([mpv#18192](https://github.com/mpv-player/mpv/issues/18192)). So OMAESTRO should run
    `yt-dlp -j` itself, wait for `available_at`, then hand mpv the direct URL.
- **Search, library and playlists:** two options.
  - `ytmapi-rs` (Rust, from youtui, one maintainer)
  - a tiny helper script using Python's `ytmusicapi`, which gets fixed fastest
- **Login:** **browser cookies only.** OAuth has failed with HTTP 400 since Aug 2025, and the
  maintainer has given up on it ([ytmusicapi#813](https://github.com/sigma67/ytmusicapi/issues/813)).
  - Export cookies from a private window, so YouTube doesn't rotate them.
  - Cookies are tied to your home IP. Fine on the desktop, but they won't work from a server.
  - Heavy use can trigger bot checks, and the yt-dlp wiki warns of a possible **account ban**
    when using cookies.
- **YouTube Music on Sonos:**
  - Sonos's list of supported formats doesn't include Opus (UNVERIFIED whether that's a hard
    no), so the relay converts to AAC or MP3, which Sonos definitely plays.
  - Sonos's own YouTube Music service **can't be controlled by third-party apps**. The only
    workaround is to start a Sonos Favourite you saved in the Sonos app.
- No ISRC anywhere in YouTube Music, so playlist matching falls back to title, artist and duration.

### 4.4 Sonos

- **Local UPnP API on port 1400 still works.**
  - Since firmware 85.0 (July 2025) there's a **"UPnP" switch** under Connection Security, on by
    default. If someone turns it off, every command gets a 403.
  - Sonos calls UPnP "no longer actively maintained", so it's a long-term risk.
  - The cloud Control API is a poor fit: it needs a public HTTPS redirect and can't play arbitrary
    local files.
- **Local files to Sonos:**
  - OMAESTRO runs a small HTTP server over the NFS mount and sends `SetAVTransportURI` or
    `AddURIToQueue` to the **group coordinator**. It includes DIDL-Lite metadata (title, artist,
    album, cover URL) so the Sonos app shows what's playing.
  - **HTTP requirements:** Sonos sends Range and HEAD requests and needs an exact
    `Content-Length` to seek. So converted files go to a cache on disk rather than being
    streamed live.
  - **Conversion:** files above 48 kHz become 24/48 (or 24/44.1 for 88.2 and 176.4 kHz). That's
    1,203 albums, done on first play and then cached.
  - "Sonos refuses rates above 48 kHz rather than downsampling" comes from one community thread
    (UNVERIFIED). Sonos documents the 48 kHz limit, not the failure. So the **first task of
    milestone 2** is to push one 24/96 FLAC to a room other than Office and see what happens.
  - **Firewall:** Omarchy's firewall (`ufw`) blocks incoming connections, so open one port to the
    LAN only.
  - The Sonos Music Library share isn't an option: it's capped at 65,000 tracks and needs SMB.
- **Live-stream routes** (MPD httpd, PipeWire→AirPlay, PipeWire→ffmpeg→HTTP) are gapless, but Sonos
  never sees track changes or metadata.
  - Your AirPlay config already works on the Arc, but it's off for a reason: apps that list audio
    devices knock the Arc off the TV.
  - Keep AirPlay opt-in and hidden from the picker.
- **Status updates:** poll about once a second while playing. GENA events (Sonos pushing changes to
  us) need another open port, so they come later.
- **Commands OMAESTRO needs**, all playback and none grouping:
  - `SetAVTransportURI`, only with `http://`, `x-rincon-queue:`, `x-rincon-mp3radio:` or
    `x-sonos-spotify:` URIs
  - `AddURIToQueue`, `RemoveAllTracksFromQueue`
  - `Play`, `Pause`, `Stop`, `Next`, `Previous`, `Seek`
  - `GetPositionInfo`, `GetMediaInfo`, `GetTransportInfo`
  - group volume (`GroupRenderingControl`)
- **Always blocked:**
  - `SetAVTransportURI` with an `x-rincon:` URI, which is what joins a group
  - `BecomeCoordinatorOfStandaloneGroup`, `BecomeGroupCoordinator*`, `ChangeCoordinator`,
    `DelegateGroupCoordinationTo`
  - everything in `DeviceProperties`
- **Office: playback allowed (decided §0).** ReSono's `send()` refuses *every* playback
  command to Office. That's fine for ReSono, but OMAESTRO uses a finer rule:
  - allowlisted actions only
  - URI-scheme check
  - sent to the Arc (Office's coordinator) only, never to its bonded speakers
  - never any grouping or bonding command

  The picker labels it "Office (switches TV)".
- **Prior art:**
  - [Music Assistant](https://github.com/music-assistant/server) does all of this in Python. Good
    to learn from, but it regroups speakers on its own, which breaks the Office rules.
  - [x2rock](https://github.com/rahga/x2rock) is a 6-week-old Rust Sonos tool for Omarchy, but it
    sends grouping commands.
  - Hand-written SOAP, like ReSono's, is probably simpler than any Rust Sonos crate (`sonos-sdk`
    0.9 is the only active one).

### 4.5 UI, album art, visualizer, media keys, Omarchy

- **Terminal: foot (decided §0).** It's already Omarchy's default for TUI launchers and already
  installed. The other candidates:

  | Terminal | Memory | Album art | Notes |
  |---|---|---|---|
  | **foot** 1.28 | **~7 MB fresh** (published). Your long-running foot windows use 23–49 MB each (measured). | Sixel | CPU-drawn, Wayland-only, fastest start (~39 ms first frame). Sixel uses a limited colour palette, so covers may band slightly (UNVERIFIED how visible; Myx lists foot among its "crispest" terminals). |
  | Ghostty 1.3.1 | ~80–100 MB (published; not measured here) | kitty protocol, full colour | GPU-drawn, ~2 ms input lag. Images drawn too small inside tmux on 1.3.1 (#13056). |
  | kitty (not installed) | ~60–80 MB (published) | kitty protocol, full colour | The kitty image protocol's origin. Only worth it if foot's covers look poor. |
  | Alacritty | low | **none** | Out: no images, so covers would be blocky. |

  Sources: [computingforgeeks 2026 benchmark](https://computingforgeeks.com/best-terminal-emulators-for-linux/),
  [devtoolreviews](https://www.devtoolreviews.com/reviews/ghostty-vs-iterm2-vs-kitty). Both are
  third-party numbers.
  - ratatui-image detects the terminal itself, so OMAESTRO works in either. The launcher just
    picks one: `foot --app-id=omaestro -e omaestro`.
  - **Milestone 6:** run OMAESTRO's real visualizer and cover art in foot and Ghostty side by
    side, measure CPU and memory, and switch if foot's covers disappoint.
- **Framework: [ratatui](https://github.com/ratatui/ratatui) 0.30.** It's healthy, and rmpc, Myx,
  spotify_player and termusic all use it. Textual (Python) is easier to write but slower, with
  weaker images. Bubble Tea (Go) has no image support built in.
- **Album art: [ratatui-image](https://github.com/ratatui/ratatui-image).** Pin it to 11.x; 12 is
  still a release candidate.
  - It auto-detects the terminal: **kitty** graphics in Ghostty, **Sixel** in foot, half-block
    characters elsewhere. Add an `art = off` option too.
  - Known bug: Ghostty 1.3.1 draws images too small **inside tmux**
    ([ghostty#13056](https://github.com/ghostty-org/ghostty/issues/13056)). It's fixed in
    Ghostty's development builds, not yet released.
- **Visualizer:** `cava` with `method = raw`, read from its output. Copy rmpc's
  `src/ui/panes/cava.rs` approach. Needs `sudo pacman -S cava`.
- **Media keys and the Omarchy bar:** MPRIS is the standard D-Bus interface Linux media keys and
  bars use.
  - Use [mpris-server](https://github.com/SeaDve/mpris-server). Unlike souvlaki, it can set
    Identity and DesktopEntry.
  - Use "omaestro" for Identity, DesktopEntry and the PipeWire stream name. Set `mpris:artUrl` to
    a local `file://` cover so the bar shows it.
  - Omarchy's media keys run `omarchy-shell media …`, and its bar picks the player whose name
    matches a playing PipeWire stream.
- **Theme:**
  - Read colours with `omarchy-theme-color --all`, or from
    `~/.local/state/omarchy/current/theme/colors.toml`.
  - To re-theme live, add a hook in `theme-set.d`, or watch the `current/` folder. Don't watch
    `theme/`: Omarchy deletes and replaces it on every theme change.
  - Your OTIS project already reads these palettes, so you can preview OMAESTRO's screens there
    first.
- **Packaging:**
  - Ship OMAESTRO's own `.desktop` file and icon, launching through
    `xdg-terminal-exec --app-id=TUI.float -e omaestro`.
  - Submit a PKGBUILD to [omacom/omarchy-pkgs](https://github.com/omacom/omarchy-pkgs). It takes
    outside PRs: pinned sources, a "watch" file for new releases, and a maintainer's
    `build-approved` label. Rust there is built with `cargo fetch --locked` and
    `cargo build --frozen`.
  - An AUR package is the other route, and `omarchy-tui-install` is the manual fallback.

### 4.6 Cross-source playlists

- **Format: [JSPF](https://xspf.org/jspf)** (JSON XSPF), one file per playlist. Each track holds:
  - title, artist, album and duration
  - `identifier`: the **MusicBrainz recording URL first** (ListenBrainz reads only the first
    one), then `urn:isrc:…`
  - `location`, in order of preference: `file://…`, `spotify:track:…`,
    `https://music.youtube.com/watch?v=…`
- **Resolving which copy to play.** Try the stored locations first. For a track with no playable
  location yet:
  1. Look in the local index by MBID or ISRC.
  2. Ask MusicBrainz which recordings share that ISRC; it allows 1 request per second. One ISRC
     can map to several recordings.
  3. Search Spotify for `isrc:`.
  4. Search YouTube Music.
  5. Fuzzy-match on title, artist and duration, using **spotDL's** proven scoring.
  6. Ask the user.

  Save each hit back to the playlist file along with *how* it matched, and show misses rather than
  guessing.
- **Import:** your own Spotify playlists; YouTube Music playlists (video IDs plus fuzzy matching);
  M3U/M3U8, which is what Fooyin exports.
- **Export:** M3U8 for local tracks (Fooyin and foobar read it), Spotify and YouTube Music via
  their APIs, and ListenBrainz if you use it.
- **Skip [Odesli/song.link](https://odesli.co):** its free API was retired on 2026-07-31 and now
  needs a key requested by email.

---

## 5. Build order

Each milestone ends with something you can use.

| # | Milestone | What works at the end |
|---|---|---|
| 0 | **Setup** | `rustup` installed. Optionally the PipeWire rates file (§4.1). Screens sketched in OTIS. |
| 1 | **Local player** *(built 2026-10-13: everything below, plus the OTIS screens, mouse, queue editing and the slide-up browser. Minimize goes to `special:omaestro`, not `special:music`, which is already Brandon's own music panel)* | Scan the NAS into SQLite. Library tree and search. mpv playback with a queue. Album art. Omarchy colours. Media keys via MPRIS. Minimize to scratchpad, plus the tray icon with controls. |
| 2 | **Outputs + Sonos** *(built 2026-10-13; the first real test on a room is still to do: it needs Brandon's go-ahead and the ufw rule. Discovery and the safety rule are tested; nothing but reads has been sent to the speakers)* | Test one 24/96 file on a Sonos room first, then: output picker (PipeWire devices and Sonos groups) with favourites and hiding. Local files play on Sonos via the built-in server and conversion cache. Office rules enforced. |
| 3 | **Spotify** *(built 2026-10-13: sign-in, search, playlists, Liked Songs, Soloist download and pairing, playback here and on Sonos. Untested until Brandon's Spotify app and Soloist key exist. Soloist sends no end-of-song event, so OMAESTRO watches the position)* | Test Soloist first, then: browse your Spotify library and playlists and play on the PC (Soloist) or Sonos (`x-sonos-spotify`). Fallback: librespot. |
| 4 | **Playlists** *(JSPF save/open with every id kept, and import from Spotify and YouTube Music, built 2026-10-13. Still to do: export to Spotify, M3U8, ID matching between sources)* | JSPF playlists mixing sources. Import and export with Spotify and M3U8. ID-based matching. |
| 5 | **YouTube Music** *(experimental; built 2026-10-13: browser-cookie sign-in, search (tested live, signed out), library, playback through mpv (tested silently) and on Sonos as cached AAC instead of a live relay)* | Search and play via yt-dlp + mpv; Sonos via the relay; import playlists. |
| 6 | **Polish + ship** *(the visualizer is built in-app with pw-record + rustfft instead of cava)* | cava visualizer, album-art colour mode, CUE tracks, foot vs Ghostty test (§4.5), PKGBUILD for omarchy-pkgs or the AUR. |

---

## 6. Questions for Brandon

All of the first round was answered on 2026-10-12; see §0. Still open:

1. **YouTube Music Premium?** It only affects audio quality: about 256 kbps with Premium cookies,
   versus the standard formats.

---

## Appendix: projects checked (2026-10-12)

| Project | What it is | Licence | Last activity | Role for OMAESTRO |
|---|---|---|---|---|
| [Myx](https://github.com/HaseebKhalid1507/Myx) | Rust Spotify TUI (librespot, ratatui) | MIT | v0.5.0, 2026-09-09; pushed 2026-10-07 | Borrow EQ, visualizer, colours, covers |
| [spotatui](https://github.com/LargeModGames/spotatui) | Rust Spotify-first TUI, multi-source | MIT | v0.45.0, 2026-10-12 | Ideas only (162k lines, private APIs) |
| [Spotify Soloist](https://github.com/spotify/soloist) | Official headless Spotify player | proprietary | pushed 2026-08-12 | Spotify engine, if the test passes |
| [librespot](https://github.com/librespot-org/librespot) | Open Spotify engine | MIT | v0.8.0 (2025-11); main active | Fallback Spotify engine |
| [go-librespot](https://github.com/devgianlu/go-librespot) | Spotify daemon with a web API | GPL-3.0 | v0.10.3, 2026-10-03 | Alternative fallback, run as its own program |
| [rspotify](https://github.com/ramsayleung/rspotify) | Rust Spotify Web API client | MIT | 0.16.1, 2026-04 | Web API calls |
| [spotify_player](https://github.com/aome510/spotify-player) / [ncspot](https://github.com/hrkfdn/ncspot) | Spotify TUIs | MIT / BSD-2 | active | Reference |
| [cliamp](https://github.com/bjarneo/cliamp) | Go multi-source TUI, ships with Omarchy | MIT | v2.3.0, 2026-09-28 | The app to stand apart from |
| [mpv](https://github.com/mpv-player/mpv) | Media player | GPL/LGPL | 0.41.0 | Local + YouTube Music engine |
| [yt-dlp](https://github.com/yt-dlp/yt-dlp) | YouTube extractor | Unlicense | 2026.08.19 | YouTube Music streams |
| [ytmusicapi](https://github.com/sigma67/ytmusicapi) / [youtui (ytmapi-rs)](https://github.com/nick42d/youtui) | YouTube Music metadata | MIT | 1.12.3 / 0.3.3, 2026-08/09 | YouTube Music browse and playlists |
| [rmpc](https://github.com/mierak/rmpc) | Rust MPD client | BSD-3 | v0.11.0, pushed 2026-10-07 | Layout, album art, cava integration ideas |
| [termusic](https://github.com/tramhao/termusic) | Rust TUI, multi-backend | GPL-3.0 | v0.13.2 | Reference only (GPL) |
| [Fooyin](https://github.com/fooyin/fooyin) | Qt foobar-style player | GPL-3.0 | v0.13.1 | Feature checklist |
| [MPD](https://github.com/MusicPlayerDaemon/MPD) | Music daemon | GPL-2.0 | 0.24.15, 2026-08 | Plan B engine |
| [SoCo](https://github.com/SoCo/SoCo) | Python Sonos library | MIT | 0.31.5, 2026-10-06 | Sonos reference (DIDL, URIs) |
| [Music Assistant](https://github.com/music-assistant/server) | Home-Assistant music server | Apache-2.0 | 2.10.6 | Sonos design ideas, not a backend |
| [sonoscli](https://github.com/steipete/sonoscli) | Go Sonos CLI | MIT | v0.3.4, 2026-07 | Proven Spotify and YouTube Music → Sonos tricks |
| [x2rock](https://github.com/rahga/x2rock) | Rust Sonos + Omarchy tool | 0BSD | 0.1.0, 2026-10 | Ideas (it sends grouping commands) |
| [ratatui-image](https://github.com/ratatui/ratatui-image) | Terminal images | MIT | 11.1.0 | Album art |
| [mpris-server](https://github.com/SeaDve/mpris-server) | MPRIS for Rust | MPL-2.0 | 0.10.0 | Media keys and bar |
| [cava](https://github.com/karlstav/cava) | Audio visualizer | MIT | 1.0.0 | Visualizer |
| [lofty](https://github.com/Serial-ATA/lofty-rs) · [rusqlite](https://github.com/rusqlite/rusqlite) · [jwalk](https://github.com/Byron/jwalk) | Tags · SQLite · parallel folder walk | MIT/Apache | active | Library scanner |
| [spotDL](https://github.com/spotDL/spotify-downloader) | Spotify→YouTube matcher | MIT | active | Matching algorithm |
| [musicbrainz_rs](https://github.com/oknozor/musicbrainz_rs) | MusicBrainz API client | MIT | pushed 2026-10-10 | ISRC ↔ recording ID lookups |
