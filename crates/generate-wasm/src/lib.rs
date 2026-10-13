//! WebAssembly binding for `ensomi-generate`.
//!
//! The generator is a state machine: `WasmGenerator.step(outputs)` returns the next step as a plain
//! object, `{kind: 'run', graph, inputs}`, `{kind: 'progress', phase}` or `{kind: 'done', ...}`, and
//! the JavaScript caller runs the named graph with ONNX Runtime Web and passes the outputs back.
//! While a graph runs, `work()` does what needs no outputs. `WasmPrepare` runs the audio stages alone into
//! the stored bytes of a `Prepared`; `WasmScheduler` drives generation for play (`next()` returns `run`,
//! `commit`, `ready`, `wait` or `done`).
//! Tensors cross as `{type, dims, data}` with the typed array ONNX Runtime Web uses for the type.
//!
//! A panic is written to `console.error` and thrown as a `WebAssembly.RuntimeError` with the panic's message,
//! like the traps of the instance (an exhausted memory or stack). After either, the instance's memory may be
//! inconsistent, so the JavaScript API replaces the instance (`js/src/core.js`).
use ensomi_gameplay::{Judge, Ruleset};
use ensomi_generate::{capabilities, generate, package, prepare, Action, Audio, Data, Draw, Event, GenerationResult, Generator, Lead, Mode, Models, Options, Page, Prepared, ReplayRandom, Request, Samples, Scheduler, Source, Step, Tensor, Tensors, TokenKind};
use js_sys::{Array, BigInt64Array, Float32Array, Float64Array, Object, Reflect, Uint8Array};
use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = console, js_name = error)]
    fn console_error(message: &str);
}

fn report_panic(info: &std::panic::PanicHookInfo) {
    let message = info.to_string();
    console_error(&message);
    wasm_bindgen::throw_val(js_sys::WebAssembly::RuntimeError::new(&message).into());
}

#[wasm_bindgen(start)]
fn start() {
    std::panic::set_hook(Box::new(report_panic));
}

fn error(message: impl AsRef<str>) -> JsError {
    JsError::new(message.as_ref())
}

fn set(object: &Object, key: &str, value: impl Into<JsValue>) {
    Reflect::set(object, &JsValue::from_str(key), &value.into()).unwrap();
}

#[wasm_bindgen]
pub struct WasmModels {
    inner: Arc<Models>,
}

#[wasm_bindgen]
impl WasmModels {
    /// The files, other than graphs, that `new WasmModels` needs for this `runtime.json`.
    #[wasm_bindgen(js_name = requiredFiles)]
    pub fn required_files(manifest_json: &str) -> Result<Array, JsError> {
        let manifest: serde_json::Value = serde_json::from_str(manifest_json).map_err(|e| error(e.to_string()))?;
        Ok(Models::required_files(&manifest).iter().map(|p| JsValue::from_str(p)).collect())
    }

    /// The licence notices this `runtime.json` names, which `new WasmModels` does not need.
    #[wasm_bindgen(js_name = noticeFiles)]
    pub fn notice_files(manifest_json: &str) -> Result<Array, JsError> {
        let manifest: serde_json::Value = serde_json::from_str(manifest_json).map_err(|e| error(e.to_string()))?;
        Ok(Models::notice_files(&manifest).iter().map(|p| JsValue::from_str(p)).collect())
    }

    /// The three `resample-*` files for a PCM sample rate the release bundles tables for, or null.
    #[wasm_bindgen(js_name = resamplerFiles)]
    pub fn resampler_files(sample_rate: f64) -> JsValue {
        match ensomi_generate::resampler_files(sample_rate) {
            Some(files) => files.iter().map(|p| JsValue::from_str(p)).collect::<Array>().into(),
            None => JsValue::NULL,
        }
    }

    /// `files` maps each path to its bytes (`Uint8Array`).
    #[wasm_bindgen(constructor)]
    pub fn new(manifest_json: &str, files: &Object) -> Result<WasmModels, JsError> {
        let manifest = serde_json::from_str(manifest_json).map_err(|e| error(e.to_string()))?;
        let mut map = HashMap::new();
        for entry in Object::entries(files).iter() {
            let pair: Array = entry.into();
            map.insert(pair.get(0).as_string().unwrap_or_default(), Uint8Array::new(&pair.get(1)).to_vec());
        }
        Ok(WasmModels { inner: Arc::new(Models::new(manifest, map).map_err(error)?) })
    }

    /// EnsomiProtocol `ModelCapabilities` as JSON.
    pub fn capabilities(&self) -> String {
        let paths: Vec<String> = self.inner.graphs.iter().map(|g| g.file.clone()).collect();
        capabilities::capabilities(&paths).to_string()
    }
}

/// Recorded random draws, to replay a release recording. Passing it to a generator moves it there.
#[wasm_bindgen]
pub struct ReplayDraws {
    draws: Vec<Draw>,
}

#[wasm_bindgen]
impl ReplayDraws {
    #[wasm_bindgen(constructor)]
    pub fn new() -> ReplayDraws {
        ReplayDraws { draws: Vec::new() }
    }

    pub fn add(&mut self, name: &str, kind: &str, values: &Float64Array, chosen: u32) {
        self.draws.push(Draw { name: name.to_string(), kind: kind.to_string(), values: values.to_vec(), chosen: chosen as usize });
    }

    pub fn recorded(&self) -> usize {
        self.draws.len()
    }
}

impl Default for ReplayDraws {
    fn default() -> Self {
        Self::new()
    }
}

#[wasm_bindgen]
pub struct WasmGenerator {
    inner: Generator,
    replay_position: Option<Rc<Cell<usize>>>,
}

fn tensor_to_js(tensor: &Tensor) -> JsValue {
    let object = Object::new();
    let (kind, data): (&str, JsValue) = match &tensor.data {
        Data::F32(v) => ("float32", Float32Array::from(v.as_slice()).into()),
        Data::I64(v) => {
            let array = BigInt64Array::new_with_length(v.len() as u32);
            array.copy_from(v);
            ("int64", array.into())
        }
        Data::Bool(v) => ("bool", Uint8Array::from(v.as_slice()).into()),
    };
    set(&object, "type", kind);
    set(&object, "dims", tensor.shape.iter().map(|&d| JsValue::from_f64(d as f64)).collect::<Array>());
    set(&object, "data", data);
    object.into()
}

fn tensor_from_js(value: &JsValue) -> Result<Tensor, JsError> {
    let kind = Reflect::get(value, &"type".into()).ok().and_then(|t| t.as_string()).unwrap_or_default();
    let dims: Vec<usize> = Array::from(&Reflect::get(value, &"dims".into()).map_err(|_| error("tensor without dims"))?).iter().map(|d| d.as_f64().unwrap_or(0.0) as usize).collect();
    let data = Reflect::get(value, &"data".into()).map_err(|_| error("tensor without data"))?;
    match kind.as_str() {
        "float32" => Ok(Tensor::f32(&dims, Float32Array::new(&data).to_vec())),
        "int64" => Ok(Tensor::i64(&dims, BigInt64Array::new(&data).to_vec())),
        "bool" => Ok(Tensor::bool(&dims, Uint8Array::new(&data).to_vec())),
        other => Err(error(format!("Unsupported graph output type: {other}"))),
    }
}

/// Interleaved PCM as a `Float32Array` or `Float64Array`, with the optional frontend streams and resampler tables.
fn audio(samples: &JsValue, channels: usize, sample_rate: f64, mel: Option<Float32Array>, beatthis: Option<Float32Array>, resampler: Option<Array>) -> Result<Audio, JsError> {
    let samples = if samples.is_instance_of::<Float64Array>() {
        Samples::F64(Float64Array::from(samples.clone()).to_vec())
    } else if samples.is_instance_of::<Float32Array>() {
        Samples::F32(Float32Array::from(samples.clone()).to_vec())
    } else {
        return Err(error("PCM must be a Float32Array or Float64Array"));
    };
    let resampler = resampler.map(|files| std::array::from_fn(|i| Uint8Array::new(&files.get(i as u32)).to_vec()));
    Ok(Audio { samples, channels, sample_rate, mel: mel.map(|a| a.to_vec()), beatthis: beatthis.map(|a| a.to_vec()), resampler })
}

fn clock_from_js(clock: js_sys::Function) -> Rc<dyn Fn() -> f64> {
    Rc::new(move || clock.call0(&JsValue::NULL).ok().and_then(|v| v.as_f64()).unwrap_or(f64::NAN))
}

/// The options of a generation, with the recorded draws to replay and where to read how many were used.
fn options(setting: Option<String>, audio_name: Option<String>, clock: js_sys::Function, replay: Option<ReplayDraws>) -> (Options, Option<Rc<Cell<usize>>>) {
    let source = replay.map(|r| ReplayRandom::new(r.draws));
    let replay_position = source.as_ref().map(|s| s.position());
    let random = source.map(|s| Box::new(s) as Box<dyn ensomi_generate::RandomSource>);
    (Options { random, setting, audio_name, ..Options::new(clock_from_js(clock)) }, replay_position)
}

/// Graph outputs as an object of `{type, dims, data}` by output name, or nothing.
fn outputs_from_js(outputs: JsValue) -> Result<Option<Tensors>, JsError> {
    if outputs.is_undefined() || outputs.is_null() {
        return Ok(None);
    }
    let mut tensors = Vec::new();
    for entry in Object::entries(&Object::from(outputs)).iter() {
        let pair: Array = entry.into();
        tensors.push((pair.get(0).as_string().unwrap_or_default(), tensor_from_js(&pair.get(1))?));
    }
    Ok(Some(tensors))
}

fn run_to_js(object: &Object, graph: String, inputs: &Tensors) {
    set(object, "kind", "run");
    set(object, "graph", graph);
    let feeds = Object::new();
    for (name, tensor) in inputs {
        set(&feeds, name, tensor_to_js(tensor));
    }
    set(object, "inputs", feeds);
}

fn done_to_js(object: &Object, result: &GenerationResult, replay_position: &Option<Rc<Cell<usize>>>) {
    set(object, "kind", "done");
    let objects = Array::new();
    for o in &result.chart.objects {
        let entry = Object::new();
        set(&entry, "lane", o.lane as u32);
        set(&entry, "start_ms", o.start_ms);
        if let Some(end) = o.end_ms {
            set(&entry, "end_ms", end);
        }
        objects.push(&entry);
    }
    let chart = Object::new();
    set(&chart, "duration_ms", result.chart.duration_ms);
    set(&chart, "objects", objects);
    set(object, "chart", chart);
    set(object, "osu", result.osu.clone());
    let receipt = Object::new();
    set(&receipt, "model_name", result.receipt.model_name.clone());
    set(&receipt, "model_version", result.receipt.model_version.clone());
    set(&receipt, "star", result.receipt.star);
    set(&receipt, "receipt_json", result.receipt.receipt_json.clone());
    set(object, "receipt", receipt);
    if let Some(position) = replay_position {
        set(object, "replay_consumed", position.get() as u32);
    }
}

/// `{tokens: [{lane, time_ms, kind}], coverage_ms, end_of_stream}`, `kind` one of `tap`, `hold_start`, `hold_end`.
fn page_to_js(page: &Page) -> JsValue {
    let tokens = Array::new();
    for t in &page.tokens {
        let token = Object::new();
        set(&token, "lane", t.lane as u32);
        set(&token, "time_ms", t.time_ms);
        set(&token, "kind", match t.kind {
            TokenKind::Tap => "tap",
            TokenKind::HoldStart => "hold_start",
            TokenKind::HoldEnd => "hold_end",
        });
        tokens.push(&token);
    }
    let object = Object::new();
    set(&object, "tokens", tokens);
    set(&object, "coverage_ms", page.coverage_ms);
    set(&object, "end_of_stream", page.end_of_stream);
    object.into()
}

#[wasm_bindgen]
impl WasmGenerator {
    /// `samples` is a `Float32Array` or `Float64Array` of interleaved PCM. `resampler` is the array
    /// of the three `resamplerFiles` for `sample_rate`, when there is no `beatthis` stream. `request_json`
    /// is the request without its seed, which is passed as a BigInt. `clock` returns milliseconds.
    #[allow(clippy::too_many_arguments)]
    #[wasm_bindgen(constructor)]
    pub fn new(
        models: &WasmModels,
        samples: &JsValue,
        channels: usize,
        sample_rate: f64,
        mel: Option<Float32Array>,
        beatthis: Option<Float32Array>,
        resampler: Option<Array>,
        request_json: &str,
        seed: u64,
        seed_is_bigint: bool,
        setting: Option<String>,
        audio_name: Option<String>,
        clock: js_sys::Function,
        replay: Option<ReplayDraws>,
    ) -> Result<WasmGenerator, JsError> {
        let audio = audio(samples, channels, sample_rate, mel, beatthis, resampler)?;
        let request = Request::from_json(request_json, seed, seed_is_bigint).map_err(error)?;
        let (options, replay_position) = options(setting, audio_name, clock, replay);
        Ok(WasmGenerator { inner: generate(models.inner.clone(), audio, request, options), replay_position })
    }

    /// The next step. Pass the outputs of the graph the previous step asked for as an object of
    /// `{type, dims, data}` by output name, and `undefined` otherwise.
    pub fn step(&mut self, outputs: JsValue) -> Result<JsValue, JsError> {
        let step = self.inner.step(outputs_from_js(outputs)?).map_err(error)?;
        let object = Object::new();
        match step {
            Step::Run { graph, inputs } => run_to_js(&object, graph, &inputs),
            Step::Progress(phase) => {
                set(&object, "kind", "progress");
                set(&object, "phase", phase as u32);
            }
            Step::Commit(_) => unreachable!("a whole-chart generation commits no pages"),
            Step::Done(result) => done_to_js(&object, &result, &self.replay_position),
        }
        Ok(object.into())
    }

    /// Does a slice of the work that needs no graph outputs (the Log-Mel spectrogram while BeatThis
    /// runs) and returns whether any is left. Call it after a `run` step while the graph runs.
    pub fn work(&mut self) -> bool {
        self.inner.work()
    }
}

/// Checks an `ensomi-package.json` (`ensomi_generate::package::check`) and returns what it describes:
/// `{model: {id, name, version}, license, notices, files: [{path, size, sha256}], totalBytes, config}`. Throws naming
/// what is wrong.
#[wasm_bindgen(js_name = checkPackage)]
pub fn check_package(manifest_json: &str) -> Result<JsValue, JsError> {
    let info = package::check(manifest_json).map_err(error)?;
    let model = Object::new();
    set(&model, "id", info.model.id);
    set(&model, "name", info.model.name);
    set(&model, "version", info.model.version);
    let files = Array::new();
    for file in info.files {
        let entry = Object::new();
        set(&entry, "path", file.path);
        set(&entry, "size", file.size as f64);
        set(&entry, "sha256", file.sha256);
        files.push(&entry);
    }
    let object = Object::new();
    set(&object, "model", model);
    set(&object, "license", info.license);
    set(&object, "notices", info.notices.iter().map(|p| JsValue::from_str(p)).collect::<Array>());
    set(&object, "files", files);
    set(&object, "totalBytes", info.total_bytes as f64);
    set(&object, "config", info.config);
    Ok(object.into())
}

/// Checks that an `ensomi-package.json` lists every file the runtime config it names needs
/// (`ensomi_generate::package::check_config`); throws naming the first it leaves out.
#[wasm_bindgen(js_name = checkPackageConfig)]
pub fn check_package_config(manifest_json: &str, config_json: &str) -> Result<(), JsError> {
    let config = serde_json::from_str(config_json).map_err(|e| error(e.to_string()))?;
    package::check_config(&package::check(manifest_json).map_err(error)?, &config).map_err(error)
}

/// Checks the stored bytes of a `Prepared`; throws for bytes of another format, runtime version or model.
#[wasm_bindgen(js_name = checkPrepared)]
pub fn check_prepared(bytes: &[u8]) -> Result<(), JsError> {
    Prepared::from_bytes(bytes).map(drop).map_err(error)
}

/// The audio stages alone: `step(outputs)` returns `{kind: 'run', ...}`, `{kind: 'progress', phase}` or
/// `{kind: 'done', bytes}`, the stored form of the song's `Prepared`.
#[wasm_bindgen]
pub struct WasmPrepare {
    inner: Generator<Prepared>,
}

#[wasm_bindgen]
impl WasmPrepare {
    /// PCM as for `WasmGenerator`.
    #[allow(clippy::too_many_arguments)]
    #[wasm_bindgen(constructor)]
    pub fn new(models: &WasmModels, samples: &JsValue, channels: usize, sample_rate: f64, mel: Option<Float32Array>, beatthis: Option<Float32Array>, resampler: Option<Array>, clock: js_sys::Function) -> Result<WasmPrepare, JsError> {
        let audio = audio(samples, channels, sample_rate, mel, beatthis, resampler)?;
        Ok(WasmPrepare { inner: prepare(models.inner.clone(), audio, Options::new(clock_from_js(clock))) })
    }

    pub fn step(&mut self, outputs: JsValue) -> Result<JsValue, JsError> {
        let step = self.inner.step(outputs_from_js(outputs)?).map_err(error)?;
        let object = Object::new();
        match step {
            Step::Run { graph, inputs } => run_to_js(&object, graph, &inputs),
            Step::Progress(phase) => {
                set(&object, "kind", "progress");
                set(&object, "phase", phase as u32);
            }
            Step::Commit(_) => unreachable!("the audio stages commit no pages"),
            Step::Done(prepared) => {
                set(&object, "kind", "done");
                set(&object, "bytes", Uint8Array::from(prepared.to_bytes().as_slice()));
            }
        }
        Ok(object.into())
    }

    pub fn work(&mut self) -> bool {
        self.inner.work()
    }
}

/// Generation for play (`ensomi_generate::Scheduler`). `next()` returns `{kind: 'run', graph, inputs}`,
/// `{kind: 'commit', page}`, `{kind: 'ready', join_ms}`, `{kind: 'wait'}` or `{kind: 'done', ...}`, with the chart, `.osu`
/// and receipt once generation has finished.
#[wasm_bindgen]
pub struct WasmScheduler {
    inner: Scheduler,
    replay_position: Option<Rc<Cell<usize>>>,
}

#[wasm_bindgen]
impl WasmScheduler {
    /// From the stored bytes of a `Prepared` when `prepared` is given, else from PCM as for `WasmGenerator`. `mode`
    /// is `whole-chart` or `ahead-of-play`; `judge` (`A` to `E`) names the rules whose settle lead the lead takes.
    #[allow(clippy::too_many_arguments)]
    #[wasm_bindgen(constructor)]
    pub fn new(
        models: &WasmModels,
        prepared: Option<Vec<u8>>,
        samples: &JsValue,
        channels: usize,
        sample_rate: f64,
        mel: Option<Float32Array>,
        beatthis: Option<Float32Array>,
        resampler: Option<Array>,
        request_json: &str,
        seed: u64,
        seed_is_bigint: bool,
        setting: Option<String>,
        audio_name: Option<String>,
        clock: js_sys::Function,
        replay: Option<ReplayDraws>,
        mode: &str,
        judge: &str,
        render_lead_ms: f64,
        margin_ms: f64,
    ) -> Result<WasmScheduler, JsError> {
        let source = match prepared {
            Some(bytes) => Source::Prepared(Arc::new(Prepared::from_bytes(&bytes).map_err(error)?)),
            None => Source::Audio(audio(samples, channels, sample_rate, mel, beatthis, resampler)?),
        };
        let mode = match mode {
            "whole-chart" => Mode::WholeChart,
            "ahead-of-play" => Mode::AheadOfPlay,
            other => return Err(error(format!("Unknown mode {other}"))),
        };
        let judge = match judge {
            "A" => Judge::A,
            "B" => Judge::B,
            "C" => Judge::C,
            "D" => Judge::D,
            "E" => Judge::E,
            other => return Err(error(format!("Unknown judge {other}"))),
        };
        let request = Request::from_json(request_json, seed, seed_is_bigint).map_err(error)?;
        let (options, replay_position) = options(setting, audio_name, clock, replay);
        let lead = Lead { settle_ms: Ruleset { judge }.settle_lead_ms(), render_ms: render_lead_ms, margin_ms };
        Ok(WasmScheduler { inner: Scheduler::new(models.inner.clone(), source, request, options, mode, lead), replay_position })
    }

    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Result<JsValue, JsError> {
        let action = self.inner.next().map_err(error)?;
        let object = Object::new();
        match action {
            Action::Run { graph, inputs } => run_to_js(&object, graph, &inputs),
            Action::Commit(page) => {
                set(&object, "kind", "commit");
                set(&object, "page", page_to_js(&page));
            }
            Action::Ready { join_ms } => {
                set(&object, "kind", "ready");
                set(&object, "join_ms", join_ms);
            }
            Action::Wait => set(&object, "kind", "wait"),
            Action::Done => match self.inner.result() {
                Some(result) => done_to_js(&object, result, &self.replay_position),
                None => set(&object, "kind", "done"),
            },
        }
        Ok(object.into())
    }

    /// The outputs of the graph `next()` asked to run, as for `WasmGenerator.step`.
    pub fn outputs(&mut self, outputs: JsValue) -> Result<(), JsError> {
        let outputs = outputs_from_js(outputs)?.ok_or_else(|| error("no graph outputs"))?;
        self.inner.handle(Event::Outputs(outputs)).map_err(error)
    }

    pub fn heard(&mut self, chart_ms: f64, wall_ms: f64) -> Result<(), JsError> {
        self.inner.handle(Event::Heard { chart_ms, wall_ms }).map_err(error)
    }

    pub fn start(&mut self, at_ms: f64) -> Result<(), JsError> {
        self.inner.handle(Event::Start { at_ms }).map_err(error)
    }

    /// New controls for the chart, read as for `new`: `ensomi_generate::Event::Request`, which says when R2 takes
    /// them; otherwise it throws and the generation is unchanged.
    pub fn request(&mut self, request_json: &str, seed: u64, seed_is_bigint: bool) -> Result<(), JsError> {
        let request = Request::from_json(request_json, seed, seed_is_bigint).map_err(error)?;
        self.inner.handle(Event::Request(request)).map_err(error)
    }

    pub fn stop(&mut self) -> Result<(), JsError> {
        self.inner.handle(Event::Stop).map_err(error)
    }

    /// While a graph runs, does a slice of the work that needs no outputs; see `WasmGenerator.work`.
    pub fn work(&mut self) -> bool {
        self.inner.work()
    }

    /// Committed chart milliseconds per busy wall millisecond so far.
    pub fn rate(&self) -> Option<f64> {
        self.inner.rate()
    }

    /// The stored form of the song's `Prepared`, once the audio stages have run.
    pub fn prepared(&self) -> Option<Uint8Array> {
        self.inner.prepared().map(|p| Uint8Array::from(p.to_bytes().as_slice()))
    }
}
