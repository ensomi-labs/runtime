//! Reader for `ensomi-runtime-reference-v1` recordings: `index.json` and the `arrays.bin` it addresses.
use ensomi_generate::{Data, Draw, Tensor};
use serde_json::Value;
use std::collections::HashMap;
use std::fs::File;
use std::os::unix::fs::FileExt;
use std::path::Path;

pub struct Recording {
    pub index: Value,
    file: File,
    events: HashMap<String, usize>,
}

pub struct Array {
    pub dtype: String,
    pub shape: Vec<usize>,
    pub bytes: Vec<u8>,
}

impl Array {
    /// The elements as doubles (booleans as 0 and 1, float16 widened).
    pub fn f64s(&self) -> Vec<f64> {
        match self.dtype.as_str() {
            "<f8" => self.bytes.chunks_exact(8).map(|c| f64::from_le_bytes(c.try_into().unwrap())).collect(),
            "<f4" => self.bytes.chunks_exact(4).map(|c| f32::from_le_bytes(c.try_into().unwrap()) as f64).collect(),
            "<i8" => self.bytes.chunks_exact(8).map(|c| i64::from_le_bytes(c.try_into().unwrap()) as f64).collect(),
            "<f2" => self.bytes.chunks_exact(2).map(|c| half_to_f64(u16::from_le_bytes(c.try_into().unwrap()))).collect(),
            "|b1" | "|u1" => self.bytes.iter().map(|&b| b as f64).collect(),
            other => panic!("unsupported recorded dtype {other}"),
        }
    }

    pub fn f32s(&self) -> Vec<f32> {
        assert_eq!(self.dtype, "<f4");
        self.bytes.chunks_exact(4).map(|c| f32::from_le_bytes(c.try_into().unwrap())).collect()
    }

    pub fn tensor(&self) -> Tensor {
        match self.dtype.as_str() {
            "<f4" => Tensor { shape: self.shape.clone(), data: Data::F32(self.f32s()) },
            "<i8" => Tensor { shape: self.shape.clone(), data: Data::I64(self.bytes.chunks_exact(8).map(|c| i64::from_le_bytes(c.try_into().unwrap())).collect()) },
            "|b1" => Tensor { shape: self.shape.clone(), data: Data::Bool(self.bytes.clone()) },
            other => panic!("unsupported recorded tensor dtype {other}"),
        }
    }
}

fn half_to_f64(h: u16) -> f64 {
    let sign = if h >> 15 == 1 { -1.0 } else { 1.0 };
    let exponent = ((h >> 10) & 0x1f) as i32;
    let fraction = (h & 0x3ff) as f64;
    match exponent {
        0 => sign * fraction * 2f64.powi(-24),
        31 => if fraction == 0.0 { sign * f64::INFINITY } else { f64::NAN },
        e => sign * (1.0 + fraction / 1024.0) * 2f64.powi(e - 15),
    }
}

impl Recording {
    pub fn open(directory: &Path) -> Result<Recording, String> {
        let text = std::fs::read_to_string(directory.join("index.json")).map_err(|e| format!("{}: {e}", directory.display()))?;
        let index: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        if index["format"] != "ensomi-runtime-reference-v1" {
            return Err("Unsupported reference format".to_string());
        }
        let file = File::open(directory.join("arrays.bin")).map_err(|e| e.to_string())?;
        let events = index["events"].as_array().ok_or("no events")?.iter().enumerate().map(|(i, e)| (e["name"].as_str().unwrap_or_default().to_string(), i)).collect();
        Ok(Recording { index, file, events })
    }

    pub fn array(&self, id: usize) -> Array {
        let entry = &self.index["arrays"][id];
        let nbytes = entry["nbytes"].as_u64().unwrap() as usize;
        let mut bytes = vec![0u8; nbytes];
        self.file.read_exact_at(&mut bytes, entry["byte_offset"].as_u64().unwrap()).unwrap();
        Array { dtype: entry["dtype"].as_str().unwrap().to_string(), shape: entry["shape"].as_array().unwrap().iter().map(|d| d.as_u64().unwrap() as usize).collect(), bytes }
    }

    /// The arrays of a named event, by key.
    pub fn event(&self, name: &str) -> Option<HashMap<String, Array>> {
        let e = &self.index["events"][*self.events.get(name)?];
        Some(e["arrays"].as_object()?.iter().map(|(k, v)| (k.clone(), self.array(v.as_u64().unwrap() as usize))).collect())
    }

    pub fn graph_calls(&self) -> &Vec<Value> {
        self.index["graphs"].as_array().unwrap()
    }

    pub fn draws(&self) -> Vec<Draw> {
        self.index["draws"]
            .as_array()
            .unwrap()
            .iter()
            .map(|d| Draw {
                name: d["name"].as_str().unwrap().to_string(),
                kind: d["kind"].as_str().unwrap().to_string(),
                values: self.array(d["values"].as_u64().unwrap() as usize).f64s(),
                chosen: d["chosen"].as_u64().unwrap() as usize,
            })
            .collect()
    }
}
