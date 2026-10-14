# Backlog

Ideas for later, roughly in the order they came up. Done ones move to
[CHANGELOG.md](CHANGELOG.md).

## File Info

A small pop-up with everything about the highlighted song. It would open from the queue,
the browser and Search, by key or right-click.

What it would show:
- **Song:** title, artist, album artist, album (year), track and disc number, genre, length.
- **File:** where it lives (copyable), size, format, bit depth, sample rate, bitrate,
  ReplayGain.
- **Spotify and YouTube Music songs:** the song's link instead of a file path.

## Sort songs by popularity

Order an artist's songs (in the browser and Search) by how popular they are, not by name.
Where the numbers could come from:
- **Last.fm:** listener and play counts, and an artist's top songs in order. Needs a free API
  key.
- **ListenBrainz:** popularity by MusicBrainz ID, which many tagged files already carry. No
  key needed.
- **Spotify's own popularity score,** for Spotify songs.

Look-ups would be cached in the library index, so sorting stays instant.

## Track plays, maybe scrobble

Count plays and remember when each song last played, in the library index. That makes
"most played" and "recently played" lists possible. Optionally send each play to Last.fm
(sign in once) or ListenBrainz (a token from your profile). A play would count after half
the song or four minutes, Last.fm's rule.

## Configure the output rate

Today PipeWire picks the rate, and the Now Playing badge shows "→ 48k" when it resamples.
OMAESTRO could make each file play at its own rate instead:
- **Permanently:** let PipeWire switch rates (allowed rates in
  `~/.config/pipewire/pipewire.conf.d/`).
- **Only while OMAESTRO plays:** ask PipeWire for the file's rate (`clock.force-rate`) and set
  it back afterwards.

Either way it affects everything else playing on that device at the same time.

## Sources: show connection status

A coloured dot and a word for each source, in Settings → Sources and on Search's provider
pills:
- **Local:** the music folder is mounted, or not.
- **Spotify:** signed in, sign-in expired, or Soloist not running.
- **YouTube Music:** signed in, or the browser login no longer works.
- **Sonos:** speakers reachable, or not.

## Update without closing and reopening

When `./install.sh` puts a new version in place, the running OMAESTRO would notice and
restart itself into it. It would keep the queue, the song and position, the output and the
volume, with only a moment's gap in the sound. It could wait for a pause, or ask first in the
status line ("v0.7.0 is ready: press U to switch").
