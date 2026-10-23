//! Chip colours: parsing `#rrggbb`, WCAG contrast, and the automatic text
//! colour. Shared so the server, the board and clients agree exactly.

use crate::{board::ChipColor, validate::ValidationError};

/// Contrast below this is hard to read for normal-size text (WCAG AA).
pub const MIN_READABLE_CONTRAST: f64 = 4.5;

pub const BLACK: &str = "#000000";
pub const WHITE: &str = "#ffffff";

/// Normalizes `#rgb` or `#rrggbb` (any case, surrounding spaces allowed) to
/// lowercase `#rrggbb`.
pub fn normalize(input: &str) -> Result<String, ValidationError> {
    let hex = input
        .trim()
        .strip_prefix('#')
        .ok_or_else(|| ValidationError::Color(input.to_owned()))?;
    if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(ValidationError::Color(input.to_owned()));
    }
    let hex = hex.to_ascii_lowercase();
    match hex.len() {
        6 => Ok(format!("#{hex}")),
        3 => Ok(hex.chars().fold(String::from("#"), |mut s, c| {
            s.push(c);
            s.push(c);
            s
        })),
        _ => Err(ValidationError::Color(input.to_owned())),
    }
}

/// Validates and normalizes both colours of a chip.
pub fn chip(color: &ChipColor) -> Result<ChipColor, ValidationError> {
    Ok(ChipColor {
        background: normalize(&color.background)?,
        text: color.text.as_deref().map(normalize).transpose()?,
    })
}

fn rgb(color: &str) -> Option<[u8; 3]> {
    let hex = normalize(color).ok()?;
    let byte = |i: usize| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok();
    Some([byte(1)?, byte(3)?, byte(5)?])
}

/// WCAG relative luminance, 0 (black) to 1 (white).
fn luminance([r, g, b]: [u8; 3]) -> f64 {
    let channel = |c: u8| {
        let c = f64::from(c) / 255.0;
        if c <= 0.039_28 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b)
}

/// WCAG contrast ratio between two colours, from 1 to 21.
pub fn contrast(a: &str, b: &str) -> Option<f64> {
    let (la, lb) = (luminance(rgb(a)?), luminance(rgb(b)?));
    let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
    Some((hi + 0.05) / (lo + 0.05))
}

/// Black or white, whichever contrasts more with `background`.
pub fn auto_text(background: &str) -> &'static str {
    let black = contrast(background, BLACK).unwrap_or(21.0);
    let white = contrast(background, WHITE).unwrap_or(1.0);
    if black >= white { BLACK } else { WHITE }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_and_rejects() {
        assert_eq!(normalize(" #ABC ").unwrap(), "#aabbcc");
        assert_eq!(normalize("#1E3A8A").unwrap(), "#1e3a8a");
        for bad in [
            "", "1e3a8a", "#12345", "#12345g", "#1234567", "red", "#fff;}", "#ff\u{0}",
        ] {
            assert!(normalize(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn contrast_matches_wcag() {
        let c = contrast(BLACK, WHITE).unwrap();
        assert!((c - 21.0).abs() < 0.01);
        assert!((contrast("#777777", WHITE).unwrap() - 4.48).abs() < 0.01);
        assert_eq!(contrast("#abc", "nope"), None);
    }

    #[test]
    fn automatic_text() {
        assert_eq!(auto_text("#fef3c7"), BLACK);
        assert_eq!(auto_text("#1e3a8a"), WHITE);
        let chip = ChipColor {
            background: "#1e3a8a".into(),
            text: None,
        };
        assert_eq!(chip.text_or_auto(), WHITE);
        assert!(chip.contrast().unwrap() > MIN_READABLE_CONTRAST);
    }
}
