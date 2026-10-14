# Security

## Reporting a problem

Please report security problems privately, through GitHub's
[private vulnerability reporting](https://github.com/WolfBrandon/omaestro/security/advisories/new)
for this repository, rather than in a public issue.

## What OMAESTRO keeps, and where

- **Sign-ins**, in `~/.config/omaestro` (a folder only you can open), each readable only by
  you (mode 0600):
  - `spotify.json`: Spotify's tokens for your own Spotify app.
  - `soloist.json`: your Soloist API key.
  - `youtube.json` and `ytm-cookies.txt`: a copy of your browser's YouTube cookies. When you
    connect YouTube Music, yt-dlp exports all the browser's cookies into a private folder;
    OMAESTRO keeps only YouTube's and deletes the rest. (Before 0.9.7 the whole export was
    kept; OMAESTRO cuts an older file down the next time it starts.)
  - `lastfm.json`: your Last.fm API key and secret, and the session key from allowing
    OMAESTRO.
  - `listenbrainz.json`: your ListenBrainz user token.
- **Your plays**, in `~/.local/share/omaestro`:
  - `plays.db`: every play, kept on this computer.
  - `scrobbles.json` (readable only by you): plays waiting to go to Last.fm or ListenBrainz.
    What's sent is the song's title, artist, album, length, track number, when you played
    it, and the MusicBrainz id and ISRC if tagged; for Spotify and YouTube Music songs also
    their web address. Never a file's name or folder.
- **A file server** on port 47100, for Sonos. It opens the first time you play on Sonos and
  stays open until you quit. It:
  - offers only the song playing now, under a random address, and nothing while Sonos is
    stopped;
  - answers only the speakers in your Sonos household; other connections are closed (and
    noted in `~/.cache/omaestro/sonos.log`);
  - serves only files inside your music folders, and OMAESTRO's own converted copies, checked
    again as each request is served;
  - answers a few connections at a time, with short request lines;
  - is never written down by address: `sonos.log` (readable only by you) records commands
    without song addresses.
- **Nothing is sent anywhere else.** No telemetry. Plays go only to the services you
  connect.

Signing out in Settings (or deleting the file) removes a sign-in.
