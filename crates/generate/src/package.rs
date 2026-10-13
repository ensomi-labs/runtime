//! `ensomi-package.json`, the manifest of a model package (format `ensomi-package/1`, `docs/model-package.md`), read
//! and checked without IO. Opening or downloading the files it lists is the caller's.
use crate::capabilities::{MODEL_ID, MODEL_NAME, MODEL_VERSION, PIPELINE_ID, PIPELINE_VERSION};
use crate::{Models, RUNTIME_VERSION};
use serde::Deserialize;
use serde_json::Value;

/// The manifest's name, at the package root beside the runtime config.
pub const MANIFEST: &str = "ensomi-package.json";
const FORMAT: &str = "ensomi-package/1";

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct PackageModel {
    pub id: String,
    pub name: String,
    pub version: String,
}

/// One file of the package.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct PackageFile {
    /// Relative to the manifest, `/`-separated.
    pub path: String,
    pub size: u64,
    /// Lowercase hex SHA-256 of the file.
    pub sha256: String,
}

/// What a client needs of a checked manifest.
#[derive(Clone, Debug, PartialEq)]
pub struct PackageInfo {
    pub model: PackageModel,
    /// An SPDX licence expression.
    pub license: String,
    /// The licence notices among `files`, for the app to show with the model.
    pub notices: Vec<String>,
    /// Every file of the package but the manifest, sorted by path.
    pub files: Vec<PackageFile>,
    /// The sum of the files' sizes.
    pub total_bytes: u64,
    /// The runtime config (`runtime.json`), one of `files`.
    pub config: String,
}

#[derive(Deserialize)]
struct Pipeline {
    id: String,
    version: u64,
}

/// The fields of `ensomi-package/1` read after the format and `min_runtime`; any others are ignored.
#[derive(Deserialize)]
struct Manifest {
    model: PackageModel,
    license: String,
    notices: Vec<String>,
    pipeline: Pipeline,
    config: String,
    files: Vec<PackageFile>,
}

fn error(message: impl std::fmt::Display) -> String {
    format!("{MANIFEST}: {message}")
}

/// `major.minor.patch`, ignoring any pre-release or build suffix.
fn version(text: &str) -> Option<[u64; 3]> {
    let parts: Vec<u64> = text.split(['-', '+']).next()?.split('.').map(|part| part.parse().ok()).collect::<Option<_>>()?;
    parts.try_into().ok()
}

/// Relative to the manifest: `/`-separated parts, none of them empty, `.` or `..`.
fn relative(path: &str) -> bool {
    !path.contains('\\') && path.split('/').all(|part| !part.is_empty() && part != "." && part != "..")
}

/// Reads an `ensomi-package.json` and checks it: the format, `min_runtime` at or below this runtime's version, the
/// model and pipeline this runtime runs, relative paths sorted by path with lowercase hex SHA-256s, and the notices and
/// config among the files.
pub fn check(manifest_json: &str) -> Result<PackageInfo, String> {
    let value: Value = serde_json::from_str(manifest_json).map_err(error)?;
    if value["format"] != FORMAT {
        return Err(error(format!("format {}; this runtime reads {FORMAT}", value["format"])));
    }
    let min_runtime = value["min_runtime"].as_str().unwrap_or_default();
    let min = version(min_runtime).ok_or_else(|| error(format!("min_runtime {} is not a version", value["min_runtime"])))?;
    if Some(min) > version(RUNTIME_VERSION) {
        return Err(error(format!("the package needs runtime {min_runtime} or later; this runtime is {RUNTIME_VERSION}")));
    }
    let manifest: Manifest = serde_json::from_value(value).map_err(error)?;
    let model = &manifest.model;
    if (model.id.as_str(), model.name.as_str(), model.version.as_str()) != (MODEL_ID, MODEL_NAME, MODEL_VERSION) {
        return Err(error(format!("the package holds {} ({} {}); this runtime runs {MODEL_ID} ({MODEL_NAME} {MODEL_VERSION})", model.id, model.name, model.version)));
    }
    let pipeline = &manifest.pipeline;
    if (pipeline.id.as_str(), pipeline.version) != (PIPELINE_ID, PIPELINE_VERSION) {
        return Err(error(format!("the package needs pipeline {}@{}; this runtime implements {PIPELINE_ID}@{PIPELINE_VERSION}", pipeline.id, pipeline.version)));
    }
    for file in &manifest.files {
        if !relative(&file.path) {
            return Err(error(format!("{:?} is not a /-separated path inside the package", file.path)));
        }
        if file.sha256.len() != 64 || !file.sha256.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
            return Err(error(format!("{}: SHA-256 {:?} is not 64 lowercase hex digits", file.path, file.sha256)));
        }
    }
    if let Some(pair) = manifest.files.windows(2).find(|pair| pair[0].path >= pair[1].path) {
        return Err(error(format!("files must be sorted by path, each once: {} comes before {}", pair[0].path, pair[1].path)));
    }
    let listed = |path: &str| manifest.files.iter().any(|file| file.path == path);
    if let Some(notice) = manifest.notices.iter().find(|notice| !listed(notice)) {
        return Err(error(format!("notice {notice} is not among the files")));
    }
    if !listed(&manifest.config) {
        return Err(error(format!("config {} is not among the files", manifest.config)));
    }
    let total_bytes = manifest.files.iter().fold(0u64, |total, file| total.saturating_add(file.size));
    Ok(PackageInfo { model: manifest.model, license: manifest.license, notices: manifest.notices, files: manifest.files, total_bytes, config: manifest.config })
}

/// Checks that a checked manifest lists every file the runtime config it names needs: the graphs, the files
/// `Models::new` reads and the licence notices. Run on that config once it is read, before the other files.
pub fn check_config(package: &PackageInfo, config: &Value) -> Result<(), String> {
    let graphs = config["graphs"].as_object().into_iter().flat_map(|graphs| graphs.values()).filter_map(|graph| graph["file"].as_str().map(str::to_string));
    let missing = graphs.chain(Models::required_files(config)).chain(Models::notice_files(config)).find(|path| !package.files.iter().any(|file| file.path == *path));
    match missing {
        Some(path) => Err(format!("{path} is named by {} but not listed in {MANIFEST}", package.config)),
        None => Ok(()),
    }
}
