//! Opt-in measurement arms. Clients never set these; absence preserves the released pipeline.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RowsExtrapolation {
    #[default]
    Clamped,
    Overall,
    EndSegment,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Experiment {
    pub rows: RowsExtrapolation,
    pub rows_min: Option<f64>,
    pub rows_max: Option<f64>,
    pub difficulty_q: Option<f64>,
    pub multiplier_max: Option<f64>,
    pub multiplier_target: Option<f64>,
    pub multiplier_blend: Option<f64>,
    pub reattack_floor_ms: Option<f64>,
    pub quad_run_cap: Option<f64>,
}

/// Extend only outside the table; keep interpolation's exact arithmetic inside it.
pub(crate) fn rows_request(star: f64, stars: &[f64], requests: &[f64], e: &Experiment) -> f64 {
    let last = stars.len() - 1;
    let base = crate::math::interp(star, stars, requests);
    if (stars[0]..=stars[last]).contains(&star) || matches!(e.rows, RowsExtrapolation::Clamped) {
        return base;
    }
    let low = star < stars[0];
    let (a, b) = match e.rows {
        RowsExtrapolation::Overall => (0, last),
        RowsExtrapolation::EndSegment if low => (0, 1),
        RowsExtrapolation::EndSegment => (last - 1, last),
        RowsExtrapolation::Clamped => unreachable!(),
    };
    let edge = if low { 0 } else { last };
    let value =
        requests[edge] + (star - stars[edge]) * (requests[b] - requests[a]) / (stars[b] - stars[a]);
    value.clamp(
        e.rows_min.unwrap_or(f64::NEG_INFINITY),
        e.rows_max.unwrap_or(f64::INFINITY),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extensions_preserve_the_table_and_distinguish_the_two_slopes() {
        let stars = [2.125, 3.0, 5.875];
        let requests = [1.5, 2.5, 3.5];
        for mode in [RowsExtrapolation::Overall, RowsExtrapolation::EndSegment] {
            let e = Experiment {
                rows: mode,
                rows_min: Some(0.0),
                rows_max: Some(6.0),
                ..Experiment::default()
            };
            for star in [2.125, 2.5, 3.0, 4.7, 5.875] {
                assert_eq!(
                    rows_request(star, &stars, &requests, &e).to_bits(),
                    crate::math::interp(star, &stars, &requests).to_bits()
                );
            }
            assert_eq!(rows_request(-100.0, &stars, &requests, &e), 0.0);
            assert_eq!(rows_request(100.0, &stars, &requests, &e), 6.0);
        }
        let overall = Experiment {
            rows: RowsExtrapolation::Overall,
            ..Experiment::default()
        };
        let end = Experiment {
            rows: RowsExtrapolation::EndSegment,
            ..Experiment::default()
        };
        assert!(
            rows_request(1.0, &stars, &requests, &end)
                < rows_request(1.0, &stars, &requests, &overall)
        );
        assert_eq!(
            rows_request(1.0, &stars, &requests, &Experiment::default()),
            1.5
        );
    }
}
