# Contributing

Thanks for looking. OMAESTRO is a hobby project, so replies may take a while, but issues and
pull requests are welcome.

## Reporting a bug

Open an issue with:
- what you did, and what happened instead;
- your OMAESTRO version (bottom right, or `omaestro --version`), distribution and terminal;
- for Spotify playback problems, `~/.cache/omaestro/soloist.log`. For Sonos, the relevant
  lines of `~/.cache/omaestro/sonos.log`. Check them for anything you'd rather not share
  first; they hold song links and speaker ids, never passwords.

## Making a change

```
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test
```

All three run on every pull request. Tests that need mpv or ffmpeg skip when those aren't
installed.

A few habits the code follows:
- **Simple and working over clever.** Few dependencies; the standard library first.
- **Comments say why,** in plain words.
- **Nothing that makes sound in tests.** mpv plays into a null output, and tests never send
  playback commands to real speakers. Tests that touch real services are `#[ignore]`d, and
  named `live_…`.
- **Sonos safety rules** (`permit` in `src/sonos/mod.rs`) are tested on their own. Any new
  command needs to be added there deliberately.
- **Changelog:** add an entry to `CHANGELOG.md` for anything users would notice.
