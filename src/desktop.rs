//! Fitting in with the desktop:
//! - MPRIS, the standard way Linux media players are controlled: the keyboard's media keys
//!   and the bar's media widget see OMAESTRO and can play, pause and skip;
//! - a tray icon with a small menu;
//! - minimizing the terminal window into a Hyprland scratchpad ("special:omaestro"), and
//!   bringing it back.
//!
//! The first two talk over D-Bus on their own threads; what they ask for arrives in the main
//! loop as `Cmd`s.

use mpris_server::{LoopStatus, Metadata, PlaybackStatus, Player, Time, TrackId};
use serde_json::Value;
use std::process::Command;
use std::sync::mpsc;
use std::thread;

/// What the desktop asks OMAESTRO to do.
#[derive(Debug, Clone, PartialEq)]
pub enum Cmd {
    Play,
    Pause,
    PlayPause,
    Stop,
    Next,
    Previous,
    /// Seek by this many microseconds.
    SeekBy(i64),
    /// Seek to this many microseconds into the song.
    SetPosition(i64),
    SetVolume(f64),
    /// Show the window.
    Raise,
    ToggleWindow,
    Quit,
}

// ---- MPRIS ----------------------------------------------------------------------------

/// What the main loop tells the MPRIS thread.
pub enum Update {
    Metadata(Metadata),
    Status(PlaybackStatus),
    /// The position, without telling listeners (they keep their own clock between seeks).
    Position(Time),
    /// The position jumped (a seek or a new song): listeners are told.
    Seeked(Time),
    Volume(f64),
}

/// The line to the MPRIS thread. Dropping it takes OMAESTRO off the bus.
pub struct Mpris {
    tx: async_channel::Sender<Update>,
}

impl Mpris {
    pub fn send(&self, update: Update) {
        let _ = self.tx.send_blocking(update);
    }
}

/// The window's app-id when started from the app launcher, and the launcher entry's name.
pub const APP_ID: &str = "org.omarchy.omaestro";

/// What the app launcher runs (`omaestro launch`): brings OMAESTRO back if it's already
/// running, even tucked away in the scratchpad, or else opens it in a new terminal window.
pub fn launch() -> anyhow::Result<()> {
    use std::os::unix::process::CommandExt;
    let raised = Command::new("busctl")
        .args([
            "--user",
            "call",
            "org.mpris.MediaPlayer2.omaestro",
            "/org/mpris/MediaPlayer2",
            "org.mpris.MediaPlayer2",
            "Raise",
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    if raised {
        return Ok(());
    }
    // xdg-terminal-exec opens the default terminal (foot here) running OMAESTRO.
    let me = std::env::current_exe()?;
    let error = Command::new("xdg-terminal-exec")
        .arg(format!("--app-id={APP_ID}"))
        .arg("--title=OMAESTRO")
        .arg("-e")
        .arg(me)
        .exec();
    Err(anyhow::anyhow!(
        "couldn't open a terminal with xdg-terminal-exec: {error}"
    ))
}

/// Registers OMAESTRO as `org.mpris.MediaPlayer2.omaestro`.
pub fn start_mpris(commands: mpsc::Sender<Cmd>) -> Result<Mpris, String> {
    let (tx, rx) = async_channel::unbounded::<Update>();
    let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
    thread::Builder::new()
        .name("mpris".into())
        .spawn(move || {
            futures_lite::future::block_on(async move {
                // A second OMAESTRO would otherwise wait silently for the name; the MPRIS spec
                // says to add ".instance<pid>" instead.
                let suffix = if name_taken("org.mpris.MediaPlayer2.omaestro").await {
                    format!("omaestro.instance{}", std::process::id())
                } else {
                    "omaestro".to_string()
                };
                let player = match Player::builder(&suffix)
                    .identity("OMAESTRO")
                    .desktop_entry(APP_ID)
                    .can_play(true)
                    .can_pause(true)
                    .can_go_next(true)
                    .can_go_previous(true)
                    .can_seek(true)
                    .can_control(true)
                    .can_raise(true)
                    .can_quit(true)
                    .loop_status(LoopStatus::None)
                    .build()
                    .await
                {
                    Ok(p) => p,
                    Err(e) => {
                        let _ = ready_tx.send(Err(e.to_string()));
                        return;
                    }
                };
                let _ = ready_tx.send(Ok(()));

                let send = |cmd: Cmd| {
                    let c = commands.clone();
                    move |_: &Player| {
                        let _ = c.send(cmd.clone());
                    }
                };
                player.connect_play(send(Cmd::Play));
                player.connect_pause(send(Cmd::Pause));
                player.connect_play_pause(send(Cmd::PlayPause));
                player.connect_stop(send(Cmd::Stop));
                player.connect_next(send(Cmd::Next));
                player.connect_previous(send(Cmd::Previous));
                player.connect_raise(send(Cmd::Raise));
                player.connect_quit(send(Cmd::Quit));
                let c = commands.clone();
                player.connect_seek(move |_, offset| {
                    let _ = c.send(Cmd::SeekBy(offset.as_micros()));
                });
                let c = commands.clone();
                player.connect_set_position(move |_, _, position| {
                    let _ = c.send(Cmd::SetPosition(position.as_micros()));
                });
                let c = commands;
                player.connect_set_volume(move |_, v| {
                    let _ = c.send(Cmd::SetVolume(v));
                });

                let updates = async {
                    while let Ok(update) = rx.recv().await {
                        let _ = match update {
                            Update::Metadata(m) => player.set_metadata(m).await,
                            Update::Status(s) => player.set_playback_status(s).await,
                            Update::Position(t) => {
                                player.set_position(t);
                                Ok(())
                            }
                            Update::Seeked(t) => {
                                player.set_position(t);
                                player.seeked(t).await
                            }
                            Update::Volume(v) => player.set_volume(v).await,
                        };
                    }
                };
                futures_lite::future::or(player.run(), updates).await;
            });
        })
        .map_err(|e| e.to_string())?;
    match ready_rx.recv() {
        Ok(Ok(())) => Ok(Mpris { tx }),
        Ok(Err(e)) => Err(e),
        Err(_) => Err("the media-key service stopped before starting".into()),
    }
}

async fn name_taken(name: &str) -> bool {
    use mpris_server::zbus::{Connection, fdo::DBusProxy, names::BusName};
    let Ok(conn) = Connection::session().await else {
        return false;
    };
    let Ok(proxy) = DBusProxy::new(&conn).await else {
        return false;
    };
    let Ok(bus_name) = BusName::try_from(name) else {
        return false;
    };
    proxy.name_has_owner(bus_name).await.unwrap_or(false)
}

/// MPRIS details for a song. `n` makes each song's id unique.
pub fn metadata(track: &crate::library::Track, n: u64, art: Option<&std::path::Path>) -> Metadata {
    let mut m = Metadata::builder()
        .title(track.title.clone())
        .artist([track.artist.clone()])
        .album(track.album.clone())
        .length(Time::from_millis(track.duration_ms));
    if let Ok(id) = TrackId::try_from(format!("/org/omaestro/track/{n}")) {
        m = m.trackid(id);
    }
    if let Some(path) = art {
        m = m.art_url(crate::playlist::file_uri(&path.display().to_string()));
    } else if track.art_url.starts_with("https://") {
        m = m.art_url(track.art_url.clone());
    }
    m.build()
}

// ---- the tray icon ---------------------------------------------------------------------

pub struct Tray {
    tx: mpsc::Sender<Cmd>,
    pub song: String,
    pub playing: bool,
    pub hidden: bool,
}

fn menu_item(label: &str, icon: &str, cmd: Cmd) -> ksni::MenuItem<Tray> {
    ksni::menu::StandardItem {
        label: label.into(),
        icon_name: icon.into(),
        activate: Box::new(move |t: &mut Tray| {
            let _ = t.tx.send(cmd.clone());
        }),
        ..Default::default()
    }
    .into()
}

impl ksni::Tray for Tray {
    fn id(&self) -> String {
        "omaestro".into()
    }

    fn title(&self) -> String {
        "OMAESTRO".into()
    }

    fn icon_name(&self) -> String {
        "multimedia-player".into()
    }

    fn category(&self) -> ksni::Category {
        ksni::Category::ApplicationStatus
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            title: "OMAESTRO".into(),
            description: self.song.clone(),
            icon_name: "multimedia-player".into(),
            icon_pixmap: Vec::new(),
        }
    }

    /// A left click shows or hides the window.
    fn activate(&mut self, _x: i32, _y: i32) {
        let _ = self.tx.send(Cmd::ToggleWindow);
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        let song = if self.song.is_empty() {
            "Nothing playing".to_string()
        } else {
            self.song.clone()
        };
        vec![
            ksni::menu::StandardItem {
                label: song,
                enabled: false,
                ..Default::default()
            }
            .into(),
            ksni::MenuItem::Separator,
            menu_item(
                if self.hidden {
                    "Show OMAESTRO"
                } else {
                    "Hide OMAESTRO"
                },
                "window",
                Cmd::ToggleWindow,
            ),
            menu_item(
                if self.playing { "Pause" } else { "Play" },
                if self.playing {
                    "media-playback-pause"
                } else {
                    "media-playback-start"
                },
                Cmd::PlayPause,
            ),
            menu_item("Next", "media-skip-forward", Cmd::Next),
            menu_item("Previous", "media-skip-backward", Cmd::Previous),
            ksni::MenuItem::Separator,
            menu_item("Quit", "application-exit", Cmd::Quit),
        ]
    }
}

/// Puts the icon in the bar's tray. Fails quietly if there's no tray.
pub fn start_tray(commands: mpsc::Sender<Cmd>) -> Option<ksni::blocking::Handle<Tray>> {
    use ksni::blocking::TrayMethods;
    Tray {
        tx: commands,
        song: String::new(),
        playing: false,
        hidden: false,
    }
    .spawn()
    .ok()
}

// ---- minimizing (Hyprland) -----------------------------------------------------------

/// The scratchpad OMAESTRO hides in. Not "music": that one is already in use here.
const SCRATCHPAD: &str = "special:omaestro";

/// The terminal window OMAESTRO runs in, by its Hyprland address ("0x55…").
pub struct Window {
    address: String,
}

fn hyprctl(args: &[&str]) -> Option<String> {
    let out = Command::new("hyprctl").args(args).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// A process's parent, from /proc/<pid>/stat (the name in brackets can contain spaces).
fn parent_of(pid: u32) -> Option<u32> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let after_name = &stat[stat.rfind(')')? + 1..];
    after_name.split_whitespace().nth(1)?.parse().ok()
}

impl Window {
    /// Finds the window: walks up from OMAESTRO to its terminal and matches it against
    /// Hyprland's window list. None outside Hyprland.
    pub fn find() -> Option<Window> {
        std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE")?;
        let clients: Value = serde_json::from_str(&hyprctl(&["clients", "-j"])?).ok()?;
        let windows: Vec<(u64, String)> = clients
            .as_array()?
            .iter()
            .filter_map(|c| Some((c["pid"].as_u64()?, c["address"].as_str()?.to_string())))
            .collect();
        let mut pid = std::process::id();
        for _ in 0..12 {
            pid = parent_of(pid)?;
            if pid <= 1 {
                return None;
            }
            if let Some((_, address)) = windows.iter().find(|(p, _)| *p == pid as u64) {
                return Some(Window {
                    address: address.clone(),
                });
            }
        }
        None
    }

    /// Whether the window is tucked away in the scratchpad now.
    pub fn hidden(&self) -> bool {
        let Some(clients) = hyprctl(&["clients", "-j"]) else {
            return false;
        };
        let clients: Value = serde_json::from_str(&clients).unwrap_or_default();
        clients.as_array().into_iter().flatten().any(|c| {
            c["address"].as_str() == Some(&self.address)
                && c["workspace"]["name"].as_str() == Some(SCRATCHPAD)
        })
    }

    pub fn hide(&self) -> bool {
        let lua = format!(
            "hl.dsp.window.move({{ workspace = \"{SCRATCHPAD}\", follow = false, window = \"address:{}\" }})",
            self.address
        );
        hyprctl(&["dispatch", &lua]).is_some()
    }

    /// Brings the window back to the workspace you're on, and focuses it.
    pub fn show(&self) -> bool {
        let Some(active) = hyprctl(&["activeworkspace", "-j"]) else {
            return false;
        };
        let active: Value = serde_json::from_str(&active).unwrap_or_default();
        let Some(id) = active["id"].as_i64() else {
            return false;
        };
        let lua = format!(
            "hl.dispatch(hl.dsp.window.move({{ workspace = '{id}', follow = false, window = 'address:{a}' }}))\n\
             hl.dispatch(hl.dsp.focus({{ window = 'address:{a}' }}))",
            a = self.address
        );
        hyprctl(&["eval", &lua]).is_some()
    }

    /// Hides or shows; returns whether it's hidden now.
    pub fn toggle(&self) -> bool {
        if self.hidden() {
            self.show();
            false
        } else {
            self.hide();
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn walks_up_to_parent_processes() {
        let me = std::process::id();
        let parent = parent_of(me).expect("a test runner has a parent");
        assert!(parent > 0 && parent != me);
        assert_eq!(parent_of(u32::MAX), None);
    }

    #[test]
    fn song_details_for_the_desktop() {
        let track = crate::library::Track {
            title: "Tighten Up".into(),
            artist: "The Black Keys".into(),
            album: "Brothers".into(),
            duration_ms: 211_000,
            ..Default::default()
        };
        let m = metadata(&track, 7, Some(std::path::Path::new("/tmp/a b.jpg")));
        assert_eq!(m.title(), Some("Tighten Up"));
        assert_eq!(m.length(), Some(Time::from_millis(211_000)));
        assert_eq!(m.art_url().as_deref(), Some("file:///tmp/a%20b.jpg"));
    }
}
