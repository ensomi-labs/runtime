use ensomi_generate::package::check;
use ensomi_generate::Models;
use ensomi_generate_ort::package::open;
use serde_json::{json, Value};
use std::path::PathBuf;

/// The published package's manifest (`conformance/package`).
const PUBLISHED: &str = include_str!("../../../conformance/package/ensomi-package.json");

/// A new empty directory for one test.
fn directory(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ensomi-package-{name}-{}", std::process::id()));
    _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A package whose manifest lists one file, a 3-byte `runtime.json`, with `runtime` written as that file, or none.
fn package_of_one_file(name: &str, runtime: Option<&[u8]>) -> PathBuf {
    let dir = directory(name);
    let mut manifest: Value = serde_json::from_str(PUBLISHED).unwrap();
    manifest["notices"] = json!([]);
    manifest["files"] = json!([{"path": "runtime.json", "size": 3, "sha256": "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"}]);
    std::fs::write(dir.join("ensomi-package.json"), manifest.to_string()).unwrap();
    if let Some(bytes) = runtime {
        std::fs::write(dir.join("runtime.json"), bytes).unwrap();
    }
    dir
}

#[test]
fn refuses_a_package_with_a_listed_file_missing_or_of_another_size() {
    for (name, runtime) in [("missing", None), ("size", Some(&b"abcd"[..]))] {
        let error = open(&package_of_one_file(name, runtime)).err().unwrap();
        assert!(error.contains("runtime.json is missing or is not the 3 bytes ensomi-package.json lists"), "{error}");
    }
    // At its listed size the file is read as the runtime config, which these three bytes are not.
    let error = open(&package_of_one_file("listed", Some(&b"abc"[..]))).err().unwrap();
    assert!(error.starts_with("runtime.json: "), "{error}");
}

#[test]
fn refuses_a_package_its_manifest_check_refuses() {
    let dir = directory("refused");
    std::fs::write(dir.join("ensomi-package.json"), PUBLISHED.replace("\"version\": 1", "\"version\": 2")).unwrap();
    let error = open(&dir).err().unwrap();
    assert!(error.contains("pipeline r2@2"), "{error}");
}

#[test]
fn refuses_a_package_whose_manifest_leaves_out_a_file_the_config_names() {
    let runtime = br#"{"graphs": {"beatthis": {"file": "beatthis.onnx"}}}"#;
    let dir = directory("unlisted");
    let mut manifest: Value = serde_json::from_str(PUBLISHED).unwrap();
    manifest["notices"] = json!([]);
    manifest["files"] = json!([{"path": "runtime.json", "size": runtime.len(), "sha256": "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"}]);
    std::fs::write(dir.join("ensomi-package.json"), manifest.to_string()).unwrap();
    std::fs::write(dir.join("runtime.json"), runtime).unwrap();
    std::fs::write(dir.join("beatthis.onnx"), b"onnx").unwrap();
    assert_eq!(open(&dir).err().unwrap(), "beatthis.onnx is named by runtime.json but not listed in ensomi-package.json");
}

#[test]
fn notices_are_the_licences_the_config_names() {
    assert_eq!(Models::notice_files(&json!({"frontend_arrays": {"beatthis_license": "BeatThis-LICENSE.txt"}})), ["BeatThis-LICENSE.txt"]);
    assert!(Models::notice_files(&json!({"frontend_arrays": {}})).is_empty());
}

/// The released model directory and its runtime config, read from beside the repository, where bings-mac keeps them
/// (conformance/generate/README.md); without them the tests that need them pass without running.
fn release() -> Option<(PathBuf, Value)> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../ensomi-model/artifacts/release-r2/bulk/hf/ensomi-r2/onnx");
    let Ok(text) = std::fs::read(dir.join("runtime.json")) else {
        eprintln!("skipped: {} is absent", dir.display());
        return None;
    };
    Some((dir, serde_json::from_slice(&text).unwrap()))
}

/// The released config names BeatThis's licence as its notice, and a directory installed without it, as runtime 0.2.0
/// clients installed the model, still opens.
#[test]
fn opens_a_release_installed_without_its_notices() {
    let Some((release_dir, release)) = release() else { return };
    let notices = Models::notice_files(&release);
    assert_eq!(notices, ["BeatThis-LICENSE.txt"]);
    assert!(release_dir.join(&notices[0]).exists());

    let dir = std::env::temp_dir().join(format!("ensomi-package-no-notices-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for path in Models::required_files(&release).into_iter().chain(["runtime.json".to_string()]) {
        std::fs::copy(release_dir.join(&path), dir.join(&path)).unwrap();
    }
    assert_eq!(open(&dir).err(), None);
}

/// The released files with the published package's manifest beside them open, with the manifest's sizes; the manifest
/// lists the released runtime config and every file it names.
#[test]
fn opens_the_release_with_its_manifest() {
    let Some((release_dir, release)) = release() else { return };
    let package = check(PUBLISHED).unwrap();
    let dir = directory("release");
    for file in &package.files {
        std::fs::copy(release_dir.join(&file.path), dir.join(&file.path)).unwrap();
    }
    std::fs::write(dir.join("ensomi-package.json"), PUBLISHED).unwrap();
    let (models, read) = open(&dir).unwrap();
    assert_eq!(read, Some(package.clone()));
    let listed: Vec<&str> = package.files.iter().map(|f| f.path.as_str()).collect();
    let graphs = models.graphs.iter().map(|g| g.file.clone());
    for path in graphs.chain(Models::required_files(&release)).chain(Models::notice_files(&release)) {
        assert!(listed.contains(&path.as_str()), "{path}");
    }
}

/// A runtime config that does not hold together fails at open, naming what is wrong.
#[test]
fn refuses_an_inconsistent_runtime_config() {
    let Some((dir, release)) = release() else { return };
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
