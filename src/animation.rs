//! Something to watch in the visualizer when there's no sound to read: Spotify playing on Sonos
//! streams straight to the speaker, so there's nothing here to draw bars from.
//!
//! Three classic demo effects, written for OMAESTRO: a starfield, a plasma and a kaleidoscope.
//! Each frame is worked out from the time alone, so nothing needs remembering between frames.
//! A frame is a grid of characters, each with a brightness from 0 to 1 that the screen turns
//! into the theme's colours.

use serde::{Deserialize, Serialize};
use std::f64::consts::TAU;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    #[default]
    Starfield,
    Plasma,
    Kaleidoscope,
    Off,
}

impl Kind {
    pub const ALL: [Kind; 4] = [Kind::Starfield, Kind::Plasma, Kind::Kaleidoscope, Kind::Off];

    pub fn name(self) -> &'static str {
        match self {
            Kind::Starfield => "Starfield",
            Kind::Plasma => "Plasma",
            Kind::Kaleidoscope => "Kaleidoscope",
            Kind::Off => "Off",
        }
    }
}

/// One character of a frame and how bright it is (0 to 1).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Cell {
    pub ch: char,
    pub level: f32,
}

const EMPTY: Cell = Cell {
    ch: ' ',
    level: 0.0,
};

/// A frame `w` × `h` characters, `t` seconds in. Rows top to bottom.
pub fn frame(kind: Kind, w: usize, h: usize, t: f64) -> Vec<Vec<Cell>> {
    match kind {
        Kind::Starfield => starfield(w, h, t),
        Kind::Plasma => shaded(w, h, |x, y| plasma(x, y, w, h, t), " .:-=+*#%@"),
        Kind::Kaleidoscope => shaded(w, h, |x, y| kaleidoscope(x, y, w, h, t), " .:+*oO#@"),
        Kind::Off => vec![vec![EMPTY; w]; h],
    }
}

/// A frame from a function giving each character's brightness, drawn with `ramp`'s characters
/// from faint to dense.
fn shaded(w: usize, h: usize, value: impl Fn(f64, f64) -> f64, ramp: &str) -> Vec<Vec<Cell>> {
    let ramp: Vec<char> = ramp.chars().collect();
    (0..h)
        .map(|y| {
            (0..w)
                .map(|x| {
                    let v = value(x as f64, y as f64).clamp(0.0, 1.0);
                    let i = ((v * (ramp.len() - 1) as f64).round() as usize).min(ramp.len() - 1);
                    Cell {
                        ch: ramp[i],
                        level: v as f32,
                    }
                })
                .collect()
        })
        .collect()
}

/// Waves crossing each other and rings from the middle, drifting at different speeds.
/// Characters are about twice as tall as they're wide, so x counts half.
fn plasma(x: f64, y: f64, w: usize, h: usize, t: f64) -> f64 {
    let (u, v) = (x * 0.5, y);
    let (cu, cv) = (w as f64 * 0.25, h as f64 * 0.5);
    let rings = ((u - cu).hypot(v - cv) * 0.55 - t * 1.4).sin();
    let sum = (u * 0.35 + t * 1.1).sin()
        + (v * 0.45 - t * 0.8).sin()
        + ((u * 0.2 + v * 0.3) + t * 0.6).sin()
        + rings;
    (sum / 4.0 + 1.0) / 2.0
}

/// A pattern of rings and petals, folded into six mirrored slices that slowly turn.
fn kaleidoscope(x: f64, y: f64, w: usize, h: usize, t: f64) -> f64 {
    let dx = (x - w as f64 / 2.0) * 0.5;
    let dy = y - h as f64 / 2.0;
    let r = dx.hypot(dy) / (h as f64 / 2.0).max(1.0);
    let wedge = TAU / 6.0;
    let mut a = (dy.atan2(dx) + t * 0.25).rem_euclid(wedge);
    if a > wedge / 2.0 {
        a = wedge - a;
    }
    let petals = (r * 2.2 - t * 1.2 + 1.5 * (a * 3.0 + t * 0.7).sin()).sin();
    let spokes = (a * 4.0 - r * 1.1 + t * 0.4).cos();
    0.5 + 0.5 * petals * spokes
}

/// Stars flying towards you, in braille dots (2 × 4 to a character). Each star's path is
/// worked out from its number and the time: it starts far away somewhere random, comes
/// closer and brighter, and starts again when it passes.
fn starfield(w: usize, h: usize, t: f64) -> Vec<Vec<Cell>> {
    const DOTS: [[u32; 4]; 2] = [[0x01, 0x02, 0x04, 0x40], [0x08, 0x10, 0x20, 0x80]];
    let mut bits = vec![vec![0u32; w]; h];
    let mut level = vec![vec![0f32; w]; h];
    let (dw, dh) = (w as f64 * 2.0, h as f64 * 4.0);
    let stars = (w * h / 5).max(12);
    for i in 0..stars {
        let speed = 0.12 + 0.18 * unit(hash(i as u64, 0));
        let travelled = t * speed + unit(hash(i as u64, 1));
        let lap = travelled.floor() as u64;
        let z = 1.0 - travelled.fract() * 0.95; // 1 (far) to 0.05 (passing)
        let sx = unit(hash(i as u64, lap * 2 + 2)) * 2.0 - 1.0;
        let sy = unit(hash(i as u64, lap * 2 + 3)) * 2.0 - 1.0;
        let x = dw / 2.0 + sx / z * dw * 0.25;
        let y = dh / 2.0 + sy / z * dh * 0.25;
        let bright = (1.0 - z) as f32;
        // Close stars are two dots wide.
        let width = if bright > 0.7 { 2 } else { 1 };
        for d in 0..width {
            let (px, py) = (x + d as f64, y);
            if px < 0.0 || py < 0.0 || px >= dw || py >= dh {
                continue;
            }
            let (px, py) = (px as usize, py as usize);
            let (cx, cy) = (px / 2, py / 4);
            bits[cy][cx] |= DOTS[px % 2][py % 4];
            level[cy][cx] = level[cy][cx].max(bright);
        }
    }
    (0..h)
        .map(|y| {
            (0..w)
                .map(|x| match char::from_u32(0x2800 + bits[y][x]) {
                    Some(ch) if bits[y][x] != 0 => Cell {
                        ch,
                        level: 0.25 + 0.75 * level[y][x],
                    },
                    _ => EMPTY,
                })
                .collect()
        })
        .collect()
}

/// A well-mixed number from two (SplitMix64), so each star gets its own random-looking path.
fn hash(a: u64, b: u64) -> u64 {
    let mut z = a
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add(b.wrapping_mul(0xBF58_476D_1CE4_E5B9))
        .wrapping_add(0x94D0_49BB_1331_11EB);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// 0 to 1 from a hash.
fn unit(h: u64) -> f64 {
    (h >> 11) as f64 / (1u64 << 53) as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_fill_the_area_move_and_stay_in_range() {
        for kind in [Kind::Starfield, Kind::Plasma, Kind::Kaleidoscope] {
            let a = frame(kind, 48, 9, 1.0);
            let b = frame(kind, 48, 9, 1.5);
            assert_eq!((a.len(), a[0].len()), (9, 48), "{kind:?}");
            assert_ne!(a, b, "{kind:?} moves");
            assert_eq!(
                a,
                frame(kind, 48, 9, 1.0),
                "{kind:?} is the same at the same time"
            );
            let cells = a.iter().flatten();
            assert!(
                cells.clone().all(|c| (0.0..=1.0).contains(&c.level)),
                "{kind:?}"
            );
            assert!(
                cells.filter(|c| c.ch != ' ').count() > 20,
                "{kind:?} draws something"
            );
        }
        assert!(
            frame(Kind::Off, 10, 3, 0.0)
                .iter()
                .flatten()
                .all(|c| c.ch == ' ')
        );
        // Tiny or empty areas are fine.
        for kind in Kind::ALL {
            assert!(frame(kind, 0, 0, 2.0).is_empty());
            frame(kind, 1, 1, 2.0);
        }
    }

    #[test]
    fn the_kaleidoscope_is_mirrored() {
        // Six mirrored slices: the same at a point and its reflection across the middle row
        // (a mirror line of the fold) when the pattern isn't turned.
        let (w, h) = (40, 20);
        for (x, y) in [(5.0, 3.0), (30.0, 6.0), (12.0, 9.0)] {
            let here = kaleidoscope(x, y, w, h, 0.0);
            let mirrored = kaleidoscope(x, h as f64 - y, w, h, 0.0);
            assert!((here - mirrored).abs() < 1e-9, "({x}, {y})");
        }
    }

    #[test]
    #[ignore = "prints frames to look at: cargo test animation -- --ignored --nocapture"]
    fn show_frames() {
        for kind in [Kind::Starfield, Kind::Plasma, Kind::Kaleidoscope] {
            println!("{kind:?}");
            for row in frame(kind, 48, 9, 3.7) {
                println!("│{}│", row.iter().map(|c| c.ch).collect::<String>());
            }
        }
    }
}
