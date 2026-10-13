//! Album art for the Now Playing pane: 16 columns by 8 rows.
//!
//! Local songs use the cover inside the file, else a cover.jpg, folder.jpg… in the album's
//! folder. Spotify and YouTube Music songs use the service's cover image. Covers are read on
//! a background thread; the picture is then turned into terminal graphics once per song (sixel
//! in foot, the kitty protocol in Ghostty, coloured half-blocks anywhere else).

use crate::library::{Source, Track};
use image::{DynamicImage, imageops::FilterType};
use lofty::{config::ParseOptions, picture::PictureType, prelude::*, probe::Probe};
use ratatui::layout::Size;
use ratatui_image::{Resize, picker::Picker, protocol::Protocol};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const COLS: u16 = 16;
pub const ROWS: u16 = 8;

/// Which song a cover belongs to.
pub fn key(track: &Track) -> String {
    match track.source {
        Source::Local => track.album_dir.clone() + "|" + &track.path,
        _ => track.art_url.clone(),
    }
}

/// The cover picture inside a file: the front cover, else any picture.
fn embedded(path: &Path) -> Option<Vec<u8>> {
    let tagged = Probe::open(path)
        .ok()?
        .options(ParseOptions::new().read_properties(false))
        .read()
        .ok()?;
    // FLAC pictures live in its Vorbis comments; an MP3 can carry two kinds of tag.
    let pictures = || tagged.tags().iter().flat_map(|t| t.pictures());
    pictures()
        .find(|p| p.pic_type() == PictureType::CoverFront)
        .or_else(|| pictures().next())
        .map(|p| p.data().to_vec())
}

/// Image names looked for in an album folder, in order (counted in the WolfMusic library).
const STEMS: &[&str] = &["cover", "folder", "front", "album", "albumart", "thumb"];
const EXTENSIONS: &[&str] = &["jpg", "jpeg", "png"];

/// cover.jpg, Folder.JPG, front.png… next to the song; for songs in a CD1/Disc 2 folder,
/// in the album folder above it too.
fn folder_image(path: &Path) -> Option<PathBuf> {
    let dir = path.parent()?;
    find_in(dir).or_else(|| {
        let name = dir.file_name()?.to_string_lossy().to_lowercase();
        let disc = ["cd", "disc", "disk"].iter().any(|p| name.starts_with(p));
        if disc { find_in(dir.parent()?) } else { None }
    })
}

fn find_in(dir: &Path) -> Option<PathBuf> {
    let files: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_file())
        .collect();
    let lower = |s: Option<&std::ffi::OsStr>| s.map(|s| s.to_string_lossy().to_lowercase());
    STEMS
        .iter()
        .find_map(|stem| {
            files.iter().find(|p| {
                lower(p.file_stem()).as_deref() == Some(*stem)
                    && lower(p.extension()).is_some_and(|e| EXTENSIONS.contains(&e.as_str()))
            })
        })
        .cloned()
}

/// Finds and decodes a song's cover. Reads files or the network: call it off the screen's thread.
pub fn load(track: &Track) -> Option<DynamicImage> {
    match track.source {
        Source::Local => {
            let path = Path::new(&track.path);
            if let Some(bytes) = embedded(path)
                && let Ok(img) = image::load_from_memory(&bytes)
            {
                return Some(img);
            }
            image::ImageReader::open(folder_image(path)?)
                .ok()?
                .with_guessed_format()
                .ok()?
                .decode()
                .ok()
        }
        _ if track.art_url.starts_with("https://") => {
            let http: ureq::Agent = ureq::Agent::config_builder()
                .timeout_global(Some(Duration::from_secs(10)))
                .build()
                .into();
            let bytes = http
                .get(&track.art_url)
                .call()
                .ok()?
                .body_mut()
                .with_config()
                .limit(10 * 1024 * 1024)
                .read_to_vec()
                .ok()?;
            image::load_from_memory(&bytes).ok()
        }
        _ => None,
    }
}

/// How this terminal shows pictures. Terminals known to draw them (foot, Ghostty, kitty,
/// WezTerm) are asked which way and how big a character is; that has to happen before any key
/// is read. Others get coloured half-blocks without asking: a terminal that doesn't answer
/// would hold things up for two seconds and swallow the next key.
pub fn picker() -> Picker {
    let term = std::env::var("TERM").unwrap_or_default();
    let program = std::env::var("TERM_PROGRAM")
        .unwrap_or_default()
        .to_lowercase();
    let answers = term.starts_with("foot")
        || term.contains("ghostty")
        || term.contains("kitty")
        || program == "ghostty"
        || program == "wezterm";
    if answers && std::env::var_os("TMUX").is_none() {
        Picker::from_query_stdio().unwrap_or_else(|_| Picker::halfblocks())
    } else {
        Picker::halfblocks()
    }
}

/// Turns a cover into terminal graphics sized for the pane. Do it once per song: it's the
/// slow part. The picture is scaled and cropped to fill the box with a smooth filter first.
pub fn encode(picker: &Picker, img: &DynamicImage) -> Option<Protocol> {
    let font = picker.font_size();
    let (w, h) = (
        u32::from(COLS) * u32::from(font.width),
        u32::from(ROWS) * u32::from(font.height),
    );
    let fitted = DynamicImage::ImageRgb8(img.resize_to_fill(w, h, FilterType::Triangle).to_rgb8());
    picker
        .new_protocol(fitted, Size::new(COLS, ROWS), Resize::Fit(None))
        .ok()
}

/// Saves a cover where the desktop's media widget can show it (MPRIS wants a file).
pub fn save_for_desktop(img: &DynamicImage) -> Option<PathBuf> {
    let path = crate::config::cache_dir().join("now-playing.jpg");
    std::fs::create_dir_all(path.parent()?).ok()?;
    img.thumbnail(512, 512).to_rgb8().save(&path).ok()?;
    Some(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;
    use ratatui::{Terminal, backend::TestBackend, layout::Rect};
    use ratatui_image::Image;

    fn gradient() -> DynamicImage {
        DynamicImage::ImageRgb8(image::RgbImage::from_fn(300, 300, |x, y| {
            image::Rgb([(x * 255 / 300) as u8, (y * 255 / 300) as u8, 128])
        }))
    }

    #[test]
    fn finds_folder_images_and_draws_half_blocks() {
        let tmp = TempDir::new("art");
        let disc = tmp.0.join("Album").join("Disc 1");
        std::fs::create_dir_all(&disc).unwrap();
        gradient()
            .to_rgb8()
            .save(tmp.0.join("Album").join("Folder.JPG"))
            .unwrap();
        let track = Track {
            path: disc.join("01.flac").display().to_string(),
            ..Track::default()
        };
        let img = load(&track).expect("finds ../Folder.JPG from a disc folder");
        assert_eq!(img.width(), 300);

        let picker = Picker::halfblocks();
        let proto = encode(&picker, &img).unwrap();
        let mut term = Terminal::new(TestBackend::new(20, 10)).unwrap();
        term.draw(|f| f.render_widget(Image::new(&proto), Rect::new(2, 1, COLS, ROWS)))
            .unwrap();
        let cell = &term.backend().buffer()[(5u16, 3u16)];
        assert!(["▀", "▄", " "].contains(&cell.symbol()));
    }
}
