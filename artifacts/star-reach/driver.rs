//! Persistent JSON-lines worker for star-reach. IO and model execution stay in this native driver.
//! cargo run --release -p ensomi-generate-ort --example star-reach -- <onnx> <resampler-assets>
//! Commands: {op:prepare, pcm, sample_rate, prepared}, {op:generate, prepared, star, seed, out}.
use ensomi_generate::{
    Audio, Models, Options, Prepared, Request, Samples, generate_prepared, prepare, resampler_files,
};
use ensomi_generate_ort::{Config, Sessions, drive, package, performance_cores};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

fn options() -> Options {
    let start = Instant::now();
    Options::new(Rc::new(move || start.elapsed().as_secs_f64() * 1000.0))
}

fn field<'a>(v: &'a Value, k: &str) -> Result<&'a str, String> {
    v[k].as_str().ok_or_else(|| format!("missing string {k}"))
}

fn command(
    v: Value,
    models: &Arc<Models>,
    sessions: &mut Sessions,
    cache: &mut HashMap<String, Arc<Prepared>>,
    assets: &Path,
) -> Result<Value, String> {
    let path = field(&v, "prepared")?;
    if v["op"] == "prepare" {
        let rate = v["sample_rate"].as_f64().ok_or("missing sample_rate")?;
        let bytes = std::fs::read(field(&v, "pcm")?).map_err(|e| e.to_string())?;
        if bytes.len() % 4 != 0 {
            return Err("partial float32 sample".into());
        }
        let pcm = bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|c| f32::from_le_bytes(*c))
            .collect();
        let resampler = if let Some(names) = resampler_files(rate) {
            let mut files = Vec::new();
            for n in names {
                files.push(std::fs::read(assets.join(n)).map_err(|e| e.to_string())?);
            }
            Some(files.try_into().unwrap())
        } else {
            None
        };
        let audio = Audio {
            samples: Samples::F32(pcm),
            channels: 1,
            sample_rate: rate,
            mel: None,
            beatthis: None,
            resampler,
        };
        let (prepared, _) = drive(&mut prepare(models.clone(), audio, options()), sessions)?;
        std::fs::write(path, prepared.to_bytes()).map_err(|e| e.to_string())?;
        let response = json!({"song_ms": prepared.song_ms, "segments": prepared.segments.iter().map(|s| [s.offset_ms, s.beat_length_ms]).collect::<Vec<_>>()});
        cache.insert(path.into(), Arc::new(prepared));
        return Ok(response);
    }
    if v["op"] != "generate" {
        return Err("unknown operation".into());
    }
    if !cache.contains_key(path) {
        let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
        cache.insert(path.into(), Arc::new(Prepared::from_bytes(&bytes)?));
    }
    let prepared = cache[path].clone();
    let seed = v["seed"].as_u64().ok_or("missing unsigned seed")?;
    let request = Request::from_json(&json!({"star": v["star"]}).to_string(), seed, false)?;
    let opts = options();
    let (result, _) = drive(
        &mut generate_prepared(models.clone(), prepared, request, opts),
        sessions,
    )?;
    let out = field(&v, "out")?;
    std::fs::write(format!("{out}.osu"), &result.osu).map_err(|e| e.to_string())?;
    std::fs::write(format!("{out}.receipt.json"), &result.receipt.receipt_json)
        .map_err(|e| e.to_string())?;
    Ok(json!({"song_ms": result.chart.duration_ms, "objects": result.chart.objects.len()}))
}

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        return Err("usage: star-reach <onnx> <resampler-assets>".into());
    }
    let dir = Path::new(&args[1]);
    let (models, _) = package::open(dir)?;
    let models = Arc::new(models);
    let mut sessions = Sessions::load(
        dir,
        &models,
        &Config::with_beatthis_threads(performance_cores()),
    )?;
    let mut cache = HashMap::new();
    let mut stdout = std::io::stdout().lock();
    for line in std::io::stdin().lock().lines() {
        let response = line
            .map_err(|e| e.to_string())
            .and_then(|s| serde_json::from_str(&s).map_err(|e| e.to_string()))
            .and_then(|v| command(v, &models, &mut sessions, &mut cache, Path::new(&args[2])));
        let response = match response {
            Ok(v) => json!({"ok": v}),
            Err(e) => json!({"error": e}),
        };
        writeln!(stdout, "{response}").map_err(|e| e.to_string())?;
        stdout.flush().map_err(|e| e.to_string())?;
    }
    Ok(())
}
