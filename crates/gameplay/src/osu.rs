//! The osu!mania 4K `.osu` loader of the macOS app (`OsuMania4KParser`), as a function from text to chart.

use crate::{Chart, ChartError, ChartObject};
use std::collections::HashMap;
use std::fmt;

/// A chart read from a `.osu` file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OsuBeatmap {
    /// `duration_ms` is the latest note start or hold end.
    pub chart: Chart,
    /// `TitleUnicode`, else `Title`.
    pub title: Option<String>,
    /// `ArtistUnicode`, else `Artist`.
    pub artist: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OsuError {
    /// `[General]` `Mode` is not 3 (osu!mania).
    UnsupportedMode,
    /// `[Difficulty]` `CircleSize`, the key count, is not 4; `keys` is it rounded, 0 when missing.
    UnsupportedKeyCount { keys: i64 },
    /// A hit object that is not a note or a hold, or that cannot be read.
    UnsupportedHitObject { line: String },
    /// The notes do not form a valid chart, for example two in one lane at the same time.
    InvalidChart { error: ChartError },
}

impl fmt::Display for OsuError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedMode => write!(f, "Only osu!mania mode is supported."),
            Self::UnsupportedKeyCount { keys } => write!(
                f,
                "Only 4-key osu!mania charts are supported; this chart has {keys} keys."
            ),
            Self::UnsupportedHitObject { line } => write!(f, "Unsupported hit object: {line}."),
            Self::InvalidChart { error } => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for OsuError {}

/// Reads an osu!mania 4K beatmap. Lines end at any line break; sections other than `General`,
/// `Metadata`, `Difficulty` and `HitObjects` are skipped. A hit object's lane is `floor(x * 4 / 512)`
/// clamped to 0-3. Times are rounded to whole milliseconds. A hold's end before its start, or missing,
/// makes a zero-length hold.
pub fn parse_osu(text: &str) -> Result<OsuBeatmap, OsuError> {
    let mut section = "";
    let mut general = HashMap::new();
    let mut metadata = HashMap::new();
    let mut difficulty = HashMap::new();
    let mut hit_object_lines = Vec::new();
    let line_breaks = [
        '\n', '\u{b}', '\u{c}', '\r', '\u{85}', '\u{2028}', '\u{2029}',
    ];
    for line in text.split(line_breaks).map(str::trim) {
        if line.is_empty() || line.starts_with("//") {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            section = name;
            continue;
        }
        let fields = match section {
            "General" => &mut general,
            "Metadata" => &mut metadata,
            "Difficulty" => &mut difficulty,
            "HitObjects" => {
                hit_object_lines.push(line);
                continue;
            }
            _ => continue,
        };
        if let Some((key, value)) = line.split_once(':') {
            fields.insert(key.trim(), value.trim());
        }
    }

    if general
        .get("Mode")
        .and_then(|mode| mode.parse::<i64>().ok())
        != Some(3)
    {
        return Err(OsuError::UnsupportedMode);
    }
    let circle_size = difficulty
        .get("CircleSize")
        .and_then(|size| size.parse::<f64>().ok())
        .ok_or(OsuError::UnsupportedKeyCount { keys: 0 })?;
    let four_keys = (circle_size - 4.0).abs() < 0.0001;
    if !four_keys {
        return Err(OsuError::UnsupportedKeyCount {
            keys: circle_size.round() as i64,
        });
    }

    let mut objects = hit_object_lines
        .iter()
        .map(|line| {
            hit_object(line).ok_or_else(|| OsuError::UnsupportedHitObject {
                line: line.to_string(),
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    // Stable: notes at the same time and lane keep their file order (and then overlap).
    objects.sort_by_key(|object| (object.start_ms, object.lane));
    let chart = Chart {
        duration_ms: objects
            .iter()
            .map(|object| object.end_ms.unwrap_or(object.start_ms))
            .max()
            .unwrap_or(0),
        objects,
    };
    chart
        .validate()
        .map_err(|error| OsuError::InvalidChart { error })?;
    let text_field = |unicode: &str, plain: &str| {
        metadata
            .get(unicode)
            .or(metadata.get(plain))
            .map(|value| value.to_string())
    };
    Ok(OsuBeatmap {
        title: text_field("TitleUnicode", "Title"),
        artist: text_field("ArtistUnicode", "Artist"),
        chart,
    })
}

/// `x,y,time,type,hitSound,endTime:hitSample` for a hold (type bit 128), `x,y,time,type,...` for a note
/// (bit 1). Sliders and spinners are not mania notes.
fn hit_object(line: &str) -> Option<ChartObject> {
    let fields: Vec<&str> = line.split(',').collect();
    if fields.len() < 5 {
        return None;
    }
    let x: i64 = fields[0].parse().ok()?;
    let time: f64 = fields[2].parse().ok()?;
    let kind: i64 = fields[3].parse().ok()?;
    if !(0.0..f64::INFINITY).contains(&time) {
        return None;
    }
    let lane = ((x as f64 * 4.0 / 512.0).floor() as i64).clamp(0, 3) as u32;
    let (circle, slider, spinner, hold) =
        (kind & 1 != 0, kind & 2 != 0, kind & 8 != 0, kind & 128 != 0);
    if slider || spinner || (circle && hold) || !(circle || hold) {
        return None;
    }
    let end = if !hold {
        None
    } else if let Some(end) = fields.get(5).filter(|field| !field.is_empty()) {
        let end: f64 = end.split(':').next()?.parse().ok()?;
        if !end.is_finite() {
            return None;
        }
        Some(end.max(time))
    } else {
        Some(time)
    };
    let millisecond = |time: f64| {
        let time = time.round();
        (time <= f64::from(u32::MAX)).then_some(time as u32)
    };
    Some(ChartObject {
        lane,
        start_ms: millisecond(time)?,
        end_ms: match end {
            Some(end) => Some(millisecond(end)?),
            None => None,
        },
    })
}
