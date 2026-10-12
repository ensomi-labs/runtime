//! What the audio stages keep for the rest of a generation, and its stored form.
//!
//! [`Prepared::to_bytes`] writes the safetensors layout: an 8-byte little-endian header length, a JSON header naming
//! each value's type (`F32`, `F64`), shape and byte range, padded with spaces to 8 bytes, then the values in
//! little-endian order. The header's `__metadata__` holds the format tag, the runtime version and the model, the
//! key fields a stored copy is valid for besides the audio itself, which the caller keys its cache by.
use crate::capabilities::{MODEL_NAME, MODEL_VERSION};
use crate::fitter::Segment;
use serde_json::{json, Value};

const FORMAT: &str = "ensomi-prepared-v1";
/// Row features per 10 ms frame (`audio::features`).
const FEATURES: usize = 130;

/// The audio stages' results that the request-dependent stages read: the rows model's features, the corrected beat
/// grid and the song's length. One per song; every request and seed generates from it.
#[derive(Debug)]
pub struct Prepared {
    /// 130 features per 10 ms frame.
    pub features: Vec<f32>,
    /// The beat grid's segments, as the `.osu` timing points and the receipt give them.
    pub segments: Vec<Segment>,
    pub song_ms: f64,
}

fn model() -> String {
    format!("{MODEL_NAME} {MODEL_VERSION}")
}

impl Prepared {
    pub fn to_bytes(&self) -> Vec<u8> {
        let features: Vec<u8> = self.features.iter().flat_map(|x| x.to_le_bytes()).collect();
        let segments: Vec<u8> = self.segments.iter().flat_map(|s| [s.offset_ms, s.beat_length_ms]).flat_map(|x| x.to_le_bytes()).collect();
        let (a, b) = (features.len(), features.len() + segments.len());
        let header = json!({
            "__metadata__": {"format": FORMAT, "runtime": crate::RUNTIME_VERSION, "model": model()},
            "features": {"dtype": "F32", "shape": [self.features.len() / FEATURES, FEATURES], "data_offsets": [0, a]},
            "segments": {"dtype": "F64", "shape": [self.segments.len(), 2], "data_offsets": [a, b]},
            "song_ms": {"dtype": "F64", "shape": [], "data_offsets": [b, b + 8]},
        })
        .to_string();
        let padded = header.len().next_multiple_of(8);
        let mut out = Vec::with_capacity(8 + padded + b + 8);
        out.extend((padded as u64).to_le_bytes());
        out.extend(header.bytes());
        out.resize(8 + padded, b' ');
        out.extend(features);
        out.extend(segments);
        out.extend(self.song_ms.to_le_bytes());
        out
    }

    /// Reads [`Prepared::to_bytes`]'s output, refusing bytes of another format, runtime version or model.
    pub fn from_bytes(bytes: &[u8]) -> Result<Prepared, String> {
        let length = bytes.get(..8).map(|b| u64::from_le_bytes(b.try_into().unwrap())).ok_or_else(|| invalid("no header"))?;
        let data_start = usize::try_from(length).ok().and_then(|n| n.checked_add(8)).filter(|&n| n <= bytes.len()).ok_or_else(|| invalid("header longer than the bytes"))?;
        let header: Value = serde_json::from_slice(&bytes[8..data_start]).map_err(|e| invalid(&e.to_string()))?;
        let meta = &header["__metadata__"];
        if meta["format"] != FORMAT {
            return Err(invalid("not an ensomi-prepared-v1 file"));
        }
        if meta["runtime"] != crate::RUNTIME_VERSION || meta["model"] != model() {
            return Err(format!("prepared: made by runtime {} for {}; this is runtime {} for {}", meta["runtime"], meta["model"], crate::RUNTIME_VERSION, model()));
        }
        let data = &bytes[data_start..];
        let (features, shape) = value(&header, data, "features", "F32", 4)?;
        if shape.len() != 2 || shape[1] != FEATURES {
            return Err(invalid("features are not 130 per frame"));
        }
        let (segments, shape) = value(&header, data, "segments", "F64", 8)?;
        if shape.len() != 2 || shape[1] != 2 || shape[0] == 0 {
            return Err(invalid("segments are not offset and beat length pairs"));
        }
        let (song_ms, shape) = value(&header, data, "song_ms", "F64", 8)?;
        if !shape.is_empty() {
            return Err(invalid("song_ms is not a number"));
        }
        let f64s = |b: &[u8]| b.as_chunks::<8>().0.iter().map(|c| f64::from_le_bytes(*c)).collect::<Vec<f64>>();
        Ok(Prepared {
            features: features.as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes(*c)).collect(),
            segments: f64s(segments).as_chunks::<2>().0.iter().map(|&[offset_ms, beat_length_ms]| Segment { offset_ms, beat_length_ms }).collect(),
            song_ms: f64s(song_ms)[0],
        })
    }
}

fn invalid(what: &str) -> String {
    format!("prepared: {what}")
}

/// A value's bytes and shape, after checking its type and that its shape and byte range agree.
fn value<'a>(header: &Value, data: &'a [u8], name: &str, dtype: &str, size: usize) -> Result<(&'a [u8], Vec<usize>), String> {
    let entry = &header[name];
    let shape: Option<Vec<usize>> = entry["shape"].as_array().and_then(|s| s.iter().map(|d| d.as_u64().map(|d| d as usize)).collect());
    let range: Vec<usize> = entry["data_offsets"].as_array().into_iter().flatten().filter_map(|d| d.as_u64().map(|d| d as usize)).collect();
    let length = shape.as_ref().and_then(|shape| shape.iter().try_fold(size, |n, &d| n.checked_mul(d)));
    match (range.as_slice(), length, shape) {
        (&[start, end], Some(n), Some(shape)) if entry["dtype"] == dtype && start <= end && end <= data.len() && end - start == n => Ok((&data[start..end], shape)),
        _ => Err(invalid(&format!("{name} does not match its type, shape or range"))),
    }
}
