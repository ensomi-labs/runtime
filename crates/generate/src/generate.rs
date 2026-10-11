//! generate.ts: the whole pipeline from PCM and a request to the chart, `.osu` text and receipt.
use crate::arrange::{arrange, Arrange, Arrangement};
use crate::audio::{beat_activations, features, load_resampler, mel_pcm, mono, resample, resampler_files, sinc_resample, spectrogram, Samples};
use crate::capabilities::{MODEL_NAME, MODEL_VERSION};
use crate::controls::{load_scopes, number, resolve_requests, Scope};
use crate::export::{audio_filename, chart_from_decisions, osu_text, validate_chart, Chart, OsuOptions};
use crate::fitter::{fit_grid, Segment};
use crate::grid::{corrected_grid, Grid};
use crate::json::{number_string, stringify};
use crate::jsmath;
use crate::machine::{Generator, Io, Phase};
use crate::math::{clip, quantile};
use crate::models::Models;
use crate::prior::{kappa_targets, predict_prior, rows_features, Prediction};
use crate::random::{RandomSource, SeededRandom};
use crate::rows::{decode_rows, RowsSection};
use serde_json::{json, Map, Value};
use std::rc::Rc;

pub struct Audio {
    /// Interleaved samples at `sample_rate`.
    pub samples: Samples,
    pub channels: usize,
    pub sample_rate: f64,
    /// Already-decoded frontend streams: mono, 24000 and 22050 Hz.
    pub mel: Option<Vec<f32>>,
    pub beatthis: Option<Vec<f32>>,
    /// The soxr table files for `sample_rate` ([`crate::resampler_files`]), needed when `beatthis` is
    /// absent and the rate is one the release bundles a table for.
    pub resampler: Option<[Vec<u8>; 3]>,
}

pub struct Pattern {
    pub kind: i64,
    pub strength: f64,
}

pub struct Section {
    pub start_ms: f64,
    pub end_ms: f64,
    pub star: Option<f64>,
    pub pattern: Option<Pattern>,
    /// The section as the caller wrote it; the receipt records star sections verbatim.
    pub raw: Map<String, Value>,
}

#[derive(Default)]
pub struct Controls {
    pub long_notes: i64,
    pub sections: Vec<Section>,
}

pub struct Request {
    /// The audio asset reference, recorded in the receipt as given.
    pub audio: Option<Value>,
    pub star: f64,
    pub seed: u64,
    /// The receipt writes a seed that was a BigInt as a string.
    pub seed_is_bigint: bool,
    pub controls: Controls,
    pub title: Option<String>,
    pub artist: Option<String>,
}

fn field_f64(v: &Value, key: &str) -> Option<f64> {
    v.get(key).and_then(Value::as_f64)
}

impl Request {
    /// Reads `{audio?, star, controls?: {long_notes?, sections?}, title?, artist?}`.
    pub fn from_json(text: &str, seed: u64, seed_is_bigint: bool) -> Result<Request, String> {
        let v: Value = serde_json::from_str(text).map_err(|e| format!("request: {e}"))?;
        let controls = &v["controls"];
        let sections = controls["sections"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|s| Section {
                        start_ms: field_f64(s, "start_ms").unwrap_or(f64::NAN),
                        end_ms: field_f64(s, "end_ms").unwrap_or(f64::NAN),
                        star: field_f64(s, "star"),
                        pattern: s.get("pattern").filter(|p| p.is_object()).map(|p| Pattern { kind: p["kind"].as_i64().unwrap_or(-1), strength: field_f64(p, "strength").unwrap_or(f64::NAN) }),
                        raw: s.as_object().cloned().unwrap_or_default(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok(Request {
            audio: v.get("audio").filter(|a| !a.is_null()).cloned(),
            star: field_f64(&v, "star").unwrap_or(f64::NAN),
            seed,
            seed_is_bigint,
            controls: Controls { long_notes: controls["long_notes"].as_i64().unwrap_or(0), sections },
            title: v["title"].as_str().map(str::to_string),
            artist: v["artist"].as_str().map(str::to_string),
        })
    }
}

pub struct Receipt {
    pub model_name: String,
    pub model_version: String,
    pub seed: u64,
    pub seed_is_bigint: bool,
    pub star: f64,
    pub receipt_json: String,
}

pub struct GenerationResult {
    pub chart: Chart,
    pub receipt: Receipt,
    pub osu: String,
}

/// Intermediate values for conformance checks. Return an error to stop the generation.
pub enum Trace<'a> {
    Listener { spectrogram: &'a [f32], log_mel: &'a [f32], beat_logits: &'a [f32], downbeat_logits: &'a [f32], segments: &'a [Segment], grid: &'a Grid, song_ms: f64 },
    Rows { head_ms: &'a [f64] },
    Prior { prediction: &'a Prediction },
    Chosen { name: String, codes: [i32; 4], releases: [f64; 4] },
}

pub type TraceFn = Box<dyn for<'a> FnMut(Trace<'a>) -> Result<(), String>>;

pub struct Options {
    /// Replaces the seeded generator, e.g. to replay recorded draws.
    pub random: Option<Box<dyn RandomSource>>,
    /// The difficulty name in the `.osu` version; `r2` by default.
    pub setting: Option<String>,
    /// The name of the original audio file, for the `.osu` audio filename.
    pub audio_name: Option<String>,
    pub trace: Option<TraceFn>,
    /// Milliseconds from any fixed origin; the core reads time only through this.
    pub clock: Rc<dyn Fn() -> f64>,
}

fn all_finite_f32(a: &[f32]) -> bool {
    a.iter().all(|x| x.is_finite())
}

fn validate(audio: &Audio, request: &Request) -> Result<f64, String> {
    let len = audio.samples.len();
    if !audio.sample_rate.is_finite() || audio.sample_rate <= 0.0 || audio.channels < 1 || len == 0 || len % audio.channels != 0 {
        return Err("PCM needs nonempty interleaved samples, a positive sample rate and a channel count".to_string());
    }
    let finite = match &audio.samples {
        Samples::F32(v) => all_finite_f32(v),
        Samples::F64(v) => v.iter().all(|x| x.is_finite()),
    };
    if !finite {
        return Err("PCM samples must be finite".to_string());
    }
    for stream in [&audio.mel, &audio.beatthis].into_iter().flatten() {
        if stream.is_empty() {
            return Err("Frontend PCM must be nonempty".to_string());
        }
        if !all_finite_f32(stream) {
            return Err("Frontend PCM samples must be finite".to_string());
        }
    }
    let duration = len as f64 / audio.channels as f64 / audio.sample_rate * 1000.0;
    if duration < 3000.0 {
        return Err("Audio must be at least three seconds long to fit a beat grid".to_string());
    }
    if !(request.star >= 2.0 && request.star < 6.0) {
        return Err("star_out_of_range: star must be in [2, 6)".to_string());
    }
    if ![0, 1, 2, 3].contains(&request.controls.long_notes) {
        return Err("unsupported_control: unknown long_notes value".to_string());
    }
    let integer = |x: f64| x.is_finite() && x.fract() == 0.0;
    for s in &request.controls.sections {
        if !integer(s.start_ms) || !integer(s.end_ms) || s.start_ms < 0.0 || s.end_ms > duration || s.end_ms <= s.start_ms || s.end_ms - s.start_ms < 8000.0 || s.end_ms - s.start_ms > 32000.0 {
            return Err("unsupported_control: sections must be 8–32 seconds within the song".to_string());
        }
        if s.star.is_some() == s.pattern.is_some() {
            return Err("unsupported_control: each section needs exactly one star or pattern target".to_string());
        }
        if s.star.is_some_and(|star| !(star >= 2.0 && star < 6.0)) {
            return Err("star_out_of_range: section star must be in [2, 6)".to_string());
        }
        if s.pattern.as_ref().is_some_and(|p| ![1, 2, 3, 4, 5].contains(&p.kind) || !(p.strength >= 0.0 && p.strength <= 1.0)) {
            return Err("unsupported_control: pattern kind and strength in [0, 1] are required".to_string());
        }
    }
    let mut ordered: Vec<&Section> = request.controls.sections.iter().collect();
    ordered.sort_by(|a, b| crate::fitter::order(a.start_ms - b.start_ms));
    if ordered.windows(2).any(|w| w[1].start_ms < w[0].end_ms) {
        return Err("unsupported_control: sections must not overlap".to_string());
    }
    Ok(duration)
}

/// Starts a generation. Drive the returned [`Generator`] with [`Generator::step`].
pub fn generate(models: Rc<Models>, audio: Audio, request: Request, options: Options) -> Generator {
    let io = Io::default();
    let future = run(io.clone(), models, audio, request, options);
    Generator::new(io, future)
}

fn seconds_since(clock: &dyn Fn() -> f64, start: f64) -> f64 {
    (clock() - start) / 1000.0
}

fn mean_of(values: impl Iterator<Item = i32>, count: usize) -> f64 {
    values.fold(0.0, |a, b| a + b as f64) / count as f64
}

async fn run(io: Io, models: Rc<Models>, audio: Audio, request: Request, mut options: Options) -> Result<GenerationResult, String> {
    validate(&audio, &request)?;
    let clock = options.clock.clone();
    let all_start = clock();
    let rate = audio.sample_rate;
    let song_ms = 1000.0 * audio.samples.len() as f64 / audio.channels as f64 / rate;
    let seed32 = (request.seed & 0xffff_ffff) as u32;
    let r2_seed32 = (request.seed.wrapping_add(954) & 0xffff_ffff) as u32;
    let mut random: Box<dyn RandomSource> = options.random.take().unwrap_or_else(|| Box::new(SeededRandom::new(seed32)));
    let mut seconds: Vec<(&str, f64)> = Vec::new();
    io.progress(Phase::Loading).await;
    let wave = mono(&audio.samples, audio.channels);
    drop(audio.samples);
    let mut start = clock();
    let beat_pcm: Vec<f32> = match audio.beatthis {
        Some(b) => b,
        None if rate == 22050.0 => wave.iter().map(|&x| x as f32).collect(),
        None if resampler_files(rate).is_some() => resample(&wave, &load_resampler(&audio.resampler.ok_or_else(|| format!("PCM at {rate} Hz needs its resampler table files"))?)?),
        None => sinc_resample(&wave, rate, 22050.0),
    };
    let rows_pcm: Vec<f32> = match audio.mel {
        Some(m) => m,
        None => mel_pcm(&wave, rate),
    };
    drop(wave);
    seconds.push(("pcm", seconds_since(&*clock, start)));
    io.progress(Phase::Timing).await;
    start = clock();
    let spect = spectrogram(&beat_pcm, &models.f32s(&models.beatthis_window)?, &models.f32s(&models.beatthis_filter)?, true);
    seconds.push(("spectrogram", seconds_since(&*clock, start)));
    start = clock();
    let acts = beat_activations(&io, &spect).await?;
    seconds.push(("beatthis", seconds_since(&*clock, start)));
    start = clock();
    let log_mel = spectrogram(&rows_pcm, &models.f32s(&models.rows_window)?, &models.f32s(&models.rows_filter)?, false);
    let feats = features(&log_mel, &acts.beat_probability, &acts.downbeat_probability);
    seconds.push(("mel", seconds_since(&*clock, start)));
    start = clock();
    let segments = fit_grid(&acts.beat, &acts.downbeat, &log_mel, song_ms, &models.fitter);
    let corrected = corrected_grid(&segments, song_ms);
    let grid = Rc::new(Grid::new(&corrected));
    seconds.push(("grid", seconds_since(&*clock, start)));
    if let Some(t) = options.trace.as_mut() {
        t(Trace::Listener { spectrogram: &spect, log_mel: &log_mel, beat_logits: &acts.beat, downbeat_logits: &acts.downbeat, segments: &segments, grid: &grid, song_ms })?;
    }
    io.progress(Phase::Rows).await;
    random.reset(seed32);
    start = clock();
    let sections = &request.controls.sections;
    let star_sections: Vec<RowsSection> = sections.iter().filter_map(|s| s.star.map(|star| RowsSection { start_ms: s.start_ms, end_ms: s.end_ms, star })).collect();
    let rows = decode_rows(&io, &models, &feats, &grid, song_ms, request.star, &mut *random, &star_sections).await?;
    seconds.push(("rows", seconds_since(&*clock, start)));
    if rows.head_ms.len() < 2 {
        return Err("generation_failed: the rows model placed fewer than two heads".to_string());
    }
    if let Some(t) = options.trace.as_mut() {
        t(Trace::Rows { head_ms: &rows.head_ms })?;
    }
    io.progress(Phase::Arrangement).await;
    start = clock();
    let band = f64::min(5.0, request.star.floor()) as u32;
    let prior_features = rows_features(&rows.head_ms, song_ms, &grid);
    let prediction = predict_prior(&models.prior, &prior_features, band)?;
    if let Some(t) = options.trace.as_mut() {
        t(Trace::Prior { prediction: &prediction })?;
    }
    let pattern_names: [(&str, Option<&str>); 6] = [("", None), ("stream", None), ("stream", Some("jumpstream")), ("stream", Some("chordstream")), ("jack", None), ("jack", Some("chordjack"))];
    let external: Vec<Scope> = sections
        .iter()
        .filter_map(|s| {
            let p = s.pattern.as_ref()?;
            let (concept, subtype) = pattern_names[p.kind as usize];
            let mut scope = Scope::new();
            scope.insert("concept".into(), json!(concept));
            if let Some(subtype) = subtype {
                scope.insert("subtype".into(), json!(subtype));
            }
            scope.insert("a_ms".into(), number(s.start_ms));
            scope.insert("b_ms".into(), number(s.end_ms));
            scope.insert("q".into(), number(p.strength));
            Some(scope)
        })
        .collect();
    let requests: Vec<Scope> = if external.is_empty() { Vec::new() } else { resolve_requests(&rows.head_ms, song_ms, &external, band, &models.calibration, &load_scopes(&models)?)? };
    let ln_mode = request.controls.long_notes;
    let ln_fraction = if ln_mode == 1 { 0.0 } else if ln_mode == 2 { 0.5 } else { 1.0 };
    seconds.push(("prior", seconds_since(&*clock, start)));
    start = clock();
    let band_key = band.to_string();
    let masks = models.calibration.masks.get(&band_key).ok_or_else(|| format!("The calibration has no band {band}"))?.clone();
    let default_q = *models.default_q.get(&band_key).ok_or_else(|| format!("The system has no default q for band {band}"))?;
    let base_r = jsmath::exp(prediction.log_r);
    let fit = models.calibration.difficulty_fit.by_band.get(&band_key).ok_or_else(|| format!("The difficulty fit has no band {band}"))?;
    let fit_number = |key: &str| fit[key].as_f64().unwrap_or(f64::NAN);
    let (alpha, beta, gamma, seed_sd) = (fit_number("alpha"), fit_number("beta"), fit_number("gamma"), fit_number("seed_sd"));
    let values = &models.calibration.difficulty.get(&band_key).ok_or_else(|| format!("The calibration has no difficulty values for band {band}"))?.values;
    let target_r = quantile(values, default_q);
    let domain = &models.calibration.difficulty_fit.domain;
    let (lo, hi) = (domain[0], f64::min(domain[1], 1.2));
    let raw = jsmath::exp((jsmath::log(target_r) - alpha - gamma * jsmath::log(base_r)) / beta);
    let mut multiplier = clip(raw, lo, hi);
    let mut passes: Vec<Value> = Vec::new();
    let mut result: Option<Arrangement> = None;
    for attempt in 0..2 {
        let pass_start = clock();
        random.reset(r2_seed32);
        let pass = attempt + 1;
        let arrangement = arrange(Arrange {
            io: &io,
            models: &models,
            head_ms: &rows.head_ms,
            song_ms,
            grid: grid.clone(),
            random: &mut *random,
            target: kappa_targets(&prediction, ln_fraction, multiplier),
            ln_fraction,
            requests: &requests,
            masks: masks.clone(),
            prefix: format!("r2_pass_{pass}"),
            trace: &mut options.trace,
        })
        .await?;
        passes.push(json!({"multiplier": number(multiplier), "achieved": number(arrangement.r), "fallback_count": arrangement.fallback_count, "wall_seconds": number(seconds_since(&*clock, pass_start))}));
        let achieved = arrangement.r;
        result = Some(arrangement);
        if attempt > 0 || (achieved - target_r).abs() <= seed_sd {
            break;
        }
        let corrected_multiplier = clip(multiplier * jsmath::exp((jsmath::log(target_r) - jsmath::log(achieved)) / beta), lo, hi);
        if corrected_multiplier == multiplier {
            break;
        }
        multiplier = corrected_multiplier;
    }
    let result = result.unwrap();
    seconds.push(("arrangement", seconds_since(&*clock, start)));
    io.progress(Phase::Export).await;
    start = clock();
    let chart = chart_from_decisions(&rows.head_ms, song_ms, &result.chart.actions, &result.chart.gap);
    validate_chart(&chart)?;
    let setting: String = options.setting.as_deref().unwrap_or("r2").encode_utf16().map(|u| if u < 128 && ((u as u8).is_ascii_alphanumeric() || u == b'_' as u16 || u == b'-' as u16) { char::from(u as u8) } else { '-' }).collect();
    let title = request.title.clone().unwrap_or_else(|| "Ensomi".to_string());
    let artist = request.artist.clone().unwrap_or_else(|| "Unknown".to_string());
    let audio_ref = |key: &str| request.audio.as_ref().and_then(|a| a.get(key)).and_then(Value::as_str).map(str::to_string);
    let name = audio_filename(&options.audio_name.clone().or_else(|| audio_ref("local_path")).or_else(|| audio_ref("uri")).unwrap_or_else(|| "audio.wav".to_string()));
    let bookmarks: Vec<f64> = requests.iter().flat_map(|r| [crate::controls::get(r, "a_ms"), crate::controls::get(r, "b_ms")]).collect();
    let seed_text = request.seed.to_string();
    let osu = osu_text(&chart, &corrected, &OsuOptions { title: &title, artist: &artist, setting: &setting, seed: seed_text.clone(), audio_name: &name, bookmarks: &bookmarks, random_algorithm: random.algorithm() });
    let k_rows = rows.head_ms.len();
    let held_sum: i32 = result.chart.held[..k_rows].iter().flatten().sum();
    let heads_sum: i32 = result.chart.attack[..k_rows].iter().flatten().sum();
    let ln_heads_sum: i32 = result.chart.ln_head[..k_rows].iter().flatten().sum();
    let held_share = mean_of(result.chart.held[..k_rows].iter().flatten().copied(), k_rows * 4);
    let _ = (held_sum, heads_sum);
    let mut detail = Map::new();
    detail.insert("runtime".into(), json!({"name": crate::RUNTIME_NAME, "version": crate::RUNTIME_VERSION, "random_algorithm": random.algorithm(), "seed32": seed32, "r2_seed32": r2_seed32}));
    detail.insert("audio".into(), request.audio.clone().unwrap_or(Value::Null));
    detail.insert("star".into(), number(request.star));
    detail.insert("band".into(), json!(band));
    detail.insert("seed".into(), if request.seed_is_bigint { json!(seed_text) } else { json!(request.seed) });
    detail.insert("setting".into(), json!(setting));
    detail.insert("metadata".into(), json!({"title": title, "artist": artist}));
    let star_raw: Vec<Value> = sections.iter().filter(|s| s.star.is_some()).map(|s| Value::Object(s.raw.clone())).collect();
    detail.insert(
        "rows".into(),
        json!({
            "request": number(rows.request),
            "sections": star_raw,
            "K": k_rows,
            "dropped_close_heads": rows.dropped,
            "segments": corrected.iter().map(|s| json!([number(s.offset_ms), number(s.beat_length_ms), 4])).collect::<Vec<_>>(),
        }),
    );
    let feature_names = ["log_rows_s", "log_gap_ms", "log_wh_s", "log_bpm"];
    let feature_object: Map<String, Value> = feature_names.iter().zip(prior_features).map(|(k, v)| (k.to_string(), number(v))).collect();
    detail.insert("prior".into(), json!({"method": models.prior.method, "features": feature_object, "predicted": prediction_json(&prediction)}));
    let mode = if ln_fraction == 0.0 { "tap" } else if ln_fraction == 0.5 { "half" } else { "natural" };
    let mut difficulty = Map::new();
    difficulty.insert("q".into(), number(default_q));
    difficulty.insert("target_r".into(), number(target_r));
    difficulty.insert("band".into(), json!([number(quantile(values, 0.05)), number(quantile(values, 0.95))]));
    difficulty.insert("map_band".into(), json!(band));
    difficulty.insert("base_r".into(), number(base_r));
    difficulty.insert("base_r_source".into(), json!("prior"));
    difficulty.insert("fit".into(), fit.clone());
    difficulty.insert("m_requested".into(), number(raw));
    difficulty.insert("domain".into(), json!([number(lo), number(hi)]));
    difficulty.insert("clamped".into(), json!(raw < lo || raw > hi));
    difficulty.insert("m_final".into(), number(multiplier));
    difficulty.insert("achieved_r".into(), number(result.r));
    difficulty.insert("error".into(), number(result.r - target_r));
    difficulty.insert("within_seed_sd".into(), json!((result.r - target_r).abs() <= seed_sd));
    difficulty.insert("passes".into(), Value::Array(passes));
    difficulty.insert("tested_band_q".into(), json!([0.1, 0.75]));
    difficulty.insert("above_tested_band".into(), json!(default_q > 0.75));
    difficulty.insert("default".into(), json!(true));
    detail.insert(
        "controls".into(),
        json!({
            "ln": {
                "mode": mode,
                "fraction": number(ln_fraction),
                "natural_held_share": number(prediction.held),
                "target_held_share": number(prediction.held * ln_fraction),
                "achieved_held_share": number(held_share),
                "ln_heads": ln_heads_sum,
            },
            "difficulty": difficulty,
            "sections": result.requests,
        }),
    );
    detail.insert("targets".into(), json!(kappa_targets(&prediction, ln_fraction, multiplier).map(number)));
    detail.insert(
        "achieved".into(),
        json!({
            "held_share": number(held_share),
            "heads_per_row": number(mean_of(result.chart.attack[..k_rows].iter().flatten().copied(), k_rows)),
            "busy_share": number(result.chart.busy[k_rows] as f64 / k_rows as f64),
            "r": number(result.r),
        }),
    );
    detail.insert("fallback_count".into(), json!(result.fallback_count));
    detail.insert("fallbacks".into(), Value::Array(result.fallbacks.clone()));
    detail.insert("mask_firings_total".into(), json!({"reattack": result.mask_firings[0], "quad": result.mask_firings[1], "jack": result.mask_firings[2]}));
    detail.insert(
        "export".into(),
        json!({
            "legal": true,
            "head_rows_equal": true,
            "replay_finalized": result.chart.held[result.chart.held.len() - 1].iter().all(|&x| x == 0),
            "objects": chart.objects.len(),
            "bookmarks_ms": bookmarks.iter().map(|&b| number(b)).collect::<Vec<_>>(),
        }),
    );
    seconds.push(("export", seconds_since(&*clock, start)));
    seconds.push(("total", seconds_since(&*clock, all_start)));
    detail.insert("seconds".into(), Value::Object(seconds.iter().map(|(k, v)| (k.to_string(), number(*v))).collect()));
    let receipt_json = stringify(&Value::Object(detail));
    let receipt = Receipt { model_name: MODEL_NAME.to_string(), model_version: MODEL_VERSION.to_string(), seed: request.seed, seed_is_bigint: request.seed_is_bigint, star: request.star, receipt_json };
    let _ = number_string;
    Ok(GenerationResult { chart, receipt, osu })
}

fn prediction_json(p: &Prediction) -> Value {
    json!({"held": number(p.held), "heads": number(p.heads), "busy_free": number(p.busy_free), "quad": number(p.quad), "log_r": number(p.log_r)})
}
