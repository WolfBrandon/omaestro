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

What hasn't been tried on the real thing yet: Spotify playback (needs your Spotify app and
Soloist key), playing on Sonos (needs the firewall line below), and minimizing (needs
OMAESTRO in its own window). See "First-time setup".

## Run it

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
   signed in to music.youtube.com with. Your desktop may ask to unlock the keyring.
3. **Sonos**: speakers fetch local files and YouTube songs from OMAESTRO on port 47100. Omarchy's
   firewall blocks that until you allow it for your home network (one time, needs your
   password):
   ```
   sudo ufw allow from 192.168.1.0/24 to any port 47100 proto tcp comment 'OMAESTRO for Sonos'
   ```
   Spotify on Sonos needs Spotify linked in the Sonos app; it doesn't need the firewall line.

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
| `o` | Where to play: this computer's outputs or a Sonos group. `f` stars one, `h` hides one, `H` shows hidden ones |
| `m` | Minimize into the `special:omaestro` scratchpad; the tray icon or a media widget brings it back |
| `F2` | Settings |
| `r` | Check the music folders for changes |
| `q` | Quit (the music stops) |

In the queue: `d` removes, `J`/`K` move, `u` undoes, `c` clears, `s` saves, `S` saves as,
`O` opens a playlist (saved ones, or your Spotify and YouTube Music playlists).

In the browser, typing goes into the search box. Results are grouped into artists, albums and
songs; while more than one group shows, each lists its best matches, and `1` `2` `3` show or
hide a group. `Tab` moves between the search box, results and songs; `Enter` on an album or
song plays its album from there, on an artist it shows their songs; `a` adds to the queue.

In Search, `Enter` plays a song right away (the queue carries on after it), `a` adds it,
`A` adds everything that source found.

Colours come from the current Omarchy theme and follow theme switches while the player is open.

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
album_art = true
visualizer = true

[outputs]                               # the picker's stars and hidden ones
favourites = []
hidden = []

[sonos]
speakers = []                           # found by themselves, then remembered
media_port = 47100
never = []                              # speaker ids never to send anything to
```

## Files it uses

- `~/.cache/omaestro/library.db`: the library index. Safe to delete; the next start rebuilds
  it, which takes about a minute.
- `~/.local/share/omaestro/playlists/*.jspf`: your playlists (JSPF, a standard format).
- `~/.config/omaestro/spotify.json`, `soloist.json`, `youtube.json`, `ytm-cookies.txt`: sign-ins,
  readable only by you. Deleting one signs out.
- `~/.local/share/omaestro/soloist/`: Spotify Soloist, downloaded by OMAESTRO.
- `~/.cache/omaestro/sonos/`: songs converted or downloaded for Sonos, cleared after a week.
- `~/.cache/omaestro/sonos.log`: every command that changed something on a speaker.
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
