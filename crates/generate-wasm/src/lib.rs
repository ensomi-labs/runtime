//! WebAssembly binding for `ensomi-generate`.
//!
//! The generator is a state machine: `WasmGenerator.step(outputs)` returns the next step as a plain
//! object, `{kind: 'run', graph, inputs}`, `{kind: 'progress', phase}` or `{kind: 'done', ...}`, and
//! the JavaScript caller runs the named graph with ONNX Runtime Web and passes the outputs back.
//! While a graph runs, `work()` does what needs no outputs.
//! Tensors cross as `{type, dims, data}` with the typed array ONNX Runtime Web uses for the type.
use ensomi_generate::{capabilities, generate, Audio, Data, Draw, Generator, Models, Options, ReplayRandom, Request, Samples, Step, Tensor};
use js_sys::{Array, BigInt64Array, Float32Array, Float64Array, Object, Reflect, Uint8Array};
use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;
use wasm_bindgen::prelude::*;

fn error(message: impl AsRef<str>) -> JsError {
    JsError::new(message.as_ref())
}

fn set(object: &Object, key: &str, value: impl Into<JsValue>) {
    Reflect::set(object, &JsValue::from_str(key), &value.into()).unwrap();
}

#[wasm_bindgen]
pub struct WasmModels {
    inner: Rc<Models>,
}

#[wasm_bindgen]
impl WasmModels {
    /// The files, other than graphs, that `new WasmModels` needs for this `runtime.json`.
    #[wasm_bindgen(js_name = requiredFiles)]
    pub fn required_files(manifest_json: &str) -> Result<Array, JsError> {
        let manifest: serde_json::Value = serde_json::from_str(manifest_json).map_err(|e| error(e.to_string()))?;
        Ok(Models::required_files(&manifest).iter().map(|p| JsValue::from_str(p)).collect())
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
        Ok(WasmModels { inner: Rc::new(Models::new(manifest, map).map_err(error)?) })
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
        let samples = if samples.is_instance_of::<Float64Array>() {
            Samples::F64(Float64Array::from(samples.clone()).to_vec())
        } else if samples.is_instance_of::<Float32Array>() {
            Samples::F32(Float32Array::from(samples.clone()).to_vec())
        } else {
            return Err(error("PCM must be a Float32Array or Float64Array"));
        };
        let resampler = resampler.map(|files| std::array::from_fn(|i| Uint8Array::new(&files.get(i as u32)).to_vec()));
        let audio = Audio { samples, channels, sample_rate, mel: mel.map(|a| a.to_vec()), beatthis: beatthis.map(|a| a.to_vec()), resampler };
        let request = Request::from_json(request_json, seed, seed_is_bigint).map_err(error)?;
        let source = replay.map(|r| ReplayRandom::new(r.draws));
        let replay_position = source.as_ref().map(|s| s.position());
        let random = source.map(|s| Box::new(s) as Box<dyn ensomi_generate::RandomSource>);
        let options = Options { random, setting, audio_name, trace: None, clock: Rc::new(move || clock.call0(&JsValue::NULL).ok().and_then(|v| v.as_f64()).unwrap_or(f64::NAN)) };
        Ok(WasmGenerator { inner: generate(models.inner.clone(), audio, request, options), replay_position })
    }

    /// The next step. Pass the outputs of the graph the previous step asked for as an object of
    /// `{type, dims, data}` by output name, and `undefined` otherwise.
    pub fn step(&mut self, outputs: JsValue) -> Result<JsValue, JsError> {
        let outputs = if outputs.is_undefined() || outputs.is_null() {
            None
        } else {
            let mut tensors = Vec::new();
            for entry in Object::entries(&Object::from(outputs)).iter() {
                let pair: Array = entry.into();
                tensors.push((pair.get(0).as_string().unwrap_or_default(), tensor_from_js(&pair.get(1))?));
            }
            Some(tensors)
        };
        let step = self.inner.step(outputs).map_err(error)?;
        let object = Object::new();
        match step {
            Step::Run { graph, inputs } => {
                set(&object, "kind", "run");
                set(&object, "graph", graph);
                let feeds = Object::new();
                for (name, tensor) in &inputs {
                    set(&feeds, name, tensor_to_js(tensor));
                }
                set(&object, "inputs", feeds);
            }
            Step::Progress(phase) => {
                set(&object, "kind", "progress");
                set(&object, "phase", phase as u32);
            }
            Step::Done(result) => {
                set(&object, "kind", "done");
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
                set(&object, "chart", chart);
                set(&object, "osu", result.osu);
                let receipt = Object::new();
                set(&receipt, "model_name", result.receipt.model_name);
                set(&receipt, "model_version", result.receipt.model_version);
                set(&receipt, "star", result.receipt.star);
                set(&receipt, "receipt_json", result.receipt.receipt_json);
                set(&object, "receipt", receipt);
                if let Some(position) = &self.replay_position {
                    set(&object, "replay_consumed", position.get() as u32);
                }
            }
        }
        Ok(object.into())
    }

    /// Does a slice of the work that needs no graph outputs (the Log-Mel spectrogram while BeatThis
    /// runs) and returns whether any is left. Call it after a `run` step while the graph runs.
    pub fn work(&mut self) -> bool {
        self.inner.work()
    }
}
