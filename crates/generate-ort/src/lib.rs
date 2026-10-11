//! Runs the Ensomi chart generator's graphs with ONNX Runtime and drives its state machine.
use ensomi_generate::{Data, GenerationResult, Generator, Models, Phase, Step, Tensor, Tensors};
use ort::session::Session;
use ort::tensor::TensorElementType;
use ort::value::{DynValue, Tensor as OrtTensor};
use std::collections::BTreeMap;
use std::path::Path;
use std::time::Instant;

pub mod recording;

fn ort_error(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// The six graphs of the release as ONNX Runtime sessions.
pub struct Sessions {
    sessions: Vec<(String, Session)>,
}

impl Sessions {
    /// Loads every graph named in the manifest from `dir`. CPU, one intra-op and one inter-op thread,
    /// as the parity recordings were checked; `coreml` adds the Core ML provider in front of the CPU.
    pub fn load(dir: &Path, models: &Models, coreml: bool) -> Result<Sessions, String> {
        let mut sessions = Vec::new();
        for graph in &models.graphs {
            #[allow(unused_mut)]
            let mut builder = Session::builder().map_err(ort_error)?.with_intra_threads(1).map_err(ort_error)?.with_inter_threads(1).map_err(ort_error)?;
            if coreml {
                #[cfg(feature = "coreml")]
                {
                    builder = builder.with_execution_providers([ort::ep::CoreML::default().build()]).map_err(ort_error)?;
                }
                #[cfg(not(feature = "coreml"))]
                return Err("built without the coreml feature".to_string());
            }
            let session = builder.commit_from_file(dir.join(&graph.file)).map_err(ort_error)?;
            sessions.push((graph.name.clone(), session));
        }
        Ok(Sessions { sessions })
    }

    pub fn run(&mut self, graph: &str, inputs: Tensors) -> Result<Tensors, String> {
        let session = &mut self.sessions.iter_mut().find(|(name, _)| name == graph).ok_or_else(|| format!("No session for graph {graph}"))?.1;
        let feeds = inputs.into_iter().map(|(name, tensor)| Ok((name, to_ort(tensor)?))).collect::<Result<Vec<(String, DynValue)>, String>>()?;
        let outputs = session.run(feeds).map_err(ort_error)?;
        let mut out = Vec::new();
        for (name, value) in &outputs {
            let element = value.dtype().tensor_type().ok_or_else(|| format!("graph output {name} is not a tensor"))?;
            let tensor = match element {
                TensorElementType::Float32 => {
                    let (shape, data) = value.try_extract_tensor::<f32>().map_err(ort_error)?;
                    Tensor { shape: shape.iter().map(|&d| d as usize).collect(), data: Data::F32(data.to_vec()) }
                }
                TensorElementType::Int64 => {
                    let (shape, data) = value.try_extract_tensor::<i64>().map_err(ort_error)?;
                    Tensor { shape: shape.iter().map(|&d| d as usize).collect(), data: Data::I64(data.to_vec()) }
                }
                TensorElementType::Bool => {
                    let (shape, data) = value.try_extract_tensor::<bool>().map_err(ort_error)?;
                    Tensor { shape: shape.iter().map(|&d| d as usize).collect(), data: Data::Bool(data.iter().map(|&b| b as u8).collect()) }
                }
                other => return Err(format!("graph output {name} has unsupported element type {other:?}")),
            };
            out.push((name.to_string(), tensor));
        }
        Ok(out)
    }
}

fn to_ort(tensor: Tensor) -> Result<DynValue, String> {
    let shape: Vec<i64> = tensor.shape.iter().map(|&d| d as i64).collect();
    Ok(match tensor.data {
        Data::F32(v) => OrtTensor::from_array((shape, v)).map_err(ort_error)?.into_dyn(),
        Data::I64(v) => OrtTensor::from_array((shape, v)).map_err(ort_error)?.into_dyn(),
        Data::Bool(v) => OrtTensor::from_array((shape, v.into_iter().map(|b| b != 0).collect::<Vec<bool>>())).map_err(ort_error)?.into_dyn(),
    })
}

/// Seconds spent in the core and in the graphs, by phase, and in each graph.
#[derive(Default)]
pub struct Timings {
    /// Indexed by [`Phase`] (1-based, 0 is before the first phase): core seconds, graph seconds.
    pub phases: [(f64, f64); 6],
    pub graphs: BTreeMap<String, (u32, f64)>,
}

/// Runs the generator to the end with `sessions`, timing the core and the graph runs.
pub fn drive(generator: &mut Generator, sessions: &mut Sessions) -> Result<(GenerationResult, Timings), String> {
    let mut timings = Timings::default();
    let mut phase = 0usize;
    let mut outputs: Option<Tensors> = None;
    loop {
        let start = Instant::now();
        let step = generator.step(outputs.take())?;
        timings.phases[phase].0 += start.elapsed().as_secs_f64();
        match step {
            Step::Run { graph, inputs } => {
                let start = Instant::now();
                outputs = Some(sessions.run(&graph, inputs)?);
                let seconds = start.elapsed().as_secs_f64();
                timings.phases[phase].1 += seconds;
                let entry = timings.graphs.entry(graph).or_default();
                entry.0 += 1;
                entry.1 += seconds;
            }
            Step::Progress(p) => phase = p as usize,
            Step::Done(result) => return Ok((*result, timings)),
        }
    }
}

pub fn phase_name(index: usize) -> &'static str {
    match index {
        0 => "validate",
        i if i == Phase::Loading as usize => "loading",
        i if i == Phase::Timing as usize => "timing",
        i if i == Phase::Rows as usize => "rows",
        i if i == Phase::Arrangement as usize => "arrangement",
        _ => "export",
    }
}
