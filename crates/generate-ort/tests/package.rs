use ensomi_generate::{Models, RUNTIME_VERSION};
use ensomi_generate_ort::package::read_manifest;
use serde_json::{json, Value};
use std::path::PathBuf;

/// A directory with one 3-byte file and the given `ensomi-model.json`.
fn package(name: &str, manifest: Value) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ensomi-package-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("beatthis.onnx"), b"abc").unwrap();
    std::fs::write(dir.join("ensomi-model.json"), manifest.to_string()).unwrap();
    dir
}

fn manifest(min_version: &str, size: Value) -> Value {
    json!({
        "modelName": "Ensomi R2",
        "modelVersion": "r2-1.0",
        "license": "CC-BY-NC-SA-4.0",
        "files": [{"path": "beatthis.onnx", "sha256Hex": "ba7816bf", "sizeBytes": size}],
        "graphs": [{"name": "beatthis", "file": "beatthis.onnx", "opset": 17}],
        "runtimeConfigPath": "runtime.json",
        "runtimeVersions": {"minVersion": min_version},
    })
}

#[test]
fn reads_the_proto3_json_manifest() {
    let read = read_manifest(&package("camel", manifest("0.1.0", json!("3")))).unwrap();
    assert_eq!(read.license, "CC-BY-NC-SA-4.0");
    assert_eq!(read.runtime_config_path, "runtime.json");
    assert_eq!((read.files[0].path.as_str(), read.files[0].sha256_hex.as_str(), read.files[0].size_bytes), ("beatthis.onnx", "ba7816bf", 3));
}

#[test]
fn accepts_proto_field_names() {
    let snake = json!({
        "model_name": "Ensomi R2",
        "model_version": "r2-1.0",
        "files": [{"path": "beatthis.onnx", "sha256_hex": "ba7816bf", "size_bytes": 3}],
        "runtime_config_path": "runtime.json",
        "runtime_versions": {"min_version": "0.0.1", "max_version_exclusive": "1.0.0"},
    });
    assert_eq!(read_manifest(&package("snake", snake)).unwrap().files[0].sha256_hex, "ba7816bf");
}

#[test]
fn refuses_a_package_for_a_later_runtime() {
    let major: u64 = RUNTIME_VERSION.split('.').next().unwrap().parse().unwrap();
    let later = format!("{}.0.0", major + 1);
    let error = read_manifest(&package("later", manifest(&later, json!("3")))).err().unwrap();
    assert!(error.contains(&format!("needs runtime {later}")), "{error}");
    // The maximum is exclusive.
    let mut capped = manifest("0.0.1", json!("3"));
    capped["runtimeVersions"]["maxVersionExclusive"] = json!(RUNTIME_VERSION);
    assert!(read_manifest(&package("capped", capped)).is_err());
}

#[test]
fn refuses_a_file_of_another_size() {
    let error = read_manifest(&package("size", manifest("0.1.0", json!("4")))).err().unwrap();
    assert!(error.contains("beatthis.onnx"), "{error}");
}

#[test]
fn refuses_another_model() {
    let mut other = manifest("0.1.0", json!("3"));
    other["modelVersion"] = json!("r3-1.0");
    assert!(read_manifest(&package("other", other)).is_err());
}

/// A runtime config that does not hold together fails at open, naming what is wrong. The released config and the
/// files it names are read from beside the repository, where bings-mac keeps them (conformance/generate/README.md);
/// without them the test passes without running.
#[test]
fn refuses_an_inconsistent_runtime_config() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../ensomi-model/artifacts/release-r2/bulk/hf/ensomi-r2/onnx");
    let Ok(text) = std::fs::read(dir.join("runtime.json")) else {
        eprintln!("skipped: {} is absent", dir.display());
        return;
    };
    let release: Value = serde_json::from_slice(&text).unwrap();
    let files: std::collections::HashMap<String, Vec<u8>> = Models::required_files(&release).into_iter().map(|path| (path.clone(), std::fs::read(dir.join(&path)).unwrap())).collect();
    assert!(Models::new(release.clone(), files.clone()).is_ok());

    let refusal = |change: fn(&mut Value)| {
        let mut config = release.clone();
        change(&mut config);
        Models::new(config, files.clone()).err().unwrap()
    };
    for (error, names) in [
        (refusal(|c| _ = c["state"]["r2_commit_outputs"].as_object_mut().unwrap().remove("next_last")), "state.r2_commit_outputs"),
        (refusal(|c| _ = c["graphs"].as_object_mut().unwrap().remove("r2_release")), "no graph r2_release"),
        (refusal(|c| c["frontend_arrays"]["rows"]["mel_filter"]["path"] = json!("beatthis-mel-filter.bin")), "beatthis-mel-filter.bin: 65664 values"),
        (refusal(|c| _ = c["calibration"]["masks"].as_object_mut().unwrap().remove("4")), "calibration.masks: no band 4"),
    ] {
        assert!(error.contains(names), "{error}");
    }
}
