# OMAESTRO

[![CI](https://github.com/WolfBrandon/omaestro/actions/workflows/ci.yml/badge.svg)](https://github.com/WolfBrandon/omaestro/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-green.svg)](LICENSE)

A terminal music player for Linux, built for [Omarchy](https://omarchy.org) (Arch + Hyprland).
One app and one queue for your local library, Spotify and YouTube Music, playing on this
computer or on Sonos speakers. Written in Rust with [ratatui](https://ratatui.rs); the screens
were designed in [OTIS](https://github.com/WolfBrandon/otis).

## Features

- **Local library**: a fast index of your music folders (a NAS is fine), a slide-up browser
  (`b`) and search.
- **One queue for everything**: local files, Spotify and YouTube Music songs side by side.
  Remove and drag to reorder, several songs at once if you like; undo (`Ctrl+Z`) and redo
  (`Ctrl+Y`); save as playlists (JSPF) and reopen them. The queue comes back when you reopen
  OMAESTRO, at the song and place you left.
- **Spotify**: sign in with your own Spotify app; search, your playlists and Liked Songs. Plays
  on this computer through [Spotify Soloist](https://github.com/spotify/soloist), or on Sonos.
- **YouTube Music**: uses your browser's login; search, your playlists and Liked Music. Plays
  through mpv, or on Sonos.
- **Sonos**: speakers are found on your network; pick a group in the output picker (`o`).
  OMAESTRO only sends playback commands, and only to a group's coordinator (see
  [Privacy and safety](#privacy-and-safety)).
- **Album art** (real pictures in terminals with sixel or the kitty graphics protocol), a
  **visualizer** (with animations when there's no sound to read, e.g. Spotify on Sonos),
  **media keys** and MPRIS, a **tray icon**, and **minimize** into a Hyprland scratchpad.
- **Themes**: follows Omarchy's theme live, or pick any installed Omarchy theme.
- **Connection status**: a dot and a word for each source (your folders, Spotify, YouTube
  Music, Sonos) in Settings and Search, checked in the background.
- **Plays and scrobbling**: every song you play is counted, giving "Recently played" and "Most
  played" (at the top of your saved playlists: `O`, or the Open button) and a count in File
  Info. Plays are sent to Last.fm and ListenBrainz once you connect them, offline plays later.
- Mouse and keyboard work everywhere.

## Requirements

- Linux with **PipeWire**, and **mpv**.
- To build: **Rust**, latest stable (edition 2024).
- Optional, each switching something on:
  - **yt-dlp**: YouTube Music.
  - **ffmpeg**: hi-res files on Sonos (it converts above 48 kHz) and the visualizer while
    playing on Sonos.
  - **pw-record** (part of PipeWire's tools): the visualizer.
  - **wl-clipboard** (`wl-copy`): the Copy buttons on sign-in screens.
  - **Hyprland** (`hyprctl`): minimize to a scratchpad, and bringing the window back from the
    launcher.
  - **xdg-terminal-exec**: the app-launcher entry that `install.sh` adds.
  - A terminal with **sixel or kitty graphics** (foot, kitty, Ghostty, WezTerm…) for real
    cover pictures; others get coloured blocks.
- A **Spotify Premium** account for Spotify playback.

On Arch:

```
sudo pacman -S --needed mpv yt-dlp ffmpeg pipewire wl-clipboard rust
```

Outside Omarchy it still runs in any terminal. Colours fall back to its built-in theme, and
minimize needs Hyprland.

## Install

```
git clone https://github.com/WolfBrandon/omaestro
cd omaestro
./install.sh            # builds, then adds OMAESTRO to the app launcher
```

`install.sh` puts three things in your home folder:
- `~/.local/bin/omaestro`, so `omaestro` works in any terminal;
- the icon, in `~/.local/share/icons/hicolor/`;
- a launcher entry, `~/.local/share/applications/org.omarchy.omaestro.desktop`.

Run it again to update. A running OMAESTRO notices the new version: the version number in the
bottom right turns from green to yellow, and clicking it opens Settings → About.
- **Auto-update on** (the default):
  - Nothing playing, or paused: it switches straight away, in the same window.
  - Playing: it offers `U` to switch now, without stopping the music for more than a moment.
    Otherwise it switches at the next pause.
- **Auto-update off** (Settings → About): it only says so, and switches when you choose
  Update now there, or press `U`.

The queue, the song and your place in it come along. `./install.sh --uninstall` removes all
three files, and your settings and playlists stay.

The launcher runs `omaestro launch`:
- If OMAESTRO is already running (even minimized), it brings that window back.
- Otherwise it opens OMAESTRO in your default terminal with the app-id
  `org.omarchy.omaestro`, for Hyprland window rules.

Or run it from the source: `cargo run --release`. Also from the command line:
`omaestro scan` (update the library index), `omaestro search led zep`, `omaestro stats`.

## First-time setup

Everything is optional; open Settings with `F2` or the gear button.

1. **Music folders** (Settings → Library). The default is `~/Music`. The first scan of a big
   library takes about a minute; after that, opening is instant and changes are picked up in
   the background.
2. **Spotify** (Settings → Sources → Spotify → Connect). The screen walks you through it:
   - Create an app at [developer.spotify.com/dashboard](https://developer.spotify.com/dashboard)
     with the Redirect URI `http://127.0.0.1:8898/callback`.
   - Paste its Client ID, and sign in in your browser.
   - For playback on this computer, paste an API key from
     [developer.spotify.com/dashboard/soloist](https://developer.spotify.com/dashboard/soloist).
     OMAESTRO downloads Soloist (Spotify's official headless player; closed source) and asks
     you to pick "OMAESTRO" once in a Spotify app.
   - Spotify ends sign-ins after six months; connect again then.
3. **YouTube Music** (Settings → Sources → YouTube Music → Connect). Pick the browser (and,
   for Chromium-based ones, the profile) you're signed in to music.youtube.com with.
   - yt-dlp exports that browser's cookies into a private folder. OMAESTRO keeps only
     YouTube's, in `~/.config/omaestro/ytm-cookies.txt` (readable only by you), deletes the
     rest straight away, and uses them to search and play as you.
   - Your desktop may ask to unlock the keyring.
   - The yt-dlp project warns that heavy automated use with cookies can get an account
     flagged. OMAESTRO only does what you'd do by hand.
4. **Last.fm and ListenBrainz** (Settings → Integrations). Where your plays are sent
   (scrobbled).
   - **Last.fm:** make a free API account at
     [last.fm/api/account/create](https://www.last.fm/api/account/create) (any name; leave
     the Callback URL empty). Paste its API key and shared secret, then allow OMAESTRO on the
     Last.fm page that opens.
   - **ListenBrainz:** paste the user token from
     [listenbrainz.org/settings](https://listenbrainz.org/settings/).
   - **When a play counts:** after half the song or 4 minutes of listening, whichever comes
     first; songs of 30 seconds or less never count. Pauses, loading and skipping ahead don't
     add to it. "Now playing" is sent as a song starts.
   - **Offline:** plays wait in `~/.local/share/omaestro/scrobbles.json` and go, oldest first,
     once the service answers. Each service's row says how it's going: "sending plays", "3
     waiting to send", or "sign in again".
   - **Spotify plays** are sent too. If Spotify already scrobbles for you (linked in
     Spotify's own settings), switch **Spotify plays** off there so they don't count twice.
   - Plays are counted here whether or not you connect either service.
5. **Sonos**. Speakers are found by themselves.
   - They fetch local files and YouTube songs from OMAESTRO on port 47100, so allow that from
     your home network if you run a firewall, e.g. with ufw (adjust the network to yours):
     ```
     sudo ufw allow from 192.168.1.0/24 to any port 47100 proto tcp comment 'OMAESTRO for Sonos'
     ```
   - Spotify on Sonos needs Spotify linked in the Sonos app, and no firewall rule: the speaker
     streams it from Spotify itself.
   - Sonos 18.x no longer lists linked accounts. So OMAESTRO reads your Spotify account's
     number from a Spotify song the Sonos app has put on a speaker: a room's queue, or a
     Spotify album or playlist in Sonos favourites. If it can't find one, play any Spotify song
     once from the Sonos app.

The web addresses on the sign-in screens can be clicked, and each has a Copy button.

## Keys

`?` shows them in the app too, and everything can be clicked.

| Key | Does |
|---|---|
| `b` | Slide the library browser up from the bottom (`b` or `Esc` again closes it) |
| `/` | Search everywhere: local, Spotify, YouTube Music (`1` `2` `3` switch providers) |
| `j` `k` or arrows | Move the highlight; `Enter` plays it |
| `Space` | Play / pause |
| `x` | Stop (play starts the song again) |
| `n` / `p` | Next / previous |
| `,` / `.` | Back / forward 10 seconds |
| `-` / `+` | Volume (OMAESTRO's own, not the desktop's; each output keeps its own) |
| `o` | Where to play: this computer's outputs and the Sonos groups, in one list. `f` stars one, `h` hides one, `H` shows hidden ones |
| `Ctrl+Z` / `Ctrl+Y` | Undo / redo the last change to the queue, from anywhere |
| `m` | Minimize into the `special:omaestro` scratchpad; the tray icon or a media widget brings it back |
| `F2` | Settings, one section at a time (`Tab` or click a pill); `/` finds a setting by name |
| `U` | Switch to a newly installed version now (otherwise it switches at the next pause) |
| `i` | File Info for the highlighted song (or right-click it): tags, format, bitrate, size, ReplayGain, path or link. `c` copies the path or link, `o` opens its folder or page |
| `r` | Check the music folders for changes |
| `q` | Quit (the music stops) |

**In the queue:**
- `d` removes, `J`/`K` move, `c` clears; `Ctrl+Z` undoes and `Ctrl+Y` redoes.
- **Several songs at once:** `Shift+↑`/`↓` (or `Shift+Home`/`End`) selects from the highlight
  on, `Ctrl+A` selects them all, and `Ctrl+click` adds or drops one (`Alt+click` selects up to
  the song clicked).
  `d` removes the selected songs, and `J`/`K` or dragging any of them moves them together.
  `Esc` lets go. (`Shift+click` is kept by the terminal for selecting text, so it doesn't
  reach OMAESTRO.)
- **Star ratings:** `1` to `5` rate the highlighted song (or all the selected ones), and `0`
  takes the rating off. Or click a star in the Rating column; clicking the rating a song
  already has takes it off. A rating belongs to the song, so it shows in every playlist and
  in File Info. Ratings stay in OMAESTRO; your music files are never changed.
- `s` saves; `S`, or clicking the playlist's name, renames it right there.
- `Ctrl+N`, or the New button (first of the four by the playlist's name), starts a new
  playlist. If the one in the queue has a name and changes, it's saved first; then the queue
  empties and becomes Untitled. `Ctrl+Z` brings it back. (`c` only empties the queue: it's
  still that playlist, so `s` would save over it.)
- `O` opens a playlist: saved ones, or your Spotify and YouTube Music playlists.
- With the mouse: drag songs to reorder, and drag the `╎` marks in the heading row to resize
  columns. The list icon at the top left (or `C`) shows or hides columns.

**In the browser:** typing goes into the search box. `Tab` moves between the box, the results
and the songs. `Enter` or a double-click adds to the end of the queue; it never replaces the
queue.

**In Search:** each provider's results get their own box, and `Tab` jumps between them.
`Enter` plays a song now (the queue carries on after it), `a` adds it, `A` adds everything that
provider found.

## Configuration

Most settings are in the app. They're saved in `~/.config/omaestro/config.toml`:

```toml
library_paths = ["~/Music"]             # folders to scan
scan_threads = 16                       # files read at once while scanning (16 suits a NAS)
scan_on_launch = true

[playback]
replaygain = "album"                    # "off", "track" or "album"
gapless = true
audio_device = ""                       # set by the output picker
output = ""                             # "" here, or "sonos:<speaker id>"
volume = 100

[appearance]
album_art = "hi-res"                    # "hi-res" (real pictures), "lo-res" (coloured blocks) or "off"
visualizer = true
animation = "starfield"                 # for Spotify on Sonos: "plasma", "kaleidoscope" or "off"
theme = "omarchy"                       # Omarchy's current theme, or an installed one: "tokyo-night"

[queue]                                 # the queue's columns (set with the mouse or C)
hidden = []                             # e.g. ["number", "format"]
title = 30                              # widths of the text columns; the last one shown
artist = 22                             # takes whatever room is left
album = 28

[outputs]                               # the picker's stars and hidden ones
favourites = []
hidden = []

[updates]
auto = true                             # switch to a newly installed version by itself

[scrobble]
spotify = true                          # send Spotify plays to Last.fm and ListenBrainz too

[sonos]
speakers = []                           # found by themselves, then remembered
media_port = 47100
never = []                              # speaker ids never to send anything to
```

## Files it uses

- `~/.config/omaestro/config.toml`: settings. The folder is readable only by you.
- `~/.config/omaestro/spotify.json`, `soloist.json`, `youtube.json`, `ytm-cookies.txt`,
  `lastfm.json`, `listenbrainz.json`: sign-ins, readable only by you. Deleting one signs out.
- `~/.local/share/omaestro/playlists/*.jspf`: your playlists (JSPF, a standard format).
- `~/.local/share/omaestro/last-queue.jspf` and `last-queue.json`: the queue as you left it.
- `~/.local/share/omaestro/plays.db`: every play (when, and the song), and your star
  ratings. Deleting it forgets them.
- `~/.local/share/omaestro/scrobbles.json`: plays waiting to go to Last.fm or ListenBrainz
  (readable only by you).
- `~/.local/share/omaestro/soloist/`: Spotify Soloist, once downloaded.
- `~/.cache/omaestro/library.db`: the library index. Safe to delete; it's rebuilt.
- `~/.cache/omaestro/sonos/`: songs converted or downloaded for Sonos, cleared after a week.
- `~/.cache/omaestro/sonos.log`: every command that changed something on a speaker
  (readable only by you; song addresses aren't written down). Past 1 MB the previous one is
  kept as `sonos.log.old`.
- `~/.cache/omaestro/soloist.log`: what was sent to Soloist and what it answered.
- `~/.cache/omaestro/scrobble.log`: plays a service refused, and why; and retries.
- `~/.cache/omaestro/sonos-spotify-account`: Sonos's number for your Spotify account.
- `$XDG_RUNTIME_DIR/omaestro/mpv.sock`: the control line to mpv while OMAESTRO runs.

## Privacy and safety

- **No telemetry.** OMAESTRO talks to:
  - Spotify's Web API, through your own app;
  - YouTube, directly and through yt-dlp;
  - Last.fm and ListenBrainz, once you connect them: each song as it starts and once it
    counts (title, artist, album, length, track number, when, and the MusicBrainz id and
    ISRC if tagged; for Spotify and YouTube Music songs, their web address). Never a file's
    name or folder;
  - Sonos speakers on your network;
  - Soloist, on this computer.
- **Sign-ins** are stored in `~/.config/omaestro`, readable only by you. The YouTube Music
  sign-in is a copy of your browser's YouTube cookies, and only those.
- **Sonos:**
  - OMAESTRO sends only playback commands (play, pause, stop, seek, volume), and only to a
    group's coordinator.
  - It never sends grouping or bonding commands, and nothing at all to bonded speakers
    (surrounds, subs).
  - Everything else it sends is a read. Every command that changes something is logged in
    `sonos.log`, and speaker ids in `[sonos] never` are left alone entirely.
- **The file server** on port 47100 opens the first time you play on Sonos, and stays open
  until you quit. It:
  - offers only the song playing now, under a random address, and nothing while Sonos is
    stopped;
  - answers only your Sonos speakers;
  - serves only files inside your music folders, and OMAESTRO's own converted copies.
- **See also** [SECURITY.md](SECURITY.md).

## Performance

Measured on a 117,707-track library on a NAS:
- first scan: 69 s;
- refresh with nothing changed: 0.3–1.3 s;
- search: about 2 ms.

## Development

```
cargo test                      # silent: mpv plays into a null output; tests needing mpv or ffmpeg skip without them
cargo clippy --all-targets
OMAESTRO_PREVIEW=search cargo test preview -- --ignored --nocapture   # prints a screen from your library
```

The `live_` tests (`cargo test live_ -- --ignored --nocapture`) check against real services:
a YouTube search and silent playback, a Sonos household
(`OMAESTRO_SONOS_IP=<speaker address>`) and a PipeWire output (`OMAESTRO_SINK=<node name>`).

See [CONTRIBUTING.md](CONTRIBUTING.md). Changes are in [CHANGELOG.md](CHANGELOG.md), and
what's planned for 1.0.0 is in [BACKLOG.md](BACKLOG.md). Versions are Release.Version.Fix.

## Acknowledgements

- [ratatui](https://ratatui.rs) and [ratatui-image](https://github.com/benjajaja/ratatui-image).
- [mpv](https://mpv.io), [yt-dlp](https://github.com/yt-dlp/yt-dlp),
  [FFmpeg](https://ffmpeg.org) and [PipeWire](https://pipewire.org).
- [Spotify Soloist](https://github.com/spotify/soloist).
- [Omarchy](https://omarchy.org), whose themes it follows.
- Designed in [OTIS](https://github.com/WolfBrandon/otis).

OMAESTRO isn't affiliated with or endorsed by Spotify, Sonos, Google/YouTube or Omarchy; their
names are trademarks of their owners.

## License

[MIT](LICENSE).
