# Road to 1.0.0

Everything here is in 1.0.0. Until it's all done, OMAESTRO stays at 0.9.x: each piece ships as
the next 0.9 version, and when the list is all ticked, that release is 1.0.0. Done items are
ticked here and described in [CHANGELOG.md](CHANGELOG.md).

In the order they'll be built:

- [x] **Connection status**: a dot and a word for each source (0.9.2).
- [x] **File Info**: everything about a song, in a pop-up (0.9.3).
- [x] **Update in place**: switch to a new version without closing and reopening (0.9.4).
- [x] **Track plays and scrobbling**: play counts and recently played, sent to Last.fm and
  ListenBrainz (sign-in 0.9.5, plays and scrobbling 0.9.8).
- [x] **Several queue songs at once**: select them to remove or move together (0.9.10).
- [x] **Star ratings**: 1 to 5 stars for any song, in the queue and File Info (0.9.11).
- [x] **Roomier stars**: a space between each star (0.9.12).
- [x] **New playlist**: a New button and Ctrl+N; saves a named playlist first (0.9.13).
- [x] **Several at once in the browser and Search**: select, then add them together (0.9.14).
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

## Output rate

**While OMAESTRO plays,** it switches the output to each file's sample rate (`clock.force-rate`),
and puts it back when playback stops or OMAESTRO quits.
- **A setting** turns it on or off.
- **Only rates the device supports** are asked for; otherwise PipeWire resamples as now.
- **The Now Playing badge** shows when it matches.
- **Other sound** on that device follows the same rate meanwhile.

Today PipeWire runs everything at 48 kHz, so 44.1 and 96 kHz files are resampled.

# After 1.0.0

## Hardening, from the October 2026 code reviews

What wasn't fixed in 0.9.7, and why:
- **Finding Sonos speakers by their own announcement (SSDP)** before falling back to
  checking every address on the network for port 1400. Found speakers are remembered, so the
  sweep only runs until then; still the quieter way.
- **Checking the Soloist download.** Spotify publishes no checksum to compare it with; tar
  already refuses files that would land outside its folder.
- **Album art from playlist files:** fetch only from known picture hosts, or don't follow
  redirects.

## Qobuz

Qobuz as a source, like Spotify: search, your playlists and favourites, hi-res playback here
and on Sonos. Not for 1.0.0, because there's no open way in yet:
- **The API is partner-only.** The app ID and secret come from Qobuz on request
  (api@qobuz.com).
- **The unofficial route is off the table.** In September 2026 Qobuz asked QBZ, the
  open-source Linux client that used the web player's own credentials, to shut down, and it
  did. A public project doing the same would get the same request, and it breaks Qobuz's
  terms for the account using it.
- **What's left:**
  - Apply to Qobuz for access as OMAESTRO's developer.
  - Without any Qobuz API, play the Qobuz albums and playlists saved as Sonos favourites,
    on Sonos only, through Sonos's own Qobuz link (Sonos service 31).
