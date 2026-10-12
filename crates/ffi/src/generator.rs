//! The chart generator: a model directory opened into ONNX Runtime sessions, and generation from mono
//! PCM with progress and cancellation. Graphs run here, so the foreign side makes one call.
//!
//! The request and result types follow EnsomiProtocol's `chart.proto`; the protocol's `UNSPECIFIED`
//! values become `None`.

use ensomi_gameplay::{Chart, ChartObject};
use ensomi_generate::capabilities::capabilities as core_capabilities;
use ensomi_generate::{generate, resampler_files, Audio, Models, Options, Phase, Request, Samples};
use ensomi_generate_ort::{drive_observed, package, Config, Sessions};
use serde_json::{json, Value};
use std::any::Any;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// Where ONNX Runtime runs the graphs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum ExecutionProvider {
    /// ONNX Runtime's CPU provider.
    Cpu,
    /// Core ML for BeatThis, the CPU for the other graphs. Apple platforms only.
    CoreMl,
}

#[derive(Clone, Debug, uniffi::Record)]
pub struct GeneratorOptions {
    pub provider: ExecutionProvider,
    /// Directory holding the runtime's `resample-<rate>*` tables, read for PCM at a rate other than
    /// 22050 Hz. The Swift package's `ChartGenerator.open` passes the tables it carries when this is unset.
    #[uniffi(default)]
    pub resampler_directory: Option<String>,
    /// Releases ONNX Runtime's working memory after each graph run (no CPU arena, no memory patterns): a
    /// lower peak footprint for a slower generation. `None` turns it on for iOS and off elsewhere.
    #[uniffi(default)]
    pub low_memory: Option<bool>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum ControlStatus {
    /// Checked against human judgement.
    Works,
    /// Accepted and measured, but not checked against human judgement.
    Experimental,
    /// Accepted but known not to do what it says; not to be offered.
    NotWorking,
}

#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct ControlCapability {
    pub name: String,
    pub status: ControlStatus,
    pub note: String,
}

#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct ModelFile {
    /// Relative to the model directory.
    pub path: String,
    /// Empty when the directory has no `ensomi-model.json`.
    pub sha256_hex: String,
    pub size_bytes: Option<u64>,
}

/// EnsomiProtocol's `ModelCapabilities`.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct ModelCapabilities {
    pub model_name: String,
    pub model_version: String,
    pub files: Vec<ModelFile>,
    /// Accepted star range: `min_star <= star < max_star_exclusive`.
    pub min_star: f64,
    pub max_star_exclusive: f64,
    pub controls: Vec<ControlCapability>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum LongNoteAmount {
    TapOnly,
    Half,
    Natural,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum PatternKind {
    Stream,
    Jumpstream,
    Chordstream,
    Jack,
    Chordjack,
}

#[derive(Clone, Debug, PartialEq, uniffi::Enum)]
pub enum SectionTarget {
    /// Star rating for this span alone.
    Star { star: f64 },
    /// Note pattern for this span; `strength` from 0 to 1.
    Pattern { kind: PatternKind, strength: f64 },
}

/// A request for one span of the song, in audio milliseconds (R2: 8 to 32 seconds).
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct SectionRequest {
    pub start_ms: u32,
    pub end_ms: u32,
    pub target: SectionTarget,
}

#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct ChartControls {
    /// `None` leaves the amount of holds to the model.
    #[uniffi(default)]
    pub long_notes: Option<LongNoteAmount>,
    /// Spans with their own request; they must not overlap.
    #[uniffi(default)]
    pub sections: Vec<SectionRequest>,
}

/// EnsomiProtocol's `GenerateChartRequest` without the audio reference: the PCM is passed beside it.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct GenerateChartRequest {
    /// Star rating of the whole chart, within the capabilities' range.
    pub star: f64,
    /// The same model, PCM, star, controls and seed give the same chart.
    #[uniffi(default = 0)]
    pub seed: u64,
    #[uniffi(default)]
    pub controls: ChartControls,
    /// Written to the `.osu`; "Ensomi" and "Unknown" when absent.
    #[uniffi(default)]
    pub title: Option<String>,
    #[uniffi(default)]
    pub artist: Option<String>,
    /// The song's file name, whose extension names the `.osu` audio file (`audio.<extension>`);
    /// `audio.wav` when absent.
    #[uniffi(default)]
    pub audio_name: Option<String>,
}

/// EnsomiProtocol's `GenerationPhase`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum GenerationPhase {
    Loading,
    Timing,
    Rows,
    Arrangement,
    Export,
}

/// EnsomiProtocol's `GenerationReceipt`.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct GenerationReceipt {
    pub model_name: String,
    pub model_version: String,
    pub seed: u64,
    pub star: f64,
    /// The runtime's complete receipt, including realised settings and timings.
    pub receipt_json: String,
}

#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct ChartResult {
    pub chart: Chart,
    /// The chart as an osu!mania 4K beatmap.
    pub osu: String,
    pub receipt: GenerationReceipt,
}

/// The `.osz` of a generated chart: `generated.osu`, the song's audio file under the name the `.osu` gives it, and
/// `receipt.json` from the receipt's JSON; see `ensomi_osz::osz`. Bytes in, bytes out: the caller reads the song and
/// saves the archive.
#[uniffi::export]
pub fn export_osz(osu: String, audio: Vec<u8>, receipt_json: String) -> Vec<u8> {
    ensomi_osz::osz(&osu, &audio, &receipt_json)
}

#[derive(Debug, uniffi::Error)]
pub enum GeneratorError {
    /// A file of the model directory or a resampler table is missing or unreadable, or the directory holds
    /// a model this runtime does not run.
    Model { message: String },
    /// The generation was refused or failed. `code` is EnsomiProtocol's `ErrorEvent.code`:
    /// `star_out_of_range`, `unsupported_control` or `generation_failed`.
    Generation { code: String, message: String },
    /// Stopped through its `Cancellation`.
    Cancelled,
}

impl std::fmt::Display for GeneratorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GeneratorError::Model { message } => write!(f, "model: {message}"),
            GeneratorError::Generation { code, message } => write!(f, "{code}: {message}"),
            GeneratorError::Cancelled => write!(f, "cancelled"),
        }
    }
}

impl std::error::Error for GeneratorError {}

fn model_error(message: String) -> GeneratorError {
    GeneratorError::Model { message }
}

/// The core's errors start with the protocol code when they have one.
fn generation_error(message: String) -> GeneratorError {
    let code = ["star_out_of_range", "unsupported_control"].into_iter().find(|code| message.starts_with(&format!("{code}:"))).unwrap_or("generation_failed");
    GeneratorError::Generation { code: code.to_string(), message }
}

/// Told of each phase a generation enters, on the generating thread.
#[uniffi::export(with_foreign)]
pub trait GenerationListener: Send + Sync {
    fn progress(&self, phase: GenerationPhase);
}

/// Stops a generation from another thread. The generation checks it before every graph run and phase.
#[derive(Debug, Default, uniffi::Object)]
pub struct Cancellation(AtomicBool);

#[uniffi::export]
impl Cancellation {
    #[uniffi::constructor]
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

/// A model directory opened into ONNX Runtime sessions. One generation runs at a time; a second call
/// waits for the first.
#[derive(uniffi::Object)]
pub struct ChartGenerator {
    capabilities: ModelCapabilities,
    resampler_directory: Option<PathBuf>,
    models: Arc<Models>,
    sessions: Mutex<Sessions>,
}

fn session_config(options: &GeneratorOptions) -> Config {
    let reuse_memory = !options.low_memory.unwrap_or(cfg!(target_os = "ios"));
    match options.provider {
        ExecutionProvider::Cpu => Config { reuse_memory, ..Config::default() },
        ExecutionProvider::CoreMl => Config { coreml_graphs: vec!["beatthis".to_string()], reuse_memory, ..Config::default() },
    }
}

/// The files a model directory needs besides its runtime config (`runtime.json`): the graphs and the
/// arrays the config names, as paths relative to the directory.
#[uniffi::export]
pub fn model_files(runtime_config: String) -> Result<Vec<String>, GeneratorError> {
    let config: Value = serde_json::from_str(&runtime_config).map_err(|e| model_error(format!("runtime config: {e}")))?;
    let graphs = config["graphs"].as_object().ok_or_else(|| model_error("runtime config: no graphs".to_string()))?;
    let mut files: Vec<String> = graphs.values().filter_map(|g| g["file"].as_str().map(str::to_string)).collect();
    files.extend(Models::required_files(&config));
    Ok(files)
}

fn capabilities(models: &Models, manifest: Option<&package::Manifest>) -> ModelCapabilities {
    let paths: Vec<String> = models.graphs.iter().map(|g| g.file.clone()).collect();
    let core = core_capabilities(&paths);
    let text = |v: &Value| v.as_str().unwrap_or_default().to_string();
    let files = match manifest {
        Some(m) => m.files.iter().map(|f| ModelFile { path: f.path.clone(), sha256_hex: f.sha256_hex.clone(), size_bytes: Some(f.size_bytes) }).collect(),
        None => paths.into_iter().map(|path| ModelFile { path, sha256_hex: String::new(), size_bytes: None }).collect(),
    };
    let controls = core["controls"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|c| ControlCapability {
            name: text(&c["name"]),
            status: match c["status"].as_u64() {
                Some(1) => ControlStatus::Works,
                Some(2) => ControlStatus::Experimental,
                _ => ControlStatus::NotWorking,
            },
            note: text(&c["note"]),
        })
        .collect();
    ModelCapabilities {
        model_name: text(&core["model_name"]),
        model_version: text(&core["model_version"]),
        files,
        min_star: core["min_star"].as_f64().unwrap_or_default(),
        max_star_exclusive: core["max_star_exclusive"].as_f64().unwrap_or_default(),
        controls,
    }
}

/// The request as the core reads it: `chart.proto`'s JSON field names and enum numbers.
fn core_request(request: &GenerateChartRequest) -> Result<Request, String> {
    let sections: Vec<Value> = request
        .controls
        .sections
        .iter()
        .map(|s| {
            let mut section = json!({"start_ms": s.start_ms, "end_ms": s.end_ms});
            match &s.target {
                SectionTarget::Star { star } => section["star"] = json!(star),
                SectionTarget::Pattern { kind, strength } => section["pattern"] = json!({"kind": *kind as u32 + 1, "strength": strength}),
            }
            section
        })
        .collect();
    let long_notes = request.controls.long_notes.map_or(0, |amount| amount as u32 + 1);
    let json = json!({"star": request.star, "controls": {"long_notes": long_notes, "sections": sections}, "title": request.title, "artist": request.artist});
    // The JavaScript API takes seeds above 2^53 - 1 as BigInt, which its receipt writes as a string.
    Request::from_json(&json.to_string(), request.seed, request.seed > (1 << 53) - 1)
}

fn read_resampler(directory: Option<&Path>, sample_rate: f64) -> Result<Option<[Vec<u8>; 3]>, GeneratorError> {
    match (directory, resampler_files(sample_rate)) {
        (Some(directory), Some(names)) => {
            let read = |name: &String| std::fs::read(directory.join(name)).map_err(|e| model_error(format!("{name}: {e}")));
            Ok(Some([read(&names[0])?, read(&names[1])?, read(&names[2])?]))
        }
        _ => Ok(None),
    }
}

fn panic_message(payload: &(dyn Any + Send)) -> &str {
    payload.downcast_ref::<&str>().copied().or_else(|| payload.downcast_ref::<String>().map(String::as_str)).unwrap_or("no message")
}

fn chart(chart: &ensomi_generate::Chart) -> Chart {
    Chart {
        duration_ms: chart.duration_ms as u32,
        objects: chart.objects.iter().map(|o| ChartObject { lane: o.lane as u32, start_ms: o.start_ms as u32, end_ms: o.end_ms.map(|end| end as u32) }).collect(),
    }
}

#[uniffi::export]
impl ChartGenerator {
    /// Opens a model directory: `runtime.json` and the files it names, or, when the directory has an
    /// `ensomi-model.json`, the runtime config that manifest names after checking the runtime range,
    /// the model and the size of every file it lists.
    #[uniffi::constructor]
    pub fn open(model_directory: String, options: GeneratorOptions) -> Result<Arc<Self>, GeneratorError> {
        let directory = Path::new(&model_directory);
        let (models, manifest) = package::open(directory).map_err(model_error)?;
        let sessions = Sessions::load(directory, &models, &session_config(&options)).map_err(model_error)?;
        Ok(Arc::new(Self {
            capabilities: capabilities(&models, manifest.as_ref()),
            resampler_directory: options.resampler_directory.map(PathBuf::from),
            models: Arc::new(models),
            sessions: Mutex::new(sessions),
        }))
    }

    pub fn capabilities(&self) -> ModelCapabilities {
        self.capabilities.clone()
    }

    /// Generates a chart from mono PCM at `sample_rate`. Blocks until the chart is done; `listener` hears each
    /// phase and `cancellation` stops it at its next step. A panic in the core or the listener ends the
    /// generation with `GeneratorError::Generation` and leaves the generator usable.
    pub fn generate(
        &self,
        pcm: Vec<f32>,
        sample_rate: f64,
        request: GenerateChartRequest,
        listener: Option<Arc<dyn GenerationListener>>,
        cancellation: Option<Arc<Cancellation>>,
    ) -> Result<ChartResult, GeneratorError> {
        let resampler = read_resampler(self.resampler_directory.as_deref(), sample_rate)?;
        let audio = Audio { samples: Samples::F32(pcm), channels: 1, sample_rate, mel: None, beatthis: None, resampler };
        let audio_name = request.audio_name.clone();
        let request = core_request(&request).map_err(generation_error)?;
        let start = Instant::now();
        let options = Options { random: None, setting: None, audio_name, trace: None, clock: Rc::new(move || start.elapsed().as_secs_f64() * 1000.0) };
        let cancelled = || cancellation.as_ref().is_some_and(|c| c.is_cancelled());

        // A panic is caught inside the lock, which therefore stays usable; ONNX Runtime sessions keep no state
        // between runs.
        let mut sessions = crate::lock(&self.sessions);
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            let mut generator = generate(self.models.clone(), audio, request, options);
            drive_observed(&mut generator, &mut sessions, |phase| {
                if cancelled() {
                    return Err("cancelled".to_string());
                }
                if let (Some(phase), Some(listener)) = (phase, &listener) {
                    listener.progress(match phase {
                        Phase::Loading => GenerationPhase::Loading,
                        Phase::Timing => GenerationPhase::Timing,
                        Phase::Rows => GenerationPhase::Rows,
                        Phase::Arrangement => GenerationPhase::Arrangement,
                        Phase::Export => GenerationPhase::Export,
                    });
                }
                Ok(())
            })
        }))
        .map_err(|panic| GeneratorError::Generation { code: "generation_failed".to_string(), message: format!("internal error: {}", panic_message(&*panic)) })?;
        let (result, _) = outcome.map_err(|e| if cancelled() { GeneratorError::Cancelled } else { generation_error(e) })?;
        Ok(ChartResult {
            chart: chart(&result.chart),
            osu: result.osu,
            receipt: GenerationReceipt {
                model_name: result.receipt.model_name,
                model_version: result.receipt.model_version,
                seed: result.receipt.seed,
                star: result.receipt.star,
                receipt_json: result.receipt.receipt_json,
            },
        })
    }
}
