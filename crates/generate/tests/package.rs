//! `ensomi-package.json` checks (`docs/model-package.md`) on the published package's manifest (`conformance/package`)
//! and on copies of it with one thing changed.
use ensomi_generate::capabilities::{MODEL_ID, MODEL_NAME, MODEL_VERSION, PIPELINE_ID, PIPELINE_VERSION};
use ensomi_generate::package::{check, check_config};
use ensomi_generate::RUNTIME_VERSION;
use serde_json::{json, Value};

const PUBLISHED: &str = include_str!("../../../conformance/package/ensomi-package.json");

fn published() -> Value {
    serde_json::from_str(PUBLISHED).unwrap()
}

/// The refusal of the published manifest with `change` made.
fn refusal(change: impl FnOnce(&mut Value)) -> String {
    let mut manifest = published();
    change(&mut manifest);
    check(&manifest.to_string()).expect_err("refused")
}

#[test]
fn reads_the_published_manifest() {
    let package = check(PUBLISHED).unwrap();
    assert_eq!((package.model.id.as_str(), package.model.name.as_str(), package.model.version.as_str()), (MODEL_ID, MODEL_NAME, MODEL_VERSION));
    assert_eq!(package.license, "CC-BY-NC-SA-4.0 AND MIT");
    assert_eq!(package.notices, ["BeatThis-LICENSE.txt"]);
    assert_eq!(package.config, "runtime.json");
    assert_eq!(package.files.len(), 30);
    assert_eq!(package.total_bytes, 120_377_244);
    let notice = &package.files[0];
    assert_eq!((notice.path.as_str(), notice.size, notice.sha256.as_str()), ("BeatThis-LICENSE.txt", 1113, "909ab6549794a18e9bb243aacfadda4a5f308436fc2846f350755c53c4f06ae1"));
}

#[test]
fn ignores_unknown_fields_and_takes_an_earlier_min_runtime() {
    let mut manifest = published();
    manifest["graphs"] = json!([{"name": "beatthis", "file": "beatthis.onnx"}]);
    manifest["files"][0]["mode"] = json!("0644");
    manifest["min_runtime"] = json!("0.1.0");
    assert_eq!(check(&manifest.to_string()), check(PUBLISHED));
}

#[test]
fn refuses_another_format() {
    let error = refusal(|m| m["format"] = json!("ensomi-package/2"));
    assert!(error.contains("\"ensomi-package/2\"; this runtime reads ensomi-package/1"), "{error}");
    assert!(check("[]").is_err());
}

#[test]
fn refuses_a_package_for_a_later_runtime_naming_both_versions() {
    let parts: Vec<u64> = RUNTIME_VERSION.split(['-', '+']).next().unwrap().split('.').map(|p| p.parse().unwrap()).collect();
    let later = format!("{}.{}.{}", parts[0], parts[1], parts[2] + 1);
    let error = refusal(|m| m["min_runtime"] = json!(later));
    assert!(error.contains(&format!("needs runtime {later} or later; this runtime is {RUNTIME_VERSION}")), "{error}");
    for version in [json!("0.3"), json!("next"), json!(3), Value::Null] {
        let error = refusal(|m| m["min_runtime"] = version);
        assert!(error.contains("is not a version"), "{error}");
    }
}

#[test]
fn refuses_another_model_or_pipeline() {
    for (field, value) in [("id", "ensomi-r3"), ("name", "Ensomi R3"), ("version", "r2-1.1")] {
        let error = refusal(|m| m["model"][field] = json!(value));
        assert!(error.contains(value) && error.contains(&format!("this runtime runs {MODEL_ID} ({MODEL_NAME} {MODEL_VERSION})")), "{error}");
    }
    let error = refusal(|m| m["pipeline"]["version"] = json!(PIPELINE_VERSION + 1));
    assert!(error.contains(&format!("pipeline {PIPELINE_ID}@{}", PIPELINE_VERSION + 1)), "{error}");
    let error = refusal(|m| m["pipeline"]["id"] = json!("r3"));
    assert!(error.contains(&format!("pipeline r3@{PIPELINE_VERSION}")), "{error}");
}

#[test]
fn refuses_paths_outside_the_package() {
    for path in ["", "/runtime.json", "../runtime.json", "onnx/../runtime.json", "./runtime.json", "onnx//runtime.json", "onnx/", "onnx\\runtime.json"] {
        let error = refusal(|m| m["files"][0]["path"] = json!(path));
        assert!(error.contains("is not a /-separated path inside the package"), "{path}: {error}");
    }
}

#[test]
fn refuses_files_out_of_order_or_listed_twice() {
    let error = refusal(|m| m["files"].as_array_mut().unwrap().swap(1, 2));
    assert!(error.contains("sorted by path, each once: beatthis-mel-filter.bin comes before beatthis-hann-window.bin"), "{error}");
    let error = refusal(|m| {
        let first = m["files"][0].clone();
        m["files"].as_array_mut().unwrap().insert(0, first);
    });
    assert!(error.contains("BeatThis-LICENSE.txt comes before BeatThis-LICENSE.txt"), "{error}");
}

#[test]
fn refuses_a_hash_that_is_not_lowercase_hex() {
    let hash = published()["files"][3]["sha256"].as_str().unwrap().to_string();
    for wrong in [hash.to_uppercase(), hash[1..].to_string(), String::new()] {
        let error = refusal(|m| m["files"][3]["sha256"] = json!(wrong));
        assert!(error.contains("beatthis.onnx: SHA-256") && error.contains("64 lowercase hex digits"), "{error}");
    }
}

#[test]
fn refuses_notices_or_a_config_outside_the_files() {
    let error = refusal(|m| m["notices"] = json!(["LICENSE.txt"]));
    assert!(error.contains("notice LICENSE.txt is not among the files"), "{error}");
    let error = refusal(|m| m["config"] = json!("config.json"));
    assert!(error.contains("config config.json is not among the files"), "{error}");
}

#[test]
fn refuses_a_manifest_without_a_field_it_needs() {
    for field in ["model", "license", "notices", "pipeline", "config", "files"] {
        let error = refusal(|m| _ = m.as_object_mut().unwrap().remove(field));
        assert!(error.contains(&format!("missing field `{field}`")), "{error}");
    }
    let error = refusal(|m| _ = m["files"][0].as_object_mut().unwrap().remove("size"));
    assert!(error.contains("missing field `size`"), "{error}");
}

/// A runtime config naming a graph, an array, a state file, a scope column and a notice, all among the published files.
fn config() -> Value {
    json!({
        "graphs": {"beatthis": {"file": "beatthis.onnx"}},
        "frontend_arrays": {"beatthis": {"mel_filter": {"path": "beatthis-mel-filter.bin"}}, "beatthis_license": "BeatThis-LICENSE.txt"},
        "state": {"r2_initial": {"rows": {"file": "r2_initial_rows.bin"}}},
        "calibration": {"scope_table": {"columns": [{"name": "S", "file": "scopes-S.bin"}]}},
    })
}

#[test]
fn refuses_a_manifest_that_leaves_out_a_file_the_config_names() {
    assert_eq!(check_config(&check(PUBLISHED).unwrap(), &config()), Ok(()));
    for path in ["beatthis.onnx", "beatthis-mel-filter.bin", "r2_initial_rows.bin", "scopes-S.bin", "BeatThis-LICENSE.txt"] {
        let mut manifest = published();
        manifest["files"].as_array_mut().unwrap().retain(|file| file["path"] != path);
        manifest["notices"].as_array_mut().unwrap().retain(|notice| notice != path);
        let package = check(&manifest.to_string()).unwrap();
        assert_eq!(check_config(&package, &config()), Err(format!("{path} is named by runtime.json but not listed in ensomi-package.json")));
    }
}
