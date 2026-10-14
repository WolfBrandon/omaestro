# Security

## Reporting a problem

Please report security problems privately, through GitHub's
[private vulnerability reporting](https://github.com/WolfBrandon/omaestro/security/advisories/new)
for this repository, rather than in a public issue.

## What OMAESTRO keeps, and where

- **Sign-ins**, in `~/.config/omaestro`, readable only by you (mode 0600):
  - `spotify.json`: Spotify's tokens for your own Spotify app.
  - `soloist.json`: your Soloist API key.
  - `youtube.json` and `ytm-cookies.txt`: a copy of your browser's YouTube cookies, made by
    yt-dlp when you connect YouTube Music.
  - `lastfm.json`: your Last.fm API key and secret, and the session key from allowing
    OMAESTRO.
  - `listenbrainz.json`: your ListenBrainz user token.
- **A file server** on port 47100, while OMAESTRO plays on Sonos. It serves only the songs
  handed to a speaker, under random addresses, and everything else gets "not found".
- **Nothing is sent anywhere else.** No telemetry.

Signing out in Settings (or deleting the file) removes a sign-in.
