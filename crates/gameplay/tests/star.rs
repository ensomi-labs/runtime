use ensomi_gameplay::{ChartObject, parse_osu, star_rating};
use serde::Deserialize;
use std::path::Path;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Vector {
    name: String,
    objects: Option<Vec<Object>>,
    osu: Option<String>,
    star_rating: f64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Object {
    lane: u32,
    start_ms: u32,
    end_ms: Option<u32>,
}

#[test]
fn matches_every_python_vector() {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../conformance/star");
    let vectors: Vec<Vector> =
        serde_json::from_str(include_str!("../../../conformance/star/vectors.json")).unwrap();
    assert_eq!(vectors.len(), 8);
    for vector in vectors {
        let objects = match (vector.objects, vector.osu) {
            (Some(objects), None) => objects
                .into_iter()
                .map(|o| ChartObject {
                    lane: o.lane,
                    start_ms: o.start_ms,
                    end_ms: o.end_ms,
                })
                .collect(),
            (None, Some(path)) => {
                parse_osu(&std::fs::read_to_string(directory.join(path)).unwrap())
                    .unwrap()
                    .chart
                    .objects
            }
            _ => panic!("{}: expected objects or osu", vector.name),
        };
        let actual = star_rating(&objects);
        if vector.star_rating == 0.0 {
            assert_eq!(actual, 0.0, "{}", vector.name);
        } else {
            let relative_error = (actual / vector.star_rating - 1.0).abs();
            assert!(
                relative_error <= 1e-9,
                "{}: {actual} vs {}, relative error {relative_error}",
                vector.name,
                vector.star_rating
            );
        }
    }
}
