//! A model directory as a client holds it: the package's manifest (`ensomi-package.json`, `docs/model-package.md`)
//! when present, the runtime config and the files the config names.
use ensomi_generate::package::{check, check_config, PackageInfo, MANIFEST};
use ensomi_generate::Models;
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;

/// Opens a model directory. With `ensomi-package.json`, checks it, requires every file it lists at its listed size,
/// reads the runtime config it names and requires the manifest to list every file that config names; the SHA-256 of
/// each file is checked when the package arrives, not here. Without the manifest, reads `runtime.json`. Then the files
/// the config names.
pub fn open(dir: &Path) -> Result<(Models, Option<PackageInfo>), String> {
    let package = if dir.join(MANIFEST).exists() {
        let text = std::fs::read_to_string(dir.join(MANIFEST)).map_err(|e| format!("{MANIFEST}: {e}"))?;
        Some(check(&text)?)
    } else {
        None
    };
    for file in package.iter().flat_map(|p| &p.files) {
        if std::fs::metadata(dir.join(&file.path)).map(|m| m.len()).ok() != Some(file.size) {
            return Err(format!("{} is missing or is not the {} bytes {MANIFEST} lists", file.path, file.size));
        }
    }
    let config_path = package.as_ref().map_or("runtime.json", |p| p.config.as_str());
    let config: Value = serde_json::from_slice(&std::fs::read(dir.join(config_path)).map_err(|e| format!("{config_path}: {e}"))?).map_err(|e| format!("{config_path}: {e}"))?;
    if let Some(package) = &package {
        check_config(package, &config)?;
    }
    let files = Models::required_files(&config)
        .into_iter()
        .map(|path| Ok((path.clone(), std::fs::read(dir.join(&path)).map_err(|e| format!("{path}: {e}"))?)))
        .collect::<Result<HashMap<_, _>, String>>()?;
    Ok((Models::new(config, files)?, package))
}
