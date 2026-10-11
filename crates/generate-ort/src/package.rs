//! A model directory as a client holds it: the runtime config (`runtime.json`), the files it names and,
//! when present, the EnsomiProtocol model-package manifest `ensomi-model.json`.
use ensomi_generate::capabilities::{MODEL_NAME, MODEL_VERSION};
use ensomi_generate::{Models, RUNTIME_VERSION};
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;

pub const MANIFEST: &str = "ensomi-model.json";

/// One file of the package, as the manifest lists it.
pub struct PackageFile {
    pub path: String,
    pub sha256_hex: String,
    pub size_bytes: u64,
}

/// What the runtime reads of `ensomi-model.json`.
pub struct Manifest {
    pub license: String,
    pub files: Vec<PackageFile>,
    pub runtime_config_path: String,
}

/// A field of a proto3 JSON object by its JSON name or its proto field name; parsers accept both.
fn field<'a>(object: &'a Value, json_name: &str, proto_name: &str) -> &'a Value {
    object.get(json_name).unwrap_or(&object[proto_name])
}

fn text(value: &Value) -> String {
    value.as_str().unwrap_or_default().to_string()
}

/// `major.minor.patch`, ignoring any pre-release or build suffix.
fn version(text: &str) -> Result<[u64; 3], String> {
    let core = text.split(['-', '+']).next().unwrap_or_default();
    let parts: Vec<u64> = core.split('.').map(|p| p.parse::<u64>()).collect::<Result<_, _>>().map_err(|_| format!("{MANIFEST}: invalid version {text:?}"))?;
    parts.try_into().map_err(|_| format!("{MANIFEST}: invalid version {text:?}"))
}

/// Reads `ensomi-model.json`, checking that this runtime is in its runtime range, that it describes the
/// model this runtime runs, and that every file it lists is present with its size. The SHA-256 of each
/// file is the downloader's check.
pub fn read_manifest(dir: &Path) -> Result<Manifest, String> {
    let bytes = std::fs::read(dir.join(MANIFEST)).map_err(|e| format!("{MANIFEST}: {e}"))?;
    let v: Value = serde_json::from_slice(&bytes).map_err(|e| format!("{MANIFEST}: {e}"))?;
    let (name, model_version) = (text(field(&v, "modelName", "model_name")), text(field(&v, "modelVersion", "model_version")));
    if name != MODEL_NAME || model_version != MODEL_VERSION {
        return Err(format!("{MANIFEST}: the package holds {name} {model_version}; this runtime runs {MODEL_NAME} {MODEL_VERSION}"));
    }
    let range = field(&v, "runtimeVersions", "runtime_versions");
    let min = text(field(range, "minVersion", "min_version"));
    let max = field(range, "maxVersionExclusive", "max_version_exclusive").as_str();
    let ours = version(RUNTIME_VERSION)?;
    if version(&min)? > ours || max.map(version).transpose()?.is_some_and(|max| max <= ours) {
        return Err(format!("{MANIFEST}: the package needs runtime {min} to {}; this runtime is {RUNTIME_VERSION}", max.unwrap_or("any later")));
    }
    let mut files = Vec::new();
    for file in field(&v, "files", "files").as_array().into_iter().flatten() {
        let size = field(file, "sizeBytes", "size_bytes");
        let size_bytes = size.as_u64().or_else(|| size.as_str().and_then(|s| s.parse().ok())).ok_or_else(|| format!("{MANIFEST}: a file without its size"))?;
        let path = text(&file["path"]);
        if std::fs::metadata(dir.join(&path)).map(|m| m.len()).ok() != Some(size_bytes) {
            return Err(format!("{path} is missing or is not the {size_bytes} bytes {MANIFEST} lists"));
        }
        files.push(PackageFile { path, sha256_hex: text(field(file, "sha256Hex", "sha256_hex")), size_bytes });
    }
    Ok(Manifest { license: text(&v["license"]), files, runtime_config_path: text(field(&v, "runtimeConfigPath", "runtime_config_path")) })
}

/// Opens a model directory: `ensomi-model.json` when present, then the runtime config it names
/// (`runtime.json` without a manifest) and the files the config names.
pub fn open(dir: &Path) -> Result<(Models, Option<Manifest>), String> {
    let manifest = if dir.join(MANIFEST).exists() { Some(read_manifest(dir)?) } else { None };
    let config_path = manifest.as_ref().map_or("runtime.json", |m| m.runtime_config_path.as_str());
    let config: Value = serde_json::from_slice(&std::fs::read(dir.join(config_path)).map_err(|e| format!("{config_path}: {e}"))?).map_err(|e| format!("{config_path}: {e}"))?;
    let files = Models::required_files(&config)
        .into_iter()
        .map(|path| Ok((path.clone(), std::fs::read(dir.join(&path)).map_err(|e| format!("{path}: {e}"))?)))
        .collect::<Result<HashMap<_, _>, String>>()?;
    Ok((Models::new(config, files)?, manifest))
}
