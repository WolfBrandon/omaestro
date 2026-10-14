# Road to 1.0.0

Everything here is in 1.0.0. Until it's all done, OMAESTRO stays at 0.9.x: each piece ships as
the next 0.9 version, and when the list is all ticked, that release is 1.0.0. Done items are
ticked here and described in [CHANGELOG.md](CHANGELOG.md).

In the order they'll be built:

- [x] **Connection status**: a dot and a word for each source (0.9.2).
- [ ] **File Info**: everything about a song, in a pop-up.
- [ ] **Update in place**: switch to a new version without closing and reopening.
- [ ] **Track plays and scrobbling**: play counts and recently played, sent to Last.fm and
  ListenBrainz.
- [ ] **Sort by popularity**: an artist's songs, most popular first.
- [ ] **Output rate**: each file at its own sample rate, instead of resampling.
- [ ] **Taskbar companion for Omarchy**: what's playing and where, in the bar.

## Taskbar companion for Omarchy

A plugin for Omarchy's shell (Quickshell, the same kind as the `brandon.*` plugins), in the
bar's right side, just before the tray:
- **The widget:** what's playing (title · artist) and a play/pause button.
- **Click:** a panel with the cover, controls, position, volume, and the output picker (this
  computer's outputs and the Sonos rooms).
- **Right-click:** brings OMAESTRO's window back.
- **How it talks to OMAESTRO:** the shell's media service (MPRIS) for the song and controls.
  For what MPRIS can't do (the outputs, and switching them), `omaestro ctl …` talks to the
  running OMAESTRO over a small local socket.
- **Setup:** `install.sh` copies the plugin into `~/.config/omarchy/plugins/omaestro` and adds
  it to the bar's layout in `~/.config/omarchy/shell.json`, backing that up first.

## File Info

A small pop-up with everything about a song. It opens with `i` or a right-click, in the queue,
the browser and Search.

What it would show:
- **Song:** title, artist, album artist, album (year), track and disc number, genre, length.
- **File:** where it lives (copyable), size, format, bit depth, sample rate, bitrate,
  ReplayGain.
- **Spotify and YouTube Music songs:** the song's link instead of a file path.

## Sort by popularity

- **The toggle:** opening an artist in the browser, a key (and a clickable label) switches
  their songs between album order and most popular first. The choice is remembered.
- **Where the numbers come from** is a choice in Settings:
  - **ListenBrainz:** looks songs up by their MusicBrainz IDs, which tagged files carry; others
    by name. No key needed.
  - **Last.fm:** listener and play counts, by artist and title. Uses the Last.fm key from
    scrobbling.
  - **Your own play counts:** no outside service.
- Look-ups are cached in the library index, so sorting stays instant.

## Track plays and scrobbling

- **Play counts and last played** for every song (local, Spotify and YouTube Music), kept in
  the library index. That gives "most played" and "recently played" lists.
- **Scrobbling to both Last.fm and ListenBrainz,** each switched on in Settings → Sources:
  - **Last.fm:** a free API key and secret you create, then sign in once in the browser.
  - **ListenBrainz:** paste the token from your ListenBrainz profile.
- **The rules:** a play counts after half the song or four minutes (Last.fm's rule), and
  "now playing" is sent as a song starts. Plays made offline are sent later.

## Output rate

**While OMAESTRO plays,** it switches the output to each file's sample rate (`clock.force-rate`),
and puts it back when playback stops or OMAESTRO quits.
- **A setting** turns it on or off.
- **Only rates the device supports** are asked for; otherwise PipeWire resamples as now.
- **The Now Playing badge** shows when it matches.
- **Other sound** on that device follows the same rate meanwhile.

Today PipeWire runs everything at 48 kHz, so 44.1 and 96 kHz files are resampled.

## Update in place

When `./install.sh` puts a new version in place, a running OMAESTRO notices:
- **If nothing's playing** (or it's paused), it restarts itself into the new version straight
  away.
- **While playing,** the status line says "v0.9.x is ready: U switches", and it switches at
  the next pause.
- **Either way** it keeps the queue, the song and position, the output and the volume.

