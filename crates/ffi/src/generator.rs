//! The chart generator: a model directory opened into ONNX Runtime sessions, and generation from mono
//! PCM with progress and cancellation. Graphs run here, so the foreign side makes one call. `prepare` runs the
//! stages that read only the audio, into a `Prepared` that a `ChartScheduler` generates from.
//!
//! The request and result types follow EnsomiProtocol's `chart.proto`; the protocol's `UNSPECIFIED`
//! values become `None`.

use ensomi_gameplay::{Chart, ChartObject};
use ensomi_generate::capabilities::capabilities as core_capabilities;
use ensomi_generate::package::{self as core_package, PackageFile, PackageInfo, PackageModel};
use ensomi_generate::{generate, prepare, resampler_files, Audio, Models, Options, Phase, Request, Samples, BEATTHIS_GRAPH};
use ensomi_generate_ort::{drive_observed, package, performance_cores, Config, Sessions};
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
    /// ONNX Runtime's intra-op threads for BeatThis; the other graphs run on one. `None` takes the device's
    /// performance cores (`hw.perflevel0.physicalcpu` on Apple platforms), 0 lets ONNX Runtime choose.
    #[uniffi(default)]
    pub beatthis_threads: Option<u32>,
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
    /// Empty when the directory has no `ensomi-package.json`.
    pub sha256_hex: String,
    pub size_bytes: Option<u64>,
}

#[uniffi::remote(Record)]
pub struct PackageModel {
    pub id: String,
    pub name: String,
    pub version: String,
}

/// One file of a model package, as its `ensomi-package.json` lists it.
#[uniffi::remote(Record)]
pub struct PackageFile {
    /// Relative to the manifest, `/`-separated.
    pub path: String,
    pub size: u64,
    /// Lowercase hex SHA-256 of the file.
    pub sha256: String,
}

/// A checked `ensomi-package.json` (`docs/model-package.md`).
#[uniffi::remote(Record)]
pub struct PackageInfo {
    pub model: PackageModel,
    /// An SPDX licence expression.
    pub license: String,
    /// The licence notices among `files`, for the app to show with the model.
    pub notices: Vec<String>,
    /// Every file of the package but the manifest, sorted by path: what a downloader fetches and checks.
    pub files: Vec<PackageFile>,
    /// The sum of the files' sizes.
    pub total_bytes: u64,
    /// The runtime config (`runtime.json`), one of `files`.
    pub config: String,
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
    /// The same model, PCM, star, controls and seed give the same chart on one device with the same options.
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
    /// A file of the model directory or a resampler table is missing or unreadable, the directory holds
    /// a model this runtime does not run, or a package manifest (`ensomi-package.json`) is refused.
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

pub(crate) fn model_error(message: String) -> GeneratorError {
    GeneratorError::Model { message }
}

/// The core's errors start with the protocol code when they have one.
pub(crate) fn generation_error(message: String) -> GeneratorError {
    let code = ["star_out_of_range", "unsupported_control"].into_iter().find(|code| message.starts_with(&format!("{code}:"))).unwrap_or("generation_failed");
    GeneratorError::Generation { code: code.to_string(), message }
}

/// Mono f32 PCM at the byte-oriented binding boundary.
pub(crate) fn pcm_f32le(pcm: Vec<u8>) -> Result<Vec<f32>, GeneratorError> {
    let (samples, remainder) = pcm.as_chunks::<4>();
    if !remainder.is_empty() {
        return Err(generation_error("PCM byte length must be a multiple of 4".to_string()));
    }
    Ok(samples.iter().map(|b| f32::from_le_bytes(*b)).collect())
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
/// waits for the first. A `ChartScheduler` takes the sessions for one graph run at a time.
#[derive(uniffi::Object)]
pub struct ChartGenerator {
    capabilities: ModelCapabilities,
    resampler_directory: Option<PathBuf>,
    pub(crate) models: Arc<Models>,
    pub(crate) sessions: Mutex<Sessions>,
}

/// What `prepare` keeps of a song: the results of the stages that read only the audio, from which any request and
/// seed generate. `bytes()` is the runtime's stored form, for a cache keyed by the audio file's hash; `init(bytes:)`
/// reads it back.
#[derive(uniffi::Object)]
pub struct Prepared(pub(crate) Arc<ensomi_generate::Prepared>);

#[uniffi::export]
impl Prepared {
    /// Fails with `GeneratorError.Model` for bytes of another format, runtime version or model.
    #[uniffi::constructor]
    pub fn new(bytes: Vec<u8>) -> Result<Arc<Self>, GeneratorError> {
        Ok(Arc::new(Self(Arc::new(ensomi_generate::Prepared::from_bytes(&bytes).map_err(model_error)?))))
    }

    pub fn bytes(&self) -> Vec<u8> {
        self.0.to_bytes()
    }
}

fn session_config(options: &GeneratorOptions) -> Config {
    let reuse_memory = !options.low_memory.unwrap_or(cfg!(target_os = "ios"));
    let config = Config::with_beatthis_threads(options.beatthis_threads.map_or_else(performance_cores, |threads| threads as usize));
    match options.provider {
        ExecutionProvider::Cpu => Config { reuse_memory, ..config },
        ExecutionProvider::CoreMl => Config { coreml_graphs: vec![BEATTHIS_GRAPH.to_string()], reuse_memory, ..config },
    }
}

fn parse_config(runtime_config: &str) -> Result<Value, GeneratorError> {
    serde_json::from_str(runtime_config).map_err(|e| model_error(format!("runtime config: {e}")))
}

/// Checks a model package's `ensomi-package.json`: its format, that this runtime is at least its `min_runtime`, that it
/// names the model and pipeline this runtime runs, its paths, and its notices and config among its files. Throws
/// `GeneratorError.Model` naming what is wrong. A downloader fetches exactly `files` and checks each one's size and
/// SHA-256 before `open`.
#[uniffi::export]
pub fn check_package(manifest_json: String) -> Result<PackageInfo, GeneratorError> {
    core_package::check(&manifest_json).map_err(model_error)
}

/// The files a model directory without `ensomi-package.json` needs besides its runtime config (`runtime.json`): the
/// graphs, the arrays the config names and its licence notices, as paths relative to the directory.
#[uniffi::export]
pub fn model_files(runtime_config: String) -> Result<Vec<String>, GeneratorError> {
    let config = parse_config(&runtime_config)?;
    let graphs = config["graphs"].as_object().ok_or_else(|| model_error("runtime config: no graphs".to_string()))?;
    let mut files: Vec<String> = graphs.values().filter_map(|g| g["file"].as_str().map(str::to_string)).collect();
    files.extend(Models::required_files(&config));
    files.extend(Models::notice_files(&config));
    Ok(files)
}

/// The licence notices among the model files, as paths relative to the directory, for an app to show with
/// the model: BeatThis's MIT licence. `open` does not read them, so a directory without them still opens.
#[uniffi::export]
pub fn notice_files(runtime_config: String) -> Result<Vec<String>, GeneratorError> {
    Ok(Models::notice_files(&parse_config(&runtime_config)?))
}

fn capabilities(models: &Models, package: Option<&PackageInfo>) -> ModelCapabilities {
    let paths: Vec<String> = models.graphs.iter().map(|g| g.file.clone()).collect();
    let core = core_capabilities(&paths);
    let text = |v: &Value| v.as_str().unwrap_or_default().to_string();
    let files = match package {
        Some(p) => p.files.iter().map(|f| ModelFile { path: f.path.clone(), sha256_hex: f.sha256.clone(), size_bytes: Some(f.size) }).collect(),
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
pub(crate) fn core_request(request: &GenerateChartRequest) -> Result<Request, String> {
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

/// A panic in the core, the listener or the session, as the generation's error.
pub(crate) fn panic_error(payload: Box<dyn Any + Send>) -> GeneratorError {
    GeneratorError::Generation { code: "generation_failed".to_string(), message: format!("internal error: {}", panic_message(&*payload)) }
}

/// Milliseconds since now, the core's clock.
pub(crate) fn clock() -> Rc<dyn Fn() -> f64> {
    let start = Instant::now();
    Rc::new(move || start.elapsed().as_secs_f64() * 1000.0)
}

fn chart(chart: &ensomi_generate::Chart) -> Chart {
    Chart {
        duration_ms: chart.duration_ms as u32,
        objects: chart.objects.iter().map(|o| ChartObject { lane: o.lane as u32, start_ms: o.start_ms as u32, end_ms: o.end_ms.map(|end| end as u32) }).collect(),
    }
}

#[uniffi::export]
impl ChartGenerator {
    /// Opens a model directory. When it has an `ensomi-package.json`, checks it (`check_package`), requires every
    /// file it lists at its listed size and reads the runtime config it names; the SHA-256s are checked when the
    /// package arrives, not here. Without the manifest, reads `runtime.json`. Then the files the config names.
    #[uniffi::constructor]
    pub fn open(model_directory: String, options: GeneratorOptions) -> Result<Arc<Self>, GeneratorError> {
        let directory = Path::new(&model_directory);
        let (models, package) = package::open(directory).map_err(model_error)?;
        let sessions = Sessions::load(directory, &models, &session_config(&options)).map_err(model_error)?;
        Ok(Arc::new(Self {
            capabilities: capabilities(&models, package.as_ref()),
            resampler_directory: options.resampler_directory.map(PathBuf::from),
            models: Arc::new(models),
            sessions: Mutex::new(sessions),
        }))
    }

    pub fn capabilities(&self) -> ModelCapabilities {
        self.capabilities.clone()
    }

    /// Runs the stages that read only the audio of mono PCM at `sample_rate`: PCM preparation, the spectrogram,
    /// BeatThis, the features and the beat grid. Blocks until they are done; `cancellation` stops them at the next
    /// step, and a cancelled `prepare` returns `GeneratorError::Cancelled` and keeps nothing.
    pub fn prepare(&self, pcm: Vec<f32>, sample_rate: f64, cancellation: Option<Arc<Cancellation>>) -> Result<Arc<Prepared>, GeneratorError> {
        let audio = self.audio(pcm, sample_rate)?;
        let cancelled = || cancellation.as_ref().is_some_and(|c| c.is_cancelled());
        let mut sessions = crate::lock(&self.sessions);
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            let mut generator = prepare(self.models.clone(), audio, Options::new(clock()));
            drive_observed(&mut generator, &mut sessions, |_| if cancelled() { Err("cancelled".to_string()) } else { Ok(()) })
        }))
        .map_err(panic_error)?;
        let (prepared, _) = outcome.map_err(|e| if cancelled() { GeneratorError::Cancelled } else { generation_error(e) })?;
        Ok(Arc::new(Prepared(Arc::new(prepared))))
    }

    /// `prepare` with mono PCM encoded as little-endian f32 bytes. A partial sample returns
    /// `GeneratorError::Generation`.
    pub fn prepare_f32le(&self, pcm: Vec<u8>, sample_rate: f64, cancellation: Option<Arc<Cancellation>>) -> Result<Arc<Prepared>, GeneratorError> {
        self.prepare(pcm_f32le(pcm)?, sample_rate, cancellation)
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
        let audio = self.audio(pcm, sample_rate)?;
        let options = Options { audio_name: request.audio_name.clone(), ..Options::new(clock()) };
        let request = core_request(&request).map_err(generation_error)?;
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
        .map_err(panic_error)?;
        let (result, _) = outcome.map_err(|e| if cancelled() { GeneratorError::Cancelled } else { generation_error(e) })?;
        Ok(chart_result(&result))
    }

    /// `generate` with mono PCM encoded as little-endian f32 bytes. Progress, cancellation and errors
    /// follow `generate`; a partial sample returns `GeneratorError::Generation`.
    pub fn generate_f32le(
        &self,
        pcm: Vec<u8>,
        sample_rate: f64,
        request: GenerateChartRequest,
        listener: Option<Arc<dyn GenerationListener>>,
        cancellation: Option<Arc<Cancellation>>,
    ) -> Result<ChartResult, GeneratorError> {
        self.generate(pcm_f32le(pcm)?, sample_rate, request, listener, cancellation)
    }
}

impl ChartGenerator {
    /// Mono PCM as the core takes it, with the resampler tables for its rate.
    pub(crate) fn audio(&self, pcm: Vec<f32>, sample_rate: f64) -> Result<Audio, GeneratorError> {
        let resampler = read_resampler(self.resampler_directory.as_deref(), sample_rate)?;
        Ok(Audio { samples: Samples::F32(pcm), channels: 1, sample_rate, mel: None, beatthis: None, resampler })
    }
}

pub(crate) fn chart_result(result: &ensomi_generate::GenerationResult) -> ChartResult {
    ChartResult {
        chart: chart(&result.chart),
        osu: result.osu.clone(),
        receipt: GenerationReceipt {
            model_name: result.receipt.model_name.clone(),
            model_version: result.receipt.model_version.clone(),
            seed: result.receipt.seed,
            star: result.receipt.star,
            receipt_json: result.receipt.receipt_json.clone(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ensomi_generate::GRAPHS;

    #[test]
    fn byte_pcm_decodes_to_the_same_f32_samples() {
        let pcm = [0.0_f32, -0.0, 1.0, -0.5, f32::MIN_POSITIVE, f32::INFINITY, f32::from_bits(0x7fc01234)];
        let bytes = pcm.iter().flat_map(|sample| sample.to_le_bytes()).collect();
        let decoded = pcm_f32le(bytes).unwrap();
        assert_eq!(decoded.iter().map(|sample| sample.to_bits()).collect::<Vec<_>>(), pcm.iter().map(|sample| sample.to_bits()).collect::<Vec<_>>());
        assert_eq!(pcm_f32le(vec![0x00, 0x00, 0x80, 0x3f, 0x00, 0x00, 0x00, 0xbf]).unwrap(), vec![1.0, -0.5]);
        assert!(pcm_f32le(Vec::new()).unwrap().is_empty());
    }

    #[test]
    fn byte_pcm_rejects_a_partial_sample() {
        for length in [1, 2, 3, 5, 6, 7] {
            assert!(matches!(pcm_f32le(vec![0; length]), Err(GeneratorError::Generation { code, message })
                if code == "generation_failed" && message == "PCM byte length must be a multiple of 4"));
        }
    }

    fn options(provider: ExecutionProvider, beatthis_threads: Option<u32>) -> GeneratorOptions {
        GeneratorOptions { provider, resampler_directory: None, low_memory: None, beatthis_threads }
    }

    #[test]
    fn beatthis_runs_on_the_performance_cores_and_the_other_graphs_on_one_thread() {
        for provider in [ExecutionProvider::Cpu, ExecutionProvider::CoreMl] {
            for (requested, expected) in [(None, performance_cores()), (Some(1), 1), (Some(3), 3), (Some(0), 0)] {
                let config = session_config(&options(provider, requested));
                assert_eq!(config.intra_threads(BEATTHIS_GRAPH), expected, "{provider:?} {requested:?}");
                for graph in GRAPHS.into_iter().filter(|&g| g != BEATTHIS_GRAPH) {
                    assert_eq!(config.intra_threads(graph), 1, "{provider:?} {requested:?} {graph}");
                }
            }
        }
    }
}
