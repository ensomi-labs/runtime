//! EnsomiProtocol 0.2 `ModelCapabilities` for the released R2 model, with its numeric enum values.
use serde_json::{json, Value};

const WORKS: u32 = 1;
const EXPERIMENTAL: u32 = 2;
const NOT_WORKING: u32 = 3;

pub const MODEL_NAME: &str = "Ensomi R2";
pub const MODEL_VERSION: &str = "r2-1.0";

pub fn capabilities(paths: &[String]) -> Value {
    let control = |name: &str, status: u32, note: &str| json!({"name": name, "status": status, "note": note});
    json!({
        "model_name": MODEL_NAME,
        "model_version": MODEL_VERSION,
        "files": paths.iter().map(|path| json!({"path": path, "sha256_hex": ""})).collect::<Vec<_>>(),
        "min_star": 2,
        "max_star_exclusive": 6,
        "controls": [
            control("star", WORKS, "Whole-song star rating; both blind pairs called the right way."),
            control("jumpstream", WORKS, "A span of 8 to 32 seconds in that style; works in blind pairs."),
            control("chordstream", WORKS, "A span of 8 to 32 seconds in that style; works in blind pairs."),
            control("stream", EXPERIMENTAL, "Reaches its target band in measurement (48 of 48 runs); not screened on its own."),
            control("jack", EXPERIMENTAL, "Reaches its target band in measurement (35 of 48 runs); not screened on its own."),
            control("section_star", EXPERIMENTAL, "An absolute star target for one span; not screened, and easier spans land short."),
            control("chordjack", NOT_WORKING, "Out of band on generated rows (18 of 28 runs)."),
            control("long_notes", NOT_WORKING, "Tap only, half or natural; half removes about a third and a blind pair could not tell."),
        ],
    })
}
