//! The parsed `runtime.json` and the files it names that the host code reads.
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;

/// The `grid_fitter` section of `runtime.json`.
#[derive(Clone, Deserialize)]
pub struct FitterConfig {
    pub min_bpm: f64,
    pub max_bpm: f64,
    pub bpm_step: f64,
    pub offset_step_ms: f64,
    pub offset_refine_step_ms: f64,
    pub offset_refine_radius_ms: f64,
    pub pulse_width_ms: f64,
    pub double_tempo_score_ratio_threshold: f64,
    pub max_segments: f64,
    pub min_segment_duration_ms: f64,
    pub split_score_improvement_threshold: f64,
    pub split_phase_change_threshold_ms: f64,
    pub split_relative_interval_change_threshold: f64,
    pub split_downbeat_signal_weight: f64,
    pub autocorrelation_candidate_count: f64,
    pub bpm_search_window_ratio: f64,
    pub bpm_search_window_min_bpm: f64,
    pub max_grid_candidates_per_segment: f64,
    pub max_split_candidates_per_segment: f64,
    pub initial_batch_split_candidate_count: f64,
    pub initial_batch_split_min_candidate_count: f64,
    pub initial_batch_split_max_parent_score: f64,
    pub long_prediction_duration_seconds: f64,
    pub long_max_segments: f64,
    pub long_max_grid_candidates_per_segment: f64,
    pub long_max_split_candidates_per_segment: f64,
    pub long_downbeat_refine_candidate_count: f64,
    pub downbeat_period_beats: f64,
    pub downbeat_tie_score_margin: f64,
    pub downbeat_split_score_bonus: f64,
    pub downbeat_refine_candidate_count: f64,
    pub merge_bpm_tolerance: f64,
    pub merge_relative_bpm_tolerance: f64,
    pub merge_phase_tolerance_ms: f64,
    pub merge_many_similar_min_segments: f64,
    pub merge_many_similar_bpm_tolerance: f64,
    pub merge_alias_min_segments: f64,
    pub merge_alias_bpm_tolerance: f64,
    pub merge_alias_phase_tolerance_ms: f64,
    pub merge_alias_max_fit_score: f64,
    pub refine_offset_quantum_ms: f64,
    pub refine_dominant_min_segments: f64,
    pub refine_dominant_min_segment_count: f64,
    pub refine_dominant_min_outlier_segments: f64,
    pub refine_dominant_min_far_outlier_segments: f64,
    pub refine_dominant_min_duration_ratio: f64,
    pub refine_dominant_bpm_tolerance: f64,
    pub refine_dominant_far_bpm_tolerance: f64,
    pub refine_dominant_far_relative_bpm_tolerance: f64,
    pub refine_dominant_max_first_anchor_ratio: f64,
    pub refine_dominant_boundary_phase_tolerance_ms: f64,
    pub refine_dominant_min_off_lattice_transitions: f64,
    pub refine_dominant_bpm_snap_tolerance: f64,
    pub refine_circular_phase_min_coherence: f64,
    pub refine_structural_max_score_loss: f64,
    pub alias_tempo_multipliers: Vec<f64>,
    pub alias_score_tie_margin: f64,
    pub alias_score_ratio_threshold: f64,
    pub alias_preferred_min_bpm: f64,
    pub alias_preferred_max_bpm: f64,
    pub alias_preferred_band_bonus: f64,
    pub alias_current_tempo_bonus: f64,
    pub alias_downbeat_score_weight: f64,
    pub alias_continuity_penalty: f64,
    pub alias_semantic_promotion_in_band_min_bpm: f64,
    pub alias_semantic_promotion_current_max_bpm: f64,
    pub alias_semantic_promotion_score_ratio_threshold: f64,
    pub alias_semantic_promotion_low_bpm_max_fit_score: f64,
    pub alias_semantic_promotion_strong_score_ratio_threshold: f64,
    pub alias_semantic_promotion_low_confidence_max_fit_score: f64,
    pub alias_semantic_promotion_low_confidence_score_ratio_threshold: f64,
    pub alias_semantic_promotion_low_confidence_max_candidate_bpm: f64,
    pub alias_semantic_promotion_low_bpm_max_segments: f64,
    pub alias_semantic_promotion_bonus: f64,
    pub alias_collapse_score_ratio_threshold: f64,
    pub alias_demotion_dropped_support_ratio_threshold: f64,
    pub alias_promotion_inserted_support_ratio_threshold: f64,
    pub alias_beat_match_tolerance_ms: f64,
}

#[derive(Clone, Deserialize)]
pub struct ArraySpec {
    pub file: String,
    pub dtype: String,
    pub shape: Vec<usize>,
}

#[derive(Deserialize)]
pub struct RowsConfig {
    pub vocab: usize,
    pub enc: usize,
    pub ctx: usize,
    pub hidden: usize,
    pub heads: usize,
    pub lags: usize,
    pub key: usize,
}

#[derive(Deserialize)]
pub struct StarMap {
    pub star: Vec<f64>,
    pub request: Vec<f64>,
}

#[derive(Deserialize)]
pub struct RowsRecord {
    pub vocab: Vec<u32>,
    pub label_mean: f64,
    pub label_std: f64,
    pub star_map: StarMap,
}

#[derive(Deserialize)]
pub struct Rows {
    pub config: RowsConfig,
    pub record: RowsRecord,
}

#[derive(Deserialize)]
pub struct PriorBand {
    pub median: Vec<f64>,
    pub coef: Vec<Vec<f64>>,
    pub mean: Vec<f64>,
    pub sd: Vec<f64>,
    pub table: Vec<Vec<f64>>,
}

#[derive(Deserialize)]
pub struct Prior {
    pub method: String,
    pub neighbours: Option<f64>,
    pub bands: HashMap<String, PriorBand>,
}

#[derive(Clone, Deserialize)]
pub struct Masks {
    pub reattack_floor_ms: f64,
    pub quad_run_cap: f64,
    pub gap_tercile_edges_ms: Vec<f64>,
    pub jack_run_caps: Vec<f64>,
}

#[derive(Deserialize)]
pub struct ScopeColumn {
    pub name: String,
    pub file: String,
    pub dtype: String,
}

#[derive(Deserialize)]
pub struct ScopeTable {
    pub columns: Vec<ScopeColumn>,
}

#[derive(Deserialize)]
pub struct DifficultyValues {
    pub values: Vec<f64>,
}

/// The per-band fit stays a JSON object: the receipt records it as it is.
#[derive(Deserialize)]
pub struct DifficultyFit {
    pub domain: Vec<f64>,
    pub by_band: HashMap<String, Value>,
}

#[derive(Deserialize)]
pub struct SubtypeBand {
    pub q_sub: f64,
    pub reference_gap_p10_ms: f64,
    pub in_band: bool,
    pub out_of_band_reasons: Value,
}

#[derive(Deserialize)]
pub struct Subtype {
    pub bands: HashMap<String, SubtypeBand>,
}

#[derive(Deserialize)]
pub struct Calibration {
    pub masks: HashMap<String, Masks>,
    pub scope_table: ScopeTable,
    pub difficulty: HashMap<String, DifficultyValues>,
    pub difficulty_fit: DifficultyFit,
    pub subtypes: HashMap<String, Subtype>,
    pub stream_eligibility_gap_ms: f64,
}

#[derive(Deserialize)]
struct Constants {
    #[serde(rename = "audio_rows.figures")]
    figures: Figures,
    #[serde(rename = "r2.common")]
    common: Common,
}

#[derive(Deserialize)]
struct Figures {
    #[serde(rename = "POSITIONS")]
    positions: Vec<f64>,
}

#[derive(Deserialize)]
struct Common {
    #[serde(rename = "ACTIONS")]
    actions: Vec<[i32; 4]>,
}

#[derive(Deserialize)]
struct FrontendFiles {
    mel_filter: PathSpec,
    hann_window: PathSpec,
}

#[derive(Deserialize)]
struct PathSpec {
    path: String,
}

#[derive(Deserialize)]
struct FrontendArrays {
    beatthis: FrontendFiles,
    rows: FrontendFiles,
}

#[derive(Deserialize)]
struct State {
    r2_initial: serde_json::Map<String, Value>,
    r2_commit_outputs: serde_json::Map<String, Value>,
}

#[derive(Deserialize)]
struct System {
    default_q: HashMap<String, f64>,
}

#[derive(Deserialize)]
struct Raw {
    format: String,
    graphs: serde_json::Map<String, Value>,
    state: State,
    frontend_arrays: FrontendArrays,
    grid_fitter: FitterConfig,
    constants: Constants,
    rows: Rows,
    r2: Value,
    calibration: Calibration,
    prior: Prior,
    system: System,
}

pub struct Graph {
    pub name: String,
    pub file: String,
    pub inputs: Vec<String>,
}

pub struct Models {
    pub graphs: Vec<Graph>,
    /// Initial R2 state tensors, by the name the graphs use.
    pub r2_initial: Vec<(String, ArraySpec)>,
    /// R2 commit output name to the state name it replaces.
    pub r2_commit_outputs: Vec<(String, String)>,
    pub beatthis_window: String,
    pub beatthis_filter: String,
    pub rows_window: String,
    pub rows_filter: String,
    pub fitter: FitterConfig,
    pub positions: Vec<f64>,
    pub actions: Vec<[i32; 4]>,
    pub rows: Rows,
    pub calibration: Calibration,
    pub prior: Prior,
    pub default_q: HashMap<String, f64>,
    files: HashMap<String, Vec<u8>>,
}

fn invalid(what: &str, e: impl std::fmt::Display) -> String {
    format!("runtime.json: {what}: {e}")
}

impl Models {
    /// `files` holds every file the manifest names except the ONNX graphs, by manifest path:
    /// the Mel filters and windows, the initial R2 state and the scope table columns
    /// ([`Models::required_files`]).
    pub fn new(manifest: Value, files: HashMap<String, Vec<u8>>) -> Result<Models, String> {
        let raw: Raw = serde_json::from_value(manifest).map_err(|e| invalid("unreadable", e))?;
        if raw.format != "ensomi-onnx-runtime-v1" {
            return Err(format!("Unsupported runtime format: {}", raw.format));
        }
        let config = &raw.r2["config"];
        let off = |key: &str| config[key] == "off";
        if config["memory"] != "none" || config["conditioner"] != "film" || !(off("ln_level") && off("ln_length") && off("anchor") && off("theta")) {
            return Err("This runtime supports the released R2 FiLM model with memory and optional readers off".to_string());
        }
        let graphs = raw
            .graphs
            .iter()
            .map(|(name, graph)| {
                let file = graph["file"].as_str().ok_or_else(|| invalid(name, "no file"))?.to_string();
                let inputs = graph["inputs"].as_array().ok_or_else(|| invalid(name, "no inputs"))?.iter().filter_map(|i| i["name"].as_str().map(str::to_string)).collect();
                Ok(Graph { name: name.clone(), file, inputs })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let r2_initial = raw.state.r2_initial.into_iter().map(|(name, spec)| Ok((name, serde_json::from_value(spec).map_err(|e| invalid("r2_initial", e))?))).collect::<Result<Vec<_>, String>>()?;
        let r2_commit_outputs = raw.state.r2_commit_outputs.into_iter().map(|(out, name)| (out, name.as_str().unwrap_or_default().to_string())).collect();
        let models = Models {
            graphs,
            r2_initial,
            r2_commit_outputs,
            beatthis_window: raw.frontend_arrays.beatthis.hann_window.path,
            beatthis_filter: raw.frontend_arrays.beatthis.mel_filter.path,
            rows_window: raw.frontend_arrays.rows.hann_window.path,
            rows_filter: raw.frontend_arrays.rows.mel_filter.path,
            fitter: raw.grid_fitter,
            positions: raw.constants.figures.positions,
            actions: raw.constants.common.actions,
            rows: raw.rows,
            calibration: raw.calibration,
            prior: raw.prior,
            default_q: raw.system.default_q,
            files,
        };
        models.check()?;
        Ok(models)
    }

    /// Checks once what the pipeline then indexes without checking: the graphs it runs, the R2 state it carries,
    /// the sizes of the arrays it reads, and for every star band a request can reach the band's calibration, prior
    /// and default. Graph outputs are checked against the shapes they should have where they are read.
    fn check(&self) -> Result<(), String> {
        for graph in ["beatthis", "rows_encoder", "rows_step", "r2_predict", "r2_release", "r2_commit"] {
            self.graph(graph)?;
        }
        let state: Vec<&str> = self.r2_initial.iter().map(|(name, _)| name.as_str()).collect();
        if !state.contains(&"last") {
            return Err(invalid("state.r2_initial", "no last"));
        }
        // r2_predict consumes `last` before each commit (`R2Stepper::predict`).
        if let Some(input) = self.graph("r2_commit")?.inputs.iter().find(|i| *i != "history_token" && (*i == "last" || !state.contains(&i.as_str()))) {
            return Err(invalid("graphs.r2_commit", format!("input {input} is not a state tensor that r2_predict leaves")));
        }
        let mut renewed: Vec<&str> = self.r2_commit_outputs.iter().map(|(_, name)| name.as_str()).collect();
        let mut initial = state.clone();
        renewed.sort_unstable();
        initial.sort_unstable();
        if renewed != initial {
            return Err(invalid("state.r2_commit_outputs", "the commit must renew each state tensor once"));
        }
        for (name, spec) in &self.r2_initial {
            let width = match spec.dtype.as_str() {
                "<f4" => 4,
                "<i8" => 8,
                other => return Err(invalid("state.r2_initial", format!("{name} has unsupported dtype {other}"))),
            };
            let (bytes, needed) = (self.file(&spec.file)?.len(), spec.shape.iter().product::<usize>() * width);
            if bytes != needed {
                return Err(format!("{}: {bytes} bytes where state tensor {name} {:?} needs {needed}", spec.file, spec.shape));
            }
        }
        for (window, filter) in [(&self.beatthis_window, &self.beatthis_filter), (&self.rows_window, &self.rows_filter)] {
            let n = self.file(window)?.len() / 4;
            let (values, needed) = (self.file(filter)?.len() / 4, (n / 2 + 1) * 128);
            if n == 0 || values != needed {
                return Err(format!("{filter}: {values} values where 128 Mel bands over the {n}-point window {window} need {needed}"));
            }
        }
        if self.positions.len() != 24 {
            return Err(invalid("constants", format!("{} figure positions where a beat has 24", self.positions.len())));
        }
        if self.actions.is_empty() {
            return Err(invalid("constants", "no R2 actions"));
        }
        let rows = &self.rows;
        if rows.config.vocab == 0 || rows.record.vocab.len() != rows.config.vocab {
            return Err(invalid("rows", format!("{} figures where the config has a vocabulary of {}", rows.record.vocab.len(), rows.config.vocab)));
        }
        let map = &rows.record.star_map;
        if map.star.is_empty() || map.star.len() != map.request.len() {
            return Err(invalid("rows.record.star_map", "star and request need the same nonzero length"));
        }
        if self.calibration.difficulty_fit.domain.len() != 2 {
            return Err(invalid("calibration.difficulty_fit.domain", "needs two values"));
        }
        // `generate` takes the band as the floor of a star in [2, 6), at most 5.
        for band in ["2", "3", "4", "5"] {
            let missing = |what: &str| invalid(what, format!("no band {band}"));
            let masks = self.calibration.masks.get(band).ok_or_else(|| missing("calibration.masks"))?;
            if masks.jack_run_caps.len() <= masks.gap_tercile_edges_ms.len() {
                return Err(invalid("calibration.masks", format!("band {band} needs a jack run cap for each gap tercile")));
            }
            self.calibration.difficulty.get(band).ok_or_else(|| missing("calibration.difficulty"))?;
            self.calibration.difficulty_fit.by_band.get(band).ok_or_else(|| missing("calibration.difficulty_fit"))?;
            self.default_q.get(band).ok_or_else(|| missing("system.default_q"))?;
            for subtype in ["jumpstream", "chordstream", "chordjack"] {
                self.calibration.subtypes.get(subtype).and_then(|s| s.bands.get(band)).ok_or_else(|| missing(&format!("calibration.subtypes.{subtype}")))?;
            }
            let prior = self.prior.bands.get(band).ok_or_else(|| missing("prior"))?;
            let fits = match self.prior.method.as_str() {
                "median" => prior.median.len() >= 5,
                "linear" => prior.coef.len() >= 5 && prior.coef.iter().all(|row| row.len() >= 5),
                _ => prior.mean.len() >= 4 && prior.sd.len() >= 4 && prior.table.iter().all(|row| row.len() >= 9),
            };
            if !fits {
                return Err(invalid("prior", format!("band {band} is too small for method {}", self.prior.method)));
            }
        }
        let mut length = None;
        for name in ["band", "length_s", "median_gap_ms", "S", "J_ex", "jump_share", "hand_share", "chord_share"] {
            let column = self.calibration.scope_table.columns.iter().find(|c| c.name == name).ok_or_else(|| invalid("calibration.scope_table", format!("no column {name}")))?;
            if !matches!(column.dtype.as_str(), "<i8" | "<f8") {
                return Err(invalid("calibration.scope_table", format!("column {name} has unsupported dtype {}", column.dtype)));
            }
            let values = self.file(&column.file)?.len() / 8;
            let expected = *length.get_or_insert(values);
            if values != expected {
                return Err(invalid("calibration.scope_table", format!("column {name} has {values} rows, not {expected}")));
            }
        }
        Ok(())
    }

    /// The paths, other than graphs, that `Models::new` needs in `files`.
    pub fn required_files(manifest: &Value) -> Vec<String> {
        let mut paths = Vec::new();
        for side in ["beatthis", "rows"] {
            for array in ["hann_window", "mel_filter"] {
                if let Some(path) = manifest["frontend_arrays"][side][array]["path"].as_str() {
                    paths.push(path.to_string());
                }
            }
        }
        if let Some(initial) = manifest["state"]["r2_initial"].as_object() {
            paths.extend(initial.values().filter_map(|spec| spec["file"].as_str().map(str::to_string)));
        }
        if let Some(columns) = manifest["calibration"]["scope_table"]["columns"].as_array() {
            paths.extend(columns.iter().filter_map(|c| c["file"].as_str().map(str::to_string)));
        }
        paths
    }

    pub fn file(&self, path: &str) -> Result<&[u8], String> {
        self.files.get(path).map(Vec::as_slice).ok_or_else(|| format!("Missing model file: {path}"))
    }

    pub fn f32s(&self, path: &str) -> Result<Vec<f32>, String> {
        Ok(self.file(path)?.as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes(*c)).collect())
    }

    pub fn f64s(&self, path: &str) -> Result<Vec<f64>, String> {
        Ok(self.file(path)?.as_chunks::<8>().0.iter().map(|c| f64::from_le_bytes(*c)).collect())
    }

    /// A column of the scope table as doubles, whether the file holds int64 or float64.
    pub fn scope_column(&self, column: &ScopeColumn) -> Result<Vec<f64>, String> {
        let bytes = self.file(&column.file)?;
        match column.dtype.as_str() {
            "<i8" => Ok(bytes.as_chunks::<8>().0.iter().map(|c| i64::from_le_bytes(*c) as f64).collect()),
            "<f8" => self.f64s(&column.file),
            other => Err(format!("Unsupported scope column dtype: {other}")),
        }
    }

    pub fn graph(&self, name: &str) -> Result<&Graph, String> {
        self.graphs.iter().find(|g| g.name == name).ok_or_else(|| format!("runtime.json has no graph {name}"))
    }
}

impl Models {
    pub fn tensor(&self, spec: &ArraySpec) -> Result<crate::machine::Tensor, String> {
        use crate::machine::Tensor;
        let bytes = self.file(&spec.file)?;
        match spec.dtype.as_str() {
            "<f4" => Ok(Tensor::f32(&spec.shape, self.f32s(&spec.file)?)),
            "<i8" => Ok(Tensor::i64(&spec.shape, bytes.as_chunks::<8>().0.iter().map(|c| i64::from_le_bytes(*c)).collect())),
            other => Err(format!("Unsupported external array dtype: {other}")),
        }
    }
}
