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
        Ok(Models {
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
        })
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
        Ok(self.file(path)?.chunks_exact(4).map(|c| f32::from_le_bytes(c.try_into().unwrap())).collect())
    }

    pub fn f64s(&self, path: &str) -> Result<Vec<f64>, String> {
        Ok(self.file(path)?.chunks_exact(8).map(|c| f64::from_le_bytes(c.try_into().unwrap())).collect())
    }

    /// A column of the scope table as doubles, whether the file holds int64 or float64.
    pub fn scope_column(&self, column: &ScopeColumn) -> Result<Vec<f64>, String> {
        let bytes = self.file(&column.file)?;
        match column.dtype.as_str() {
            "<i8" => Ok(bytes.chunks_exact(8).map(|c| i64::from_le_bytes(c.try_into().unwrap()) as f64).collect()),
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
            "<i8" => Ok(Tensor::i64(&spec.shape, bytes.chunks_exact(8).map(|c| i64::from_le_bytes(c.try_into().unwrap())).collect())),
            other => Err(format!("Unsupported external array dtype: {other}")),
        }
    }
}
