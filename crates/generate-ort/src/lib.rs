//! Runs the Ensomi chart generator's graphs with ONNX Runtime and drives its state machine.
use ensomi_generate::{Data, Generator, Models, Phase, Step, Tensor, Tensors, BEATTHIS_GRAPH};
use ort::session::Session;
use ort::tensor::TensorElementType;
use ort::value::{DynValue, Tensor as OrtTensor};
use std::collections::BTreeMap;
use std::path::Path;
use std::time::Instant;

pub mod package;
pub mod recording;

fn ort_error(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// How ONNX Runtime runs the graphs.
#[derive(Clone, Debug)]
pub struct Config {
    /// Graphs that the Core ML execution provider runs, in front of the CPU provider (Apple platforms,
    /// the `coreml` feature). Empty runs every graph on the CPU.
    pub coreml_graphs: Vec<String>,
    /// Core ML `MLComputeUnits`: `ALL`, `CPUAndGPU`, `CPUAndNeuralEngine` or `CPUOnly`.
    pub coreml_units: String,
    /// Core ML `ModelFormat`: `MLProgram` or `NeuralNetwork`.
    pub coreml_format: String,
    /// Intra-op threads of each session but those in `graph_threads`; 0 lets ONNX Runtime choose. Every session has
    /// one inter-op thread.
    pub threads: usize,
    /// Intra-op threads of the named graphs' sessions, in place of `threads`. The native clients give BeatThis
    /// ([`BEATTHIS_GRAPH`]) the device's performance cores ([`performance_cores`]).
    pub graph_threads: BTreeMap<String, usize>,
    /// Whether ONNX Runtime keeps memory for the next run (the CPU arena, and memory patterns outside
    /// Android). Off lowers the peak footprint.
    pub reuse_memory: bool,
}

impl Default for Config {
    /// CPU only, one intra-op and one inter-op thread for every graph, as the parity recordings were checked.
    fn default() -> Self {
        Config { coreml_graphs: Vec::new(), coreml_units: "ALL".to_string(), coreml_format: "MLProgram".to_string(), threads: 1, graph_threads: BTreeMap::new(), reuse_memory: true }
    }
}

impl Config {
    /// [`Config::default`] but BeatThis's session on `threads` intra-op threads. The native clients take
    /// [`performance_cores`].
    pub fn with_beatthis_threads(threads: usize) -> Config {
        Config { graph_threads: BTreeMap::from([(BEATTHIS_GRAPH.to_string(), threads)]), ..Config::default() }
    }

    /// The intra-op threads of `graph`'s session.
    pub fn intra_threads(&self, graph: &str) -> usize {
        self.graph_threads.get(graph).copied().unwrap_or(self.threads)
    }
}

/// The device's performance cores, BeatThis's intra-op threads in the native clients: on Apple platforms
/// `hw.perflevel0.physicalcpu`, the cores of the fastest performance level. Elsewhere, or where the system does not
/// report it, the available parallelism up to 4: that count includes efficiency cores and SMT siblings, and on the Mac
/// where BeatThis was timed (4 performance and 6 efficiency cores) its 4 performance cores were about as fast as
/// ONNX Runtime's own choice. At least 1.
pub fn performance_cores() -> usize {
    perflevel0_cores().unwrap_or_else(|| std::thread::available_parallelism().map_or(1, |n| n.get().min(4)))
}

#[cfg(target_vendor = "apple")]
fn perflevel0_cores() -> Option<usize> {
    let mut cores: libc::c_int = 0;
    let mut size = std::mem::size_of::<libc::c_int>();
    // SAFETY: `cores` and `size` outlive the call and `size` is the byte size of `cores`; no new value is set.
    let status = unsafe { libc::sysctlbyname(c"hw.perflevel0.physicalcpu".as_ptr(), (&raw mut cores).cast(), &mut size, std::ptr::null_mut(), 0) };
    (status == 0 && cores > 0).then_some(cores as usize)
}

#[cfg(not(target_vendor = "apple"))]
fn perflevel0_cores() -> Option<usize> {
    None
}

/// The six graphs of the release as ONNX Runtime sessions.
pub struct Sessions {
    sessions: Vec<(String, Session)>,
}

impl Sessions {
    /// Loads every graph named in the manifest from `dir`.
    pub fn load(dir: &Path, models: &Models, config: &Config) -> Result<Sessions, String> {
        let mut sessions = Vec::new();
        for graph in &models.graphs {
            #[allow(unused_mut)]
            let mut builder = Session::builder().map_err(ort_error)?.with_intra_threads(config.intra_threads(&graph.name)).map_err(ort_error)?.with_inter_threads(1).map_err(ort_error)?;
            if config.coreml_graphs.contains(&graph.name) {
                #[cfg(feature = "coreml")]
                {
                    use ort::ep::ArbitrarilyConfigurableExecutionProvider;
                    let coreml = ort::ep::CoreML::default().with_arbitrary_config("MLComputeUnits", &config.coreml_units).with_arbitrary_config("ModelFormat", &config.coreml_format);
                    builder = builder.with_execution_providers([coreml.build().error_on_failure()]).map_err(ort_error)?;
                }
                #[cfg(not(feature = "coreml"))]
                return Err("built without the coreml feature".to_string());
            }
            if !config.reuse_memory {
                builder = builder.with_execution_providers([ort::ep::CPU::default().with_arena_allocator(false).build()]).map_err(ort_error)?;
            }
            // On Android, the arena alone uses less peak memory than the arena plus pattern buffers.
            // Keep the arena's reuse between calls unless the client requests the lowest footprint.
            if !config.reuse_memory || cfg!(target_os = "android") {
                builder = builder.with_memory_pattern(false).map_err(ort_error)?;
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

/// Runs the generator to the end with `sessions`, timing the core and the graph runs. Committed pages are
/// left out; the result holds the whole chart.
pub fn drive<T>(generator: &mut Generator<T>, sessions: &mut Sessions) -> Result<(T, Timings), String> {
    drive_observed(generator, sessions, |_| Ok(()))
}

/// [`drive`], calling `observe` with each phase the generator enters and with `None` before each graph
/// run. An error from `observe` stops the generation with that error.
pub fn drive_observed<T>(generator: &mut Generator<T>, sessions: &mut Sessions, mut observe: impl FnMut(Option<Phase>) -> Result<(), String>) -> Result<(T, Timings), String> {
    let mut timings = Timings::default();
    let mut phase = 0usize;
    let mut outputs: Option<Tensors> = None;
    loop {
        let start = Instant::now();
        let step = generator.step(outputs.take())?;
        timings.phases[phase].0 += start.elapsed().as_secs_f64();
        match step {
            Step::Run { graph, inputs } => {
                observe(None)?;
                let start = Instant::now();
                outputs = Some(sessions.run(&graph, inputs)?);
                let seconds = start.elapsed().as_secs_f64();
                timings.phases[phase].1 += seconds;
                let entry = timings.graphs.entry(graph).or_default();
                entry.0 += 1;
                entry.1 += seconds;
            }
            Step::Progress(p) => {
                observe(Some(p))?;
                phase = p as usize;
            }
            Step::Commit(_) => {}
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

#[cfg(test)]
mod tests {
    use super::*;
    use ensomi_generate::GRAPHS;

    #[test]
    fn graph_threads_change_only_the_graphs_they_name() {
        let config = Config::with_beatthis_threads(4);
        assert_eq!(config.intra_threads(BEATTHIS_GRAPH), 4);
        for graph in GRAPHS.into_iter().filter(|&g| g != BEATTHIS_GRAPH) {
            assert_eq!(config.intra_threads(graph), 1, "{graph}");
        }
        assert!(GRAPHS.iter().all(|graph| Config::default().intra_threads(graph) == 1));
    }

    #[test]
    fn a_device_has_at_least_one_performance_core() {
        let cores = performance_cores();
        assert!(cores >= 1);
        if cfg!(not(target_vendor = "apple")) {
            assert!(cores <= 4);
        }
    }
}
