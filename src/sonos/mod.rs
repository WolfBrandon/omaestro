//! Sonos, over the speakers' own local control API (UPnP/SOAP on port 1400).
//!
//! SAFETY. Some speakers here are bonded into a home theater (Office: the Arc, two Era 300s,
//! two subs and two lamps), which a wrong command can break. So every command goes through
//! `permit`, which allows only:
//! - playback commands (play a link, pause, seek, volume…) sent to a group's coordinator,
//!   the speaker that runs the group;
//! - reading state from any visible speaker.
//!
//! It always refuses:
//! - grouping and bonding commands, and anything else not on the list;
//! - any command to a bonded (hidden) speaker;
//! - links that would join a group (`x-rincon:`).
//!
//! Every command that changes something is written to `~/.cache/omaestro/sonos.log`.

pub mod server;

use anyhow::{Context, Result, anyhow, bail};
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::net::{IpAddr, SocketAddr, TcpStream, UdpSocket};
use std::time::Duration;

const PORT: u16 = 1400;

/// The commands OMAESTRO may send, by service. Nothing else is ever sent.
const ALLOWED: &[(&str, &[&str])] = &[
    (
        "AVTransport",
        &[
            "SetAVTransportURI",
            "Play",
            "Pause",
            "Stop",
            "Seek",
            "GetPositionInfo",
            "GetMediaInfo",
            "GetTransportInfo",
        ],
    ),
    (
        "GroupRenderingControl",
        &["GetGroupVolume", "SetGroupVolume", "SnapshotGroupVolume"],
    ),
    ("ZoneGroupTopology", &["GetZoneGroupState"]),
    // Reading the speakers' queues, playlists and favourites (to find the Spotify account).
    ("ContentDirectory", &["Browse"]),
];

/// Links a speaker may be told to play.
const URI_SCHEMES: &[&str] = &["http://", "x-rincon-mp3radio:", "x-sonos-spotify:"];

/// What the safety check knows about where a command is going.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum To<'a> {
    /// A speaker from the household map.
    Speaker {
        uuid: &'a str,
        coordinator: bool,
        bonded: bool,
    },
    /// Only an address (while looking for speakers): reading is all that's allowed.
    Address,
}

/// Why a command was not sent.
#[derive(Debug, Clone, PartialEq)]
pub struct Refused(pub String);

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "OMAESTRO refused to send that to Sonos: {}", self.0)
    }
}

impl std::error::Error for Refused {}

fn is_read(action: &str) -> bool {
    action.starts_with("Get") || action == "Browse"
}

/// The safety rule. Pure, so it's tested on its own (see the tests at the bottom).
/// `never` is a list of speaker ids that must never receive anything.
pub fn permit(
    service: &str,
    action: &str,
    to: To,
    uri: Option<&str>,
    never: &[String],
) -> Result<(), Refused> {
    let listed = ALLOWED
        .iter()
        .any(|(s, actions)| *s == service && actions.contains(&action));
    if !listed {
        return Err(Refused(format!(
            "{service}.{action} isn't on OMAESTRO's list"
        )));
    }
    match to {
        To::Address => {
            if !is_read(action) {
                return Err(Refused(format!(
                    "{action} needs a known group coordinator, not just an address"
                )));
            }
        }
        To::Speaker {
            uuid,
            coordinator,
            bonded,
        } => {
            if bonded || never.iter().any(|n| n == uuid) {
                return Err(Refused(format!(
                    "{uuid} is a bonded speaker; OMAESTRO never addresses those"
                )));
            }
            if !is_read(action) && !coordinator {
                return Err(Refused(format!(
                    "{uuid} doesn't run its group; playback goes to the coordinator only"
                )));
            }
        }
    }
    if action == "SetAVTransportURI" {
        let uri = uri.unwrap_or_default();
        if uri.starts_with("x-rincon:") {
            return Err(Refused("x-rincon: links join groups".into()));
        }
        if !URI_SCHEMES.iter().any(|s| uri.starts_with(s)) {
            return Err(Refused(format!("won't play links like {uri:?}")));
        }
    }
    Ok(())
}

// ---- the household map -----------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct Speaker {
    pub uuid: String,
    pub room: String,
    pub ip: String,
}

/// A group of rooms playing together, as the Sonos app shows them.
#[derive(Debug, Clone, PartialEq)]
pub struct Group {
    pub id: String,
    /// The speaker that runs the group; all playback goes to it.
    pub coordinator: Speaker,
    /// Every room in the group, the coordinator's first.
    pub rooms: Vec<String>,
    /// A soundbar with surrounds: playing here switches the TV's sound over.
    pub home_theater: bool,
}

impl Group {
    /// "Dining Room + Kitchen Island + Portable".
    pub fn name(&self) -> String {
        self.rooms.join(" + ")
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Household {
    pub groups: Vec<Group>,
    /// Hidden speakers: the other halves of stereo pairs, subs, home-theater surrounds.
    pub bonded: HashSet<String>,
    /// Every visible speaker's address.
    pub ips: HashMap<String, String>,
}

impl Household {
    pub fn group(&self, coordinator: &str) -> Option<&Group> {
        self.groups
            .iter()
            .find(|g| g.coordinator.uuid == coordinator)
    }

    /// Where a command to this speaker is going, for the safety check.
    fn to<'a>(&self, uuid: &'a str) -> To<'a> {
        To::Speaker {
            uuid,
            coordinator: self.groups.iter().any(|g| g.coordinator.uuid == uuid),
            bonded: self.bonded.contains(uuid) || !self.ips.contains_key(uuid),
        }
    }
}

/// The value of `name="…"` in a tag's text.
fn attr(tag: &str, name: &str) -> Option<String> {
    let start = tag.find(&format!(" {name}=\""))? + name.len() + 3;
    let end = tag[start..].find('"')? + start;
    Some(unescape(&tag[start..end]))
}

/// Every start tag `<name …>` in `xml`, as its text.
fn tags<'a>(xml: &'a str, name: &str) -> Vec<&'a str> {
    let open = format!("<{name} ");
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some(i) = rest.find(&open) {
        let tag = &rest[i..];
        let end = tag.find('>').map_or(tag.len(), |e| e + 1);
        out.push(&tag[..end]);
        rest = &tag[end..];
    }
    out
}

/// "http://192.0.2.145:1400/xml/…" → "192.0.2.145".
fn ip_of(location: &str) -> Option<String> {
    let host = location.strip_prefix("http://")?.split('/').next()?;
    Some(host.split(':').next()?.to_string())
}

/// Reads the household map from a ZoneGroupState document.
pub fn parse_topology(xml: &str) -> Household {
    let mut h = Household::default();
    // Each group is the text between <ZoneGroup …> and </ZoneGroup>.
    for chunk in xml.split("</ZoneGroup>") {
        let Some(group_tag) = tags(chunk, "ZoneGroup").into_iter().next() else {
            continue;
        };
        let coordinator = attr(group_tag, "Coordinator").unwrap_or_default();
        let id = attr(group_tag, "ID").unwrap_or_default();
        let mut rooms = Vec::new();
        let mut leader = None;
        let mut home_theater = false;
        for tag in tags(chunk, "ZoneGroupMember") {
            let uuid = attr(tag, "UUID").unwrap_or_default();
            let hidden = attr(tag, "Invisible").as_deref() == Some("1")
                || attr(tag, "IsZoneBridge").as_deref() == Some("1");
            if hidden {
                h.bonded.insert(uuid);
                continue;
            }
            let room = attr(tag, "ZoneName").unwrap_or_default();
            let ip = attr(tag, "Location")
                .and_then(|l| ip_of(&l))
                .unwrap_or_default();
            h.ips.insert(uuid.clone(), ip.clone());
            let speaker = Speaker {
                uuid: uuid.clone(),
                room: room.clone(),
                ip,
            };
            if uuid == coordinator {
                home_theater = attr(tag, "HTSatChanMapSet").is_some_and(|m| m.contains(":CC"));
                rooms.insert(0, room);
                leader = Some(speaker);
            } else {
                rooms.push(room);
            }
        }
        for tag in tags(chunk, "Satellite") {
            if let Some(uuid) = attr(tag, "UUID") {
                h.bonded.insert(uuid);
            }
        }
        if let Some(coordinator) = leader {
            h.groups.push(Group {
                id,
                coordinator,
                rooms,
                home_theater,
            });
        }
    }
    h.groups.sort_by_key(Group::name);
    h
}

// ---- talking to speakers -----------------------------------------------------------------

fn control_path(service: &str) -> &'static str {
    match service {
        "AVTransport" => "/MediaRenderer/AVTransport/Control",
        "GroupRenderingControl" => "/MediaRenderer/GroupRenderingControl/Control",
        "RenderingControl" => "/MediaRenderer/RenderingControl/Control",
        "ZoneGroupTopology" => "/ZoneGroupTopology/Control",
        "ContentDirectory" => "/MediaServer/ContentDirectory/Control",
        _ => "/",
    }
}

/// `&`, `<`, `>`, `"` and `'` for use inside XML.
pub fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// The text inside `<name>…</name>` in a reply.
fn field(xml: &str, name: &str) -> Option<String> {
    let start = xml.find(&format!("<{name}>"))? + name.len() + 2;
    let end = xml[start..].find(&format!("</{name}>"))? + start;
    Some(unescape(&xml[start..end]))
}

fn http() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(6)))
        .http_status_as_error(false)
        .build()
        .into()
}

/// Sends one SOAP command and returns the reply. Only `Sonos::send` and the household
/// reading call this, after `permit` has said yes.
fn soap(ip: &str, service: &str, action: &str, args: &[(&str, String)]) -> Result<String> {
    let mut inner = String::new();
    if matches!(
        service,
        "AVTransport" | "GroupRenderingControl" | "RenderingControl"
    ) {
        inner.push_str("<InstanceID>0</InstanceID>");
    }
    for (k, v) in args {
        inner.push_str(&format!("<{k}>{}</{k}>", escape(v)));
    }
    let body = format!(
        "<?xml version=\"1.0\"?><s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\" \
         s:encodingStyle=\"http://schemas.xmlsoap.org/soap/encoding/\"><s:Body>\
         <u:{action} xmlns:u=\"urn:schemas-upnp-org:service:{service}:1\">{inner}</u:{action}>\
         </s:Body></s:Envelope>"
    );
    let mut res = http()
        .post(&format!("http://{ip}:{PORT}{}", control_path(service)))
        .header("Content-Type", "text/xml; charset=\"utf-8\"")
        .header(
            "SOAPACTION",
            &format!("\"urn:schemas-upnp-org:service:{service}:1#{action}\""),
        )
        .send(body.as_bytes())
        .with_context(|| format!("reaching the speaker at {ip}"))?;
    let status = res.status().as_u16();
    let text = res.body_mut().read_to_string().unwrap_or_default();
    match status {
        200 => Ok(text),
        403 => bail!(
            "the speaker refused (403): is \"UPnP\" switched off in the Sonos app under \
             Connection Security?"
        ),
        _ => {
            let code = field(&text, "errorCode").unwrap_or_else(|| status.to_string());
            bail!("{action} failed (Sonos error {code})")
        }
    }
}

/// Reads the household map from the speaker at `ip`.
pub fn read_topology(ip: &str) -> Result<Household> {
    permit(
        "ZoneGroupTopology",
        "GetZoneGroupState",
        To::Address,
        None,
        &[],
    )
    .map_err(|e| anyhow!("{e}"))?;
    let reply = soap(ip, "ZoneGroupTopology", "GetZoneGroupState", &[])?;
    let state = field(&reply, "ZoneGroupState").context("no household map in the reply")?;
    Ok(parse_topology(&state))
}

/// Finds the household: asks the known speakers first, then looks around the local network
/// (any speaker that answers knows all the others). Returns the map and the addresses of the
/// visible speakers, to remember for next time.
pub fn discover(known: &[String]) -> Result<Household> {
    for ip in known {
        if reachable(ip, Duration::from_millis(500))
            && let Ok(h) = read_topology(ip)
            && !h.groups.is_empty()
        {
            return Ok(h);
        }
    }
    for ip in scan_lan()? {
        if let Ok(h) = read_topology(&ip)
            && !h.groups.is_empty()
        {
            return Ok(h);
        }
    }
    bail!("No Sonos speakers answered on this network")
}

/// Whether a speaker answers on its port at all (a connection, no request).
pub fn reachable(ip: &str, timeout: Duration) -> bool {
    ip.parse::<IpAddr>()
        .ok()
        .is_some_and(|ip| TcpStream::connect_timeout(&SocketAddr::new(ip, PORT), timeout).is_ok())
}

/// This computer's address on the local network, as the speakers would reach it.
pub fn local_ip(towards: &str) -> Option<IpAddr> {
    // Connecting a UDP socket sends nothing; it just picks the route.
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect((towards, 9)).ok()?;
    Some(socket.local_addr().ok()?.ip())
}

/// Every address on this computer's /24 network with port 1400 open.
fn scan_lan() -> Result<Vec<String>> {
    let IpAddr::V4(me) = local_ip("192.168.1.1")
        .or_else(|| local_ip("10.0.0.1"))
        .context("no network")?
    else {
        bail!("no IPv4 network");
    };
    let [a, b, c, _] = me.octets();
    let found = std::sync::Mutex::new(Vec::new());
    let hosts: Vec<u8> = (1..=254).collect();
    std::thread::scope(|s| {
        for chunk in hosts.chunks(4) {
            let found = &found;
            s.spawn(move || {
                for d in chunk {
                    let ip = format!("{a}.{b}.{c}.{d}");
                    if reachable(&ip, Duration::from_millis(300))
                        && let Ok(mut list) = found.lock()
                    {
                        list.push(ip);
                    }
                }
            });
        }
    });
    Ok(found.into_inner().unwrap_or_default())
}

/// The household and the one way commands reach it.
#[derive(Debug, Clone, Default)]
pub struct Sonos {
    pub household: Household,
    /// Speakers that must never receive anything, from the config file.
    pub never: Vec<String>,
}

impl Sonos {
    /// Sends a command to a speaker, if the safety rule allows it. Changes are logged.
    pub fn send(
        &self,
        uuid: &str,
        service: &str,
        action: &str,
        args: &[(&str, String)],
    ) -> Result<String> {
        let uri = args
            .iter()
            .find(|(k, _)| *k == "CurrentURI")
            .map(|(_, v)| v.as_str());
        permit(service, action, self.household.to(uuid), uri, &self.never)?;
        let ip = self
            .household
            .ips
            .get(uuid)
            .ok_or_else(|| anyhow!("no address for {uuid}"))?;
        if !is_read(action) {
            log(&format!("{uuid} {service}.{action} {}", uri.unwrap_or("")));
        }
        soap(ip, service, action, args)
    }

    pub fn play_uri(&self, group: &Group, uri: &str, metadata: &str) -> Result<()> {
        let uuid = &group.coordinator.uuid;
        self.send(
            uuid,
            "AVTransport",
            "SetAVTransportURI",
            &[
                ("CurrentURI", uri.to_string()),
                ("CurrentURIMetaData", metadata.to_string()),
            ],
        )?;
        self.send(uuid, "AVTransport", "Play", &[("Speed", "1".into())])?;
        Ok(())
    }

    pub fn pause(&self, group: &Group) -> Result<()> {
        self.send(&group.coordinator.uuid, "AVTransport", "Pause", &[])
            .map(|_| ())
    }

    pub fn resume(&self, group: &Group) -> Result<()> {
        self.send(
            &group.coordinator.uuid,
            "AVTransport",
            "Play",
            &[("Speed", "1".into())],
        )
        .map(|_| ())
    }

    pub fn stop(&self, group: &Group) -> Result<()> {
        self.send(&group.coordinator.uuid, "AVTransport", "Stop", &[])
            .map(|_| ())
    }

    pub fn seek(&self, group: &Group, seconds: f64) -> Result<()> {
        let s = seconds.max(0.0) as u64;
        let target = format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60);
        self.send(
            &group.coordinator.uuid,
            "AVTransport",
            "Seek",
            &[("Unit", "REL_TIME".into()), ("Target", target)],
        )
        .map(|_| ())
    }

    pub fn volume(&self, group: &Group) -> Result<u8> {
        let reply = self.send(
            &group.coordinator.uuid,
            "GroupRenderingControl",
            "GetGroupVolume",
            &[],
        )?;
        Ok(field(&reply, "CurrentVolume")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0))
    }

    pub fn set_volume(&self, group: &Group, volume: u8) -> Result<()> {
        let uuid = &group.coordinator.uuid;
        self.send(uuid, "GroupRenderingControl", "SnapshotGroupVolume", &[])?;
        self.send(
            uuid,
            "GroupRenderingControl",
            "SetGroupVolume",
            &[("DesiredVolume", volume.min(100).to_string())],
        )
        .map(|_| ())
    }

    /// What the group is doing: its state, where it is in the song and how long the song is
    /// (seconds), and the link it's playing.
    pub fn position(&self, group: &Group) -> Result<Position> {
        let uuid = &group.coordinator.uuid;
        let transport = self.send(uuid, "AVTransport", "GetTransportInfo", &[])?;
        let info = self.send(uuid, "AVTransport", "GetPositionInfo", &[])?;
        Ok(Position {
            state: field(&transport, "CurrentTransportState").unwrap_or_default(),
            position: field(&info, "RelTime")
                .map(|t| clock_secs(&t))
                .unwrap_or(0.0),
            duration: field(&info, "TrackDuration")
                .map(|t| clock_secs(&t))
                .unwrap_or(0.0),
            uri: field(&info, "TrackURI").unwrap_or_default(),
        })
    }

    /// The Spotify account linked in the Sonos app, which a Spotify link for Sonos has to name.
    /// Older Sonos software lists linked accounts (`/status/accounts`); newer software (18.x)
    /// leaves that list empty, so OMAESTRO reads the account from Spotify songs the Sonos app
    /// already put on the speakers: each group's queue, then Spotify albums or playlists in
    /// Sonos favourites.
    /// What it finds is remembered, so a cleared queue later doesn't matter.
    pub fn spotify_account(&self) -> Option<SpotifyAccount> {
        let saved = std::fs::read_to_string(account_file()).ok();
        if let Some(account) = saved.as_deref().and_then(SpotifyAccount::from_line) {
            return Some(account);
        }
        let found = self.find_spotify_account()?;
        let _ = std::fs::write(account_file(), found.to_line());
        Some(found)
    }

    fn find_spotify_account(&self) -> Option<SpotifyAccount> {
        let coordinators: Vec<&Speaker> = self
            .household
            .groups
            .iter()
            .map(|g| &g.coordinator)
            .collect();
        let first = coordinators.first()?;
        let listed = http()
            .get(&format!("http://{}:{PORT}/status/accounts", first.ip))
            .call()
            .ok()
            .and_then(|mut res| res.body_mut().read_to_string().ok());
        if let Some(account) = listed.as_deref().and_then(spotify_account_in) {
            return Some(account);
        }
        let browse = |uuid: &str, id: &str| {
            self.send(
                uuid,
                "ContentDirectory",
                "Browse",
                &[
                    ("ObjectID", id.into()),
                    ("BrowseFlag", "BrowseDirectChildren".into()),
                    ("Filter", "*".into()),
                    ("StartingIndex", "0".into()),
                    ("RequestedCount", "200".into()),
                    ("SortCriteria", String::new()),
                ],
            )
            .ok()
        };
        coordinators
            .iter()
            .find_map(|c| browse(&c.uuid, "Q:0").and_then(|xml| spotify_account_in_links(&xml)))
            .or_else(|| browse(&first.uuid, "FV:2").and_then(|x| spotify_account_in_links(&x)))
    }
}

/// Where the Spotify account Sonos uses is remembered.
fn account_file() -> std::path::PathBuf {
    crate::config::cache_dir().join("sonos-spotify-account")
}

/// Forgets the remembered Spotify account, after Sonos turned a Spotify link down, so the
/// next try looks again (the account may have been linked again in the Sonos app).
pub fn forget_spotify_account() {
    let _ = std::fs::remove_file(account_file());
}

/// Sonos's numbers for the Spotify account linked in the Sonos app, as they appear in its
/// Spotify links: "x-sonos-spotify:spotify%3atrack%3a…?sid=12&flags=8232&sn=2".
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpotifyAccount {
    /// The music service's number: 12 for Spotify (9 in some countries).
    pub sid: u32,
    /// Which linked account.
    pub sn: u32,
    pub flags: u32,
}

/// The flags Sonos's own app puts on a Spotify song (Sonos 18.8).
const SPOTIFY_TRACK_FLAGS: u32 = 8232;

impl SpotifyAccount {
    /// The service type in Sonos's account tokens: sid 12 → 3079.
    fn kind(self) -> u32 {
        self.sid * 256 + 7
    }

    /// "12 2 8232", as remembered.
    fn to_line(self) -> String {
        format!("{} {} {}", self.sid, self.sn, self.flags)
    }

    fn from_line(line: &str) -> Option<SpotifyAccount> {
        let mut n = line.split_whitespace().map(|w| w.parse::<u32>().ok());
        Some(SpotifyAccount {
            sid: n.next()??,
            sn: n.next()??,
            flags: n.next()??,
        })
    }
}

/// Finds the Spotify account in a `/status/accounts` page (older Sonos software): Spotify's
/// service type is 2311 or 3079.
fn spotify_account_in(text: &str) -> Option<SpotifyAccount> {
    tags(text, "Account").into_iter().find_map(|tag| {
        let kind: u32 = attr(tag, "Type")?.parse().ok()?;
        if kind != 2311 && kind != 3079 {
            return None;
        }
        let serial = attr(tag, "SerialNum")
            .and_then(|s| s.parse().ok())
            .unwrap_or(1);
        Some(SpotifyAccount {
            sid: (kind - 7) / 256,
            sn: serial,
            flags: 8224,
        })
    })
}

/// Finds the Spotify account in the links of a queue, playlist or favourites list: a Spotify
/// song ("x-sonos-spotify:…") or a Spotify album or playlist ("x-rincon-cpcontainer:…spotify…").
fn spotify_account_in_links(xml: &str) -> Option<SpotifyAccount> {
    // The list is XML inside XML (and favourites go one level deeper).
    let text = unescape(&unescape(&unescape(xml)));
    text.split(|c: char| c == '<' || c == '>' || c == '"' || c.is_whitespace())
        .find_map(|link| {
            let song = link.starts_with("x-sonos-spotify:");
            let list = link.starts_with("x-rincon-cpcontainer:") && link.contains("spotify");
            if !song && !list {
                return None;
            }
            let (_, query) = link.split_once('?')?;
            let number = |key: &str| {
                query.split('&').find_map(|pair| {
                    let (k, v) = pair.split_once('=')?;
                    (k == key).then(|| v.parse::<u32>().ok())?
                })
            };
            Some(SpotifyAccount {
                sid: number("sid")?,
                sn: number("sn")?,
                // A list's flags aren't a song's.
                flags: if song {
                    number("flags").unwrap_or(SPOTIFY_TRACK_FLAGS)
                } else {
                    SPOTIFY_TRACK_FLAGS
                },
            })
        })
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Position {
    pub state: String,
    pub position: f64,
    pub duration: f64,
    pub uri: String,
}

/// "0:04:03" → 243.
fn clock_secs(t: &str) -> f64 {
    t.split(':')
        .filter_map(|p| p.parse::<f64>().ok())
        .fold(0.0, |acc, p| acc * 60.0 + p)
}

fn log(line: &str) {
    let path = crate::config::cache_dir().join("sonos.log");
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(f, "{} {line}", crate::secrets::now_secs());
    }
}

// ---- what Sonos shows about a song ----------------------------------------------------

const DIDL_OPEN: &str = "<DIDL-Lite xmlns:dc=\"http://purl.org/dc/elements/1.1/\" \
    xmlns:upnp=\"urn:schemas-upnp-org:metadata-1-0/upnp/\" \
    xmlns:r=\"urn:schemas-rinconnetworks-com:metadata-1-0/\" \
    xmlns=\"urn:schemas-upnp-org:metadata-1-0/DIDL-Lite/\">";

/// Track details for a file OMAESTRO serves over HTTP.
pub fn didl_http(
    title: &str,
    artist: &str,
    album: &str,
    art: &str,
    url: &str,
    mime: &str,
    seconds: u64,
) -> String {
    let art = if art.is_empty() {
        String::new()
    } else {
        format!("<upnp:albumArtURI>{}</upnp:albumArtURI>", escape(art))
    };
    format!(
        "{DIDL_OPEN}<item id=\"-1\" parentID=\"-1\" restricted=\"true\">\
         <res protocolInfo=\"http-get:*:{mime}:*\" duration=\"{}:{:02}:{:02}\">{}</res>{art}\
         <dc:title>{}</dc:title><dc:creator>{}</dc:creator><upnp:album>{}</upnp:album>\
         <upnp:class>object.item.audioItem.musicTrack</upnp:class>\
         <desc id=\"cdudn\" nameSpace=\"urn:schemas-rinconnetworks-com:metadata-1-0/\">RINCON_AssociatedZPUDN</desc>\
         </item></DIDL-Lite>",
        seconds / 3600,
        seconds / 60 % 60,
        seconds % 60,
        escape(url),
        escape(title),
        escape(artist),
        escape(album)
    )
}

/// A Spotify track's link and details for Sonos, which streams it from Spotify itself with
/// the account linked in the Sonos app. Made the way the Sonos app makes them.
pub fn spotify_link(
    uri: &str,
    title: &str,
    artist: &str,
    album: &str,
    account: SpotifyAccount,
) -> (String, String) {
    let SpotifyAccount { sid, sn, flags } = account;
    let kind = account.kind();
    let encoded = uri.replace(':', "%3a");
    let link = format!("x-sonos-spotify:{encoded}?sid={sid}&flags={flags}&sn={sn}");
    let meta = format!(
        "{DIDL_OPEN}<item id=\"1003{flags:04x}{encoded}\" parentID=\"-1\" restricted=\"true\">\
         <dc:title>{}</dc:title><dc:creator>{}</dc:creator><upnp:album>{}</upnp:album>\
         <upnp:class>object.item.audioItem.musicTrack</upnp:class>\
         <desc id=\"cdudn\" nameSpace=\"urn:schemas-rinconnetworks-com:metadata-1-0/\">\
         SA_RINCON{kind}_X_#Svc{kind}-0-Token</desc></item></DIDL-Lite>",
        escape(title),
        escape(artist),
        escape(album)
    );
    (link, meta)
}

#[cfg(test)]
mod tests {
    use super::*;

    // A real household's layout (ids and addresses replaced): Office is an Arc with bonded
    // surrounds, the kind of setup the safety rule is about.
    const ARC: &str = "RINCON_000E58A0000101400";
    const OFFICE_BONDED: [&str; 6] = [
        "RINCON_000E58A0000201400",
        "RINCON_000E58A0000301400",
        "RINCON_000E58A0000401400",
        "RINCON_000E58A0000501400",
        "RINCON_000E58A0000601400",
        "RINCON_000E58A0000701400",
    ];
    const DINING: &str = "RINCON_000E58A0000801400";
    const KITCHEN: &str = "RINCON_000E58A0000901400";

    fn household() -> Household {
        parse_topology(include_str!("test-topology.xml"))
    }

    #[test]
    fn reads_the_household() {
        let h = household();
        let names: Vec<String> = h.groups.iter().map(Group::name).collect();
        assert_eq!(
            names,
            [
                "Dining Room + Kitchen Island + Portable",
                "Living Room",
                "Office"
            ]
        );
        let office = h.group(ARC).unwrap();
        assert!(office.home_theater);
        assert_eq!(office.coordinator.ip, "192.0.2.145");
        for uuid in OFFICE_BONDED {
            assert!(h.bonded.contains(uuid), "{uuid} should be known as bonded");
            assert!(!h.ips.contains_key(uuid));
        }
        assert!(!h.group(DINING).unwrap().home_theater);
    }

    #[test]
    fn office_gets_playback_only_and_its_bonded_speakers_get_nothing() {
        let h = household();
        let ok = |service, action, uuid, uri| permit(service, action, h.to(uuid), uri, &[]);
        // Playback to the Arc, which runs the Office group: allowed.
        assert!(ok("AVTransport", "Play", ARC, None).is_ok());
        assert!(
            ok(
                "AVTransport",
                "SetAVTransportURI",
                ARC,
                Some("http://192.0.2.60:47100/t/1")
            )
            .is_ok()
        );
        assert!(ok("GroupRenderingControl", "SetGroupVolume", ARC, None).is_ok());
        // Grouping or bonding, to Office or anywhere: refused.
        for action in [
            "BecomeCoordinatorOfStandaloneGroup",
            "BecomeGroupCoordinator",
            "BecomeGroupCoordinatorAndSource",
            "ChangeCoordinator",
            "DelegateGroupCoordinationTo",
            "AddURIToQueue",
            "RemoveAllTracksFromQueue",
        ] {
            assert!(
                ok("AVTransport", action, ARC, None).is_err(),
                "{action} must be refused"
            );
            assert!(
                ok("AVTransport", action, DINING, None).is_err(),
                "{action} must be refused"
            );
        }
        for action in [
            "AddBondedZones",
            "RemoveBondedZones",
            "CreateStereoPair",
            "SetZoneAttributes",
            "AddHTSatellite",
        ] {
            assert!(ok("DeviceProperties", action, ARC, None).is_err());
        }
        // Reading lists (Browse) is fine anywhere but a bonded speaker.
        assert!(permit("ContentDirectory", "Browse", To::Address, None, &[]).is_ok());
        assert!(ok("ContentDirectory", "Browse", DINING, None).is_ok());
        assert!(ok("ContentDirectory", "Browse", OFFICE_BONDED[0], None).is_err());
        assert!(ok("ContentDirectory", "DestroyObject", DINING, None).is_err());
        // Joining a group by link: refused.
        let join = format!("x-rincon:{DINING}");
        assert!(ok("AVTransport", "SetAVTransportURI", ARC, Some(&join)).is_err());
        assert!(
            ok(
                "AVTransport",
                "SetAVTransportURI",
                DINING,
                Some("x-rincon:RINCON_X")
            )
            .is_err()
        );
        assert!(
            ok(
                "AVTransport",
                "SetAVTransportURI",
                DINING,
                Some("file:///etc/passwd")
            )
            .is_err()
        );
        // The bonded Office speakers receive nothing at all, not even reads.
        for uuid in OFFICE_BONDED {
            assert!(ok("AVTransport", "Play", uuid, None).is_err());
            assert!(ok("AVTransport", "GetTransportInfo", uuid, None).is_err());
            assert!(ok("GroupRenderingControl", "GetGroupVolume", uuid, None).is_err());
        }
        // A room that isn't running its group gets reads but no playback.
        assert!(ok("AVTransport", "GetPositionInfo", KITCHEN, None).is_ok());
        assert!(ok("AVTransport", "Play", KITCHEN, None).is_err());
        // An unknown speaker counts as bonded.
        assert!(ok("AVTransport", "Play", "RINCON_UNKNOWN", None).is_err());
        // Only reads go to a bare address.
        assert!(
            permit(
                "ZoneGroupTopology",
                "GetZoneGroupState",
                To::Address,
                None,
                &[]
            )
            .is_ok()
        );
        assert!(permit("AVTransport", "Play", To::Address, None, &[]).is_err());
        // The config's "never" list wins over everything.
        assert!(permit("AVTransport", "Play", h.to(ARC), None, &[ARC.to_string()]).is_err());
    }

    /// Reads a real household (read-only requests only):
    /// `OMAESTRO_SONOS_IP=… cargo test live_sonos -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn live_sonos_household() {
        // A speaker's address or two: OMAESTRO_SONOS_IP=192.168.1.20,192.168.1.21
        let Ok(ips) = std::env::var("OMAESTRO_SONOS_IP") else {
            return;
        };
        let known: Vec<String> = ips.split(',').map(|ip| ip.trim().to_string()).collect();
        let h = discover(&known).unwrap();
        for g in &h.groups {
            println!(
                "{} · {} at {}{}",
                g.name(),
                g.coordinator.uuid,
                g.coordinator.ip,
                if g.home_theater {
                    " · home theater"
                } else {
                    ""
                }
            );
        }
        println!("{} bonded speakers", h.bonded.len());
        assert!(!h.groups.is_empty());
        // Reads only: the accounts page and the lists (Browse), never anything that plays.
        let sonos = Sonos {
            household: h,
            never: Vec::new(),
        };
        println!("Spotify on Sonos: {:?}", sonos.find_spotify_account());
    }

    #[test]
    fn spotify_links_and_track_details() {
        // Older Sonos software lists the accounts.
        let accounts = r#"<ZPSupportInfo><Accounts><Account Type="519" SerialNum="3"/><Account Type="2311" SerialNum="1" Deleted="0"/></Accounts></ZPSupportInfo>"#;
        let old = spotify_account_in(accounts).unwrap();
        assert_eq!((old.sid, old.sn, old.flags), (9, 1, 8224));
        let (link, meta) = spotify_link(
            "spotify:track:4uLU6hMCjMI75M1A2tKUQC",
            "Rick & Roll",
            "Rick",
            "Hits",
            old,
        );
        assert_eq!(
            link,
            "x-sonos-spotify:spotify%3atrack%3a4uLU6hMCjMI75M1A2tKUQC?sid=9&flags=8224&sn=1"
        );
        assert!(meta.contains("SA_RINCON2311_X_#Svc2311-0-Token"));
        assert!(meta.contains("Rick &amp; Roll"));

        // Sonos 18.8 lists nothing (read from the speakers on 2026-10-13)...
        let empty = r#"<?xml version="1.0" ?><ZPSupportInfo></ZPSupportInfo>"#;
        assert_eq!(spotify_account_in(empty), None);
        // ...but the Dining Room's queue holds songs the Sonos app added: a Browse reply, with
        // the song's link in it and again, encoded, in its picture's address.
        let queue = "<s:Envelope><s:Body><u:BrowseResponse><Result>&lt;DIDL-Lite&gt;&lt;item id=&quot;Q:0/1&quot; parentID=&quot;Q:0&quot; restricted=&quot;true&quot;&gt;&lt;res protocolInfo=&quot;sonos.com-spotify:*:audio/x-spotify:*&quot; duration=&quot;0:04:03&quot;&gt;x-sonos-spotify:spotify%3atrack%3a0eX5QY6o5cb9W72zVmWWjV?sid=12&amp;amp;flags=8232&amp;amp;sn=2&lt;/res&gt;&lt;upnp:albumArtURI&gt;/getaa?s=1&amp;amp;u=x-sonos-spotify%3aspotify%253atrack%253a0eX5QY6o5cb9W72zVmWWjV%3fsid%3d12%26flags%3d8232%26sn%3d2&lt;/upnp:albumArtURI&gt;&lt;dc:title&gt;The Power of Equality&lt;/dc:title&gt;&lt;/item&gt;&lt;/DIDL-Lite&gt;</Result><NumberReturned>1</NumberReturned></u:BrowseResponse></s:Body></s:Envelope>";
        let found = spotify_account_in_links(queue).unwrap();
        assert_eq!(
            found,
            SpotifyAccount {
                sid: 12,
                sn: 2,
                flags: 8232
            }
        );
        assert_eq!(SpotifyAccount::from_line(&found.to_line()), Some(found));
        let (link, meta) = spotify_link(
            "spotify:track:26q6YTrXt9l8qshIveiTX9",
            "Rebirth",
            "",
            "",
            found,
        );
        assert_eq!(
            link,
            "x-sonos-spotify:spotify%3atrack%3a26q6YTrXt9l8qshIveiTX9?sid=12&flags=8232&sn=2"
        );
        // As in the Sonos app's own favourite of the same song.
        assert!(meta.contains("id=\"10032028spotify%3atrack%3a26q6YTrXt9l8qshIveiTX9\""));
        assert!(meta.contains("SA_RINCON3079_X_#Svc3079-0-Token"));
        // Other services' lists, and queues without Spotify, don't count.
        let other =
            "x-rincon-cpcontainer:10062e6cplaylist%3a17151827?sid=31&amp;flags=11884&amp;sn=9";
        assert_eq!(spotify_account_in_links(other), None);
        let playlist = "&lt;res&gt;x-rincon-cpcontainer:1006206cspotify%3aplaylist%3aABC?sid=12&amp;amp;flags=8300&amp;amp;sn=2&lt;/res&gt;";
        assert_eq!(
            spotify_account_in_links(playlist),
            Some(SpotifyAccount {
                sid: 12,
                sn: 2,
                flags: 8232
            })
        );
        let didl = didl_http(
            "A < B",
            "Artist",
            "Album",
            "",
            "http://x/t/1",
            "audio/flac",
            243,
        );
        assert!(didl.contains("duration=\"0:04:03\"") && didl.contains("A &lt; B"));
        assert_eq!(clock_secs("0:04:03"), 243.0);
    }
}
