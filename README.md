# OMAESTRO

A terminal music player for Omarchy: one app for the local library, Spotify, YouTube Music
and Sonos. The research and the plan are in [RESEARCH.md](RESEARCH.md); the screens were
designed in OTIS (project "Omaestro").

## What works

- **Local library**: a fast index of the NAS, the slide-up browser (`b`) and search.
- **One queue for everything**: local files, Spotify and YouTube Music songs side by side;
  remove, move, undo, clear; saved as playlists you can reopen (`O`).
- **Spotify**: sign in from Settings; search, your playlists and Liked Songs; plays on this
  computer through Spotify Soloist (set up from the same screen), or on Sonos.
- **YouTube Music**: sign in with your browser's login; search, your playlists and Liked
  Music; plays through mpv, or on Sonos.
- **Sonos**: the speakers are found by themselves; pick a group in the output picker (`o`).
  OMAESTRO only ever sends playback commands to a group's coordinator, never grouping or
  bonding commands, and nothing at all to bonded speakers (Office's surrounds, subs, lamps).
- **Album art**, the **visualizer**, **media keys** and the bar's media widget (MPRIS), a
  **tray icon**, and **minimize** (`m`) into a Hyprland scratchpad.
- Mouse and keyboard both work everywhere.

Not yet confirmed on the real thing: Spotify playback on this computer (through Soloist),
the visualizer while playing on Sonos, and minimizing (`m`) from the launcher's window.
Playing local files on Sonos works (needs the firewall line in "First-time setup").

## Install it as an app

```
cd ~/projects/local/omaestro
./install.sh                   # build, then add OMAESTRO to the app launcher
```

Then open it with Super+Space → OMAESTRO. Run `./install.sh` again after changes to update the
app (the version shows at the bottom right). It puts three things in your home folder:

- `~/.local/bin/omaestro`: the program (so `omaestro` also works in any terminal);
- `~/.local/share/icons/hicolor/…/omaestro.svg` and `.png`: the icon;
- `~/.local/share/applications/org.omarchy.omaestro.desktop`: the launcher entry.

`./install.sh --uninstall` takes them away again; your settings and playlists stay.

The launcher runs `omaestro launch`. If OMAESTRO is already running, even minimized, that
brings its window back instead of opening a second one. Otherwise it opens OMAESTRO in your
default terminal (foot) with the app-id `org.omarchy.omaestro`, so Hyprland window rules can
pick it out. It opens tiled; for a floating window, add a rule matching that app-id.

## Run it from the source

```
cd ~/projects/local/omaestro
cargo run --release            # the player
cargo run --release -- scan    # update the library index from the terminal
cargo run --release -- search led zep
```

The first build takes a few minutes. After that, `cargo run --release` starts in about a
second. Quitting stops the music, wherever it's playing.

## First-time setup

All optional; each one switches something on.

1. **Spotify** (gear button or `F2` → Sources → Spotify → Connect). The screen walks you
   through it: create an app at developer.spotify.com/dashboard with the Redirect URI
   `http://127.0.0.1:8898/callback`, paste its Client ID, and sign in in the browser. Then the
   playback step: paste an API key from developer.spotify.com/dashboard/soloist, and pick
   "OMAESTRO" once in the Spotify app on your phone. Spotify ends sign-ins after six months;
   connect again then.
2. **YouTube Music** (Settings → Sources → YouTube Music → Connect): pick the browser you're
   signed in to music.youtube.com with. Chromium, Chrome and Brave list each profile
   separately (with its account), so pick the profile you're signed in on. Your desktop may ask
   to unlock the keyring.

The web addresses on the sign-in screens can be clicked to open them, and each has a Copy
button. In foot, Ctrl+Shift+O also picks out links on screen.
3. **Sonos**: speakers fetch local files and YouTube songs from OMAESTRO on port 47100. Omarchy's
   firewall blocks that until you allow it for your home network (one time, needs your
   password):
   ```
   sudo ufw allow from 192.168.1.0/24 to any port 47100 proto tcp comment 'OMAESTRO for Sonos'
   ```
   Spotify on Sonos needs Spotify linked in the Sonos app; it doesn't need the firewall line.
   Sonos plays Spotify songs from Spotify itself, and the link has to name your Spotify
   account the way Sonos numbers it. Newer Sonos software (18.x) won't list accounts, so
   OMAESTRO finds the number on Spotify songs the Sonos app has already put on a speaker
   (a room's queue, or a Spotify album or playlist in Sonos favourites) and remembers it in
   `~/.cache/omaestro/sonos-spotify-account`. If it can't find one, play any Spotify song
   once from the Sonos app, in any room.

## Keys

`?` shows these in the app too, and everything can be clicked.

| Key | Does |
|---|---|
| `b` | Slide the library browser up from the bottom (`b` or `Esc` again closes it) |
| `/` | Search everywhere: local, Spotify, YouTube Music (`1` `2` `3` switch sources) |
| `j` `k` or arrows | Move the highlight; `Enter` plays it |
| `Space` | Play / pause |
| `x` | Stop (play starts the song again) |
| `n` / `p` | Next / previous |
| `,` / `.` | Back / forward 10 seconds |
| `-` / `+` | Volume (only OMAESTRO's, not the desktop's) |
| `o` | Where to play: this computer's outputs and the Sonos groups (marked [SONOS]) in one list. `f` stars one, `h` hides one, `H` shows hidden ones |
| `Ctrl+Z` | Undo the last change to the queue, from anywhere (the status line says what was undone) |
| `m` | Minimize into the `special:omaestro` scratchpad; the tray icon or a media widget brings it back |
| `F2` | Settings: a box per section; `Tab` jumps between them, `/` finds a setting by name |
| `r` | Check the music folders for changes |
| `q` | Quit (the music stops) |

In the queue: `d` removes, `J`/`K` move, `u` or `Ctrl+Z` undoes, `c` clears, `s` saves, `S`
saves as, `O` opens a playlist (saved ones, or your Spotify and YouTube Music playlists).
Undo brings back the playlist's name too.

With the mouse, drag a song up or down the queue to move it. Drag the `╎` marks in the
heading row to make a column wider or narrower; the last text column gives way. Click the
list icon at the queue's top left (or press `C`) to show or hide columns: number, source
icon, artist, album (with its year), format and time. Column choices are saved.

In the browser, typing goes into the search box. Results are grouped into artists, albums and
songs; while more than one group shows, each lists its best matches, and `1` `2` `3` show or
hide a group. `Tab` moves between the search box, results and songs. `Enter` (or a
double-click) adds the song or album to the end of the queue without playing it; on an artist
it shows their songs. The browser never replaces the queue, so an open playlist is safe.

In Search, type in the box at the top; `1` `2` `3` (or clicking them) switch Local, Spotify
and YouTube Music on and off. Each provider's results get their own box; `Tab` jumps to the
next one. `Enter` plays a song right away (the queue carries on after it), `a` adds it, `A`
adds everything that provider found. `Esc` or the × closes Search.

Colours come from Omarchy's current theme and follow theme switches while the player is open.
Or pick any installed Omarchy theme for OMAESTRO alone: Settings (`F2`) → Appearance → Theme
lists them all, the chosen one ► between arrows ◄. `←` `→` on the Theme row try them in turn,
or click one; each shows straight away and is saved. A picked
theme paints its own background, so light ones work in a dark terminal. Everforest and White
have very faint dim text (headings, times), as they do everywhere.

## Settings

Most settings are in the app (gear button). They live in `~/.config/omaestro/config.toml`:

```toml
library_paths = ["~/WolfMusic/media"]   # folders to scan
scan_threads = 16                       # files read at once while scanning
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
theme = "omarchy"                       # Omarchy's theme, or an installed one: "tokyo-night"

[queue]                                 # the queue's columns (set with the mouse or C)
hidden = []                             # e.g. ["number", "format"]
title = 30                              # widths of the text columns; the last one shown
artist = 22                             # takes whatever room is left
album = 28

[outputs]                               # the picker's stars and hidden ones
favourites = []
hidden = []

[sonos]
speakers = []                           # found by themselves, then remembered
media_port = 47100
never = []                              # speaker ids never to send anything to
```

## Versions

The version is Release.Version.Fix (shown bottom right, and by `omaestro --version`). What
changed in each one is in [CHANGELOG.md](CHANGELOG.md); ideas for later are in
[BACKLOG.md](BACKLOG.md).

## Files it uses

- `~/.cache/omaestro/library.db`: the library index. Safe to delete; the next start rebuilds
  it, which takes about a minute.
- `~/.local/share/omaestro/playlists/*.jspf`: your playlists (JSPF, a standard format).
- `~/.config/omaestro/spotify.json`, `soloist.json`, `youtube.json`, `ytm-cookies.txt`: sign-ins,
  readable only by you. Deleting one signs out.
- `~/.local/share/omaestro/soloist/`: Spotify Soloist, downloaded by OMAESTRO.
- `~/.cache/omaestro/sonos/`: songs converted or downloaded for Sonos, cleared after a week.
- `~/.cache/omaestro/sonos.log`: every command that changed something on a speaker.
- `~/.cache/omaestro/sonos-spotify-account`: Sonos's number for your Spotify account.
- `~/.cache/omaestro/soloist-last.json`: Soloist's latest report on the song playing, kept
  for checking what it says about stream quality.
- `$XDG_RUNTIME_DIR/omaestro/mpv.sock`: OMAESTRO's control line to mpv while it runs.

## Measured on the WolfMusic NAS (117,707 tracks)

- First scan: 69 s
- Refresh with nothing changed: 0.3–1.3 s
- Search: ~2 ms

## Tests

```
cargo test                                      # silent: mpv plays into a null output
OMAESTRO_PREVIEW=search cargo test preview -- --ignored --nocapture   # prints a screen from your real library
cargo test live_ -- --ignored --nocapture       # online checks: YouTube search and playback (silent), reading the Sonos household
```
