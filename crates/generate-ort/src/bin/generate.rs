//! Generates a chart natively from raw PCM, as the Swift API does: mono float32 samples, little-endian, at
//! the given rate, with no frontend streams and no controls. Writes the `.osu` text.
//!
//! Usage: generate --model-dir <onnx> --assets <resampler tables> --pcm <file.f32> --sample-rate <hz>
//!        --star <star> [--seed <seed>] [--title <t>] [--artist <a>] [--audio-name <name>] --out <chart.osu>
use ensomi_generate::{generate, resampler_files, Audio, Options, Request, Samples};
use ensomi_generate_ort::{drive, package, Config, Sessions};
use serde_json::json;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

fn main() -> Result<(), String> {
    let mut args: HashMap<String, String> = HashMap::new();
    let mut argv = std::env::args().skip(1);
    while let (Some(key), Some(value)) = (argv.next(), argv.next()) {
        args.insert(key.trim_start_matches("--").to_string(), value);
    }
    let arg = |key: &str| args.get(key).cloned().ok_or_else(|| format!("--{key} is required"));
    let model_dir = PathBuf::from(arg("model-dir")?);
    let sample_rate: f64 = arg("sample-rate")?.parse().map_err(|_| "--sample-rate is a number")?;
    let pcm: Vec<f32> = std::fs::read(arg("pcm")?).map_err(|e| e.to_string())?.as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes(*c)).collect();
    let resampler = match resampler_files(sample_rate) {
        Some(names) => {
            let assets = PathBuf::from(arg("assets")?);
            Some(names.map(|n| std::fs::read(assets.join(n)).unwrap()))
        }
        None => None,
    };
    // The request the Swift API builds for a request without controls.
    let request = json!({"star": arg("star")?.parse::<f64>().map_err(|_| "--star is a number")?, "controls": {"long_notes": 0, "sections": []}, "title": args.get("title"), "artist": args.get("artist")});
    let seed: u64 = args.get("seed").map_or(Ok(0), |s| s.parse()).map_err(|_| "--seed is an unsigned integer")?;
    let (models, _) = package::open(&model_dir)?;
    let models = Arc::new(models);
    let mut sessions = Sessions::load(&model_dir, &models, &Config::default())?;
    let start = Instant::now();
    let options = Options { random: None, setting: None, audio_name: args.get("audio-name").cloned(), trace: None, clock: Rc::new(move || start.elapsed().as_secs_f64() * 1000.0) };
    let audio = Audio { samples: Samples::F32(pcm), channels: 1, sample_rate, mel: None, beatthis: None, resampler };
    let mut generator = generate(models.clone(), audio, Request::from_json(&request.to_string(), seed, seed > (1 << 53) - 1)?, options);
    let (result, _) = drive(&mut generator, &mut sessions)?;
    std::fs::write(arg("out")?, &result.osu).map_err(|e| e.to_string())?;
    println!("{} objects in {:.2} s", result.chart.objects.len(), start.elapsed().as_secs_f64());
    Ok(())
}
