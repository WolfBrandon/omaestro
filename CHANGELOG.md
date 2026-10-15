# Changelog

Versions are Release.Version.Fix: the middle number goes up for new features, the last for
fixes only. The first moves when a release is called.

## 0.9.11 — 2026-10-14

- **Star ratings.**
  - In the queue, `1` to `5` give the highlighted song that many stars, or all the
    selected songs at once. `0` takes the rating off.
  - Or click a star in the new **Rating** column; clicking the rating a song already has
    takes it off. An unrated song shows faint stars when it's highlighted: click the song,
    then a star.
  - A rating belongs to the song, local, Spotify or YouTube Music, so it shows wherever the
    song is, and in File Info.
  - Ratings are kept in `plays.db` beside your plays. They're never written into your music
    files or sent anywhere.
  - The column can be hidden like the others (`C`).
- **The hint bar keeps `? all keys` in view:** it now comes before the keys that drop off
  first on a narrow screen.

## 0.9.10 — 2026-10-14

- **Select several songs in the queue,** to remove or move them together.
  - `Shift+↑`/`↓` (or `Shift+Home`/`End`) selects from the highlighted song on; `Ctrl+A`
    selects the whole queue.
  - With the mouse, `Ctrl+click` adds a song or drops it, and `Alt+click` selects everything
    up to the song clicked. `Shift+click` can't do it: the terminal keeps Shift for
    selecting text.
  - `d` removes the selected songs, and `J`/`K` move them up or down together. Dragging
    any of them moves them all, and they land side by side.
  - Each is one step for `Ctrl+Z`. `Esc`, a plain click or arrow, or any other change to the
    queue lets go of the selection. The queue's heading says how many are selected.

## 0.9.9 — 2026-10-14

- **`u` no longer undoes.** It sat one Shift away from `U` (switch to a new version), so a
  slip took songs out of the queue, one per press, as each "added" was undone. Undo is
  `Ctrl+Z` alone now.
- **Redo:** `Ctrl+Y` puts back what undo took away, a step at a time, until the queue is
  edited again. After an undo, the status line says so.

## 0.9.8 — 2026-10-14

- **Plays are counted.**
  - A song counts once you've heard half of it or 4 minutes, whichever comes first. Songs of
    30 seconds or less never count, and pauses, loading and skipping ahead don't add to it.
  - Every play is kept in `~/.local/share/omaestro/plays.db`, from any source and on any
    output. Picking a song up after switching outputs, or after an update, is the same
    play.
- **Recently played and Most played** are at the top of your saved playlists (`O`, or the
  Open button), once there are plays; each opens into the queue like a playlist. File Info
  says how many times a song has been played, and when last.
- **Scrobbling** to Last.fm and ListenBrainz, whichever you've connected.
  - "Now playing" is sent as a song starts, and the play once it counts.
  - Plays wait on disk while offline or while a service is down, then go oldest first.
  - Settings → Integrations shows how each is doing: "sending plays", "3 waiting to send" or
    "sign in again".
  - **Spotify plays** (on by default) can be switched off there, for anyone whose Spotify
    already scrobbles.
  - What's sent: title, artist, album, length, track number and when; the MusicBrainz id and
    ISRC if tagged; a streamed song's web address. Never a file's name or folder.
- **Tests no longer touch your real settings.** Running OMAESTRO's tests used your real
  settings folder, and one of them signed a real Last.fm account out. Tests now keep their
  own folder in `/tmp`.

## 0.9.7 — 2026-10-14

Fixes from three independent code reviews (Grok, Antigravity, Codex).

- **YouTube cookies:** connecting YouTube Music kept your browser's whole cookie export, every
  site's, where only YouTube's are used.
  - Now the export goes into a private folder, only YouTube's are kept, and the rest are
    deleted straight away.
  - An existing file is cut down the next time OMAESTRO starts.
- **Playlists are never written over by another:**
  - Renaming one to a name that's taken, or naming an unsaved queue that way, now says so
    instead.
  - Names that share a file name count as taken: "Road/Trip" and "Road:Trip" are both
    `Road-Trip.jspf`.
  - A Spotify or YouTube Music playlist with the same name as one of yours opens as
    "Name (Spotify)".
- **An interrupted library scan** (quitting during "Finishing", say) left search and the album
  list out of date until some file changed. The next scan now finishes the job.
- **Unreadable files are retried.** A file whose tags couldn't be read (the NAS blinked) is
  tried again at the next scan, instead of staying listed by its file name.
- **A song that can't start on Sonos** now leaves the player stopped on that song, with the
  reason. Space tries it again; before, Space did nothing.
- **The file server for Sonos:**
  - It offers only the song playing now, and nothing once Sonos stops.
  - It answers only your speakers.
  - It serves only files inside your music folders (a playlist file can't point it anywhere
    else, nor can a file swapped for a link).
  - It takes a few connections at a time, with short requests.
  - Song addresses are no longer written to `sonos.log`, which is now readable only by you and
    starts over past 1 MB.
  - SECURITY.md said the server runs only while playing on Sonos; it now says what it does.
- **Smaller fixes:**
  - Sonos is only ever told to play OMAESTRO's own song links or Spotify links.
  - Spotify and YouTube song ids are checked before they're used in a link, a file name or a
    command.
  - Spotify's "next page" links are only followed to Spotify itself, since your sign-in goes
    with them.
  - A stray request to the Spotify sign-in page no longer cancels a sign-in in progress.
  - Random addresses and sign-in codes fail safely if the system's random source can't be
    read, instead of coming out the same every time.
  - `~/.config/omaestro`, and the player's control folder, are readable only by you.
  - A music folder whose check hangs (a NAS gone away) isn't checked again until that check
    returns, so stuck checks don't pile up.

## 0.9.6 — 2026-10-14

- **Updating is your choice:** Settings → About → Auto-update, on or off.
  - **On** (as before): a newly installed version is switched to by itself, straight away if
    nothing's playing, otherwise at the next pause.
  - **Off:** OMAESTRO only says so, and switches when you choose Update now, or press `U`.
- **The version in the bottom right** is all that's there now. It's green when up to date and
  yellow when a new version is waiting; clicking it opens Settings → About, where Update now
  appears. "Designed with OTIS" moved to About.

## 0.9.5 — 2026-10-14

- **Settings → Integrations**, a new section for where your plays will be sent.
  - **Last.fm:** make a free API account, paste its key and secret, then allow OMAESTRO on
    Last.fm's page. The screen moves on by itself once you have.
  - **ListenBrainz:** paste the token from your settings page; it's checked with
    ListenBrainz.
  - Both are kept readable only by you, and sign out from the same rows.
  - Sending plays (scrobbling) comes next.
- Qobuz is on the backlog for after 1.0.0, with why: its API is partner-only, and Qobuz had
  the open-source client that worked around that shut down.

## 0.9.4 — 2026-10-14

- **Update in place.** When `./install.sh` puts a new version where OMAESTRO was started from,
  a running OMAESTRO notices within a few seconds.
  - **Nothing playing, or paused:** it switches straight away, in the same window.
  - **Playing:** the status line offers `U` to switch now and carry on playing from the same
    place. Otherwise it switches at the next pause.
  - The queue, the song and the place in it come along. A new file only counts once it
    answers `--version`, so a half-built one is never started.

## 0.9.3 — 2026-10-14

- **File Info:** `i`, or a right-click on a song, in the queue, the browser or Search.
  - **The song:** album (year), album artist, track and disc, genre, length, ISRC and its
    MusicBrainz ID.
  - **For files:** the path, format, bitrate, channels, size and ReplayGain. These are read
    from the file in the background, with a time limit.
  - **For Spotify and YouTube Music songs:** their link.
  - `c` copies the path or link, and `o` opens the song's folder or page. Opened from Search,
    closing it goes back to Search.

## 0.9.2 — 2026-10-14

- **Connection status**, the first item on the road to 1.0.0: a coloured dot and a word for
  each source, in Settings (Sources, and Sonos under Outputs) and on Search's provider pills.
  - **Local:** the music folders are ready, unreadable, or not answering (a NAS gone away).
  - **Spotify:** signed in, sign-in expired, can't reach Spotify, or signed in but not set up
    to play here.
  - **YouTube Music:** signed in, or the browser login no longer works.
  - **Sonos:** how many groups' speakers answer.
  - **How it checks:** in the background, at the start, every two minutes, and when Settings
    opens. Each check has a time limit, so a hung NAS never holds anything up.

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
