//! The corpus prior: the held, head, busy and quad shares a chart of this star band is expected to have.
use crate::grid::Grid;
use crate::jsmath;
use crate::math::{clip, median};
use crate::models::Prior;
use crate::strain::reference_workload;
use serde::Serialize;

#[derive(Clone, Serialize)]
pub struct Prediction {
    pub held: f64,
    pub heads: f64,
    pub busy_free: f64,
    pub quad: f64,
    pub log_r: f64,
}

pub fn rows_features(head_ms: &[f64], song_ms: f64, grid: &Grid) -> [f64; 4] {
    let seconds = song_ms / 1000.0;
    let gap = if head_ms.len() > 1 { median(&head_ms[1..].iter().zip(head_ms).map(|(t, p)| t - p).collect::<Vec<_>>()) } else { song_ms };
    [
        jsmath::log(head_ms.len() as f64 / seconds),
        jsmath::log(f64::max(gap, 1.0)),
        jsmath::log(f64::max(reference_workload(head_ms), 1e-6) / seconds),
        jsmath::log(median(&head_ms.iter().map(|&t| grid.bpm(t)).collect::<Vec<_>>())),
    ]
}

pub fn predict_prior(prior: &Prior, features: &[f64; 4], band: u32) -> Result<Prediction, String> {
    let p = prior.bands.get(&band.to_string()).ok_or_else(|| format!("The prior has no band {band}"))?;
    let values: Vec<f64> = match prior.method.as_str() {
        "median" => p.median.clone(),
        "linear" => (0..5)
            .map(|i| {
                let mut s = 0.0;
                for (j, x) in std::iter::once(1.0).chain(features.iter().copied()).enumerate() {
                    s += x * p.coef[j][i];
                }
                s
            })
            .collect(),
        _ => {
            let z: Vec<f64> = features.iter().enumerate().map(|(i, x)| (x - p.mean[i]) / p.sd[i]).collect();
            let mut distances: Vec<(usize, f64)> = p
                .table
                .iter()
                .enumerate()
                .map(|(i, row)| {
                    let mut s = 0.0;
                    for (j, x) in z.iter().enumerate() {
                        s += (row[j] - x) * (row[j] - x);
                    }
                    (i, s.sqrt())
                })
                .collect();
            distances.sort_by(|a, b| crate::fitter::order(a.1 - b.1));
            let near: Vec<&Vec<f64>> = distances.iter().take(prior.neighbours.unwrap_or(64.0) as usize).map(|x| &p.table[x.0]).collect();
            (0..5).map(|i| median(&near.iter().map(|row| row[i + 4]).collect::<Vec<_>>())).collect()
        }
    };
    Ok(Prediction { held: clip(values[0], 0.0, 1.0), heads: clip(values[1], 1.0, 4.0), busy_free: clip(values[2], 0.0, 1.0), quad: clip(values[3], 0.0, 1.0), log_r: values[4] })
}

pub fn kappa_targets(p: &Prediction, ln_fraction: f64, multiplier: f64) -> [f64; 3] {
    [p.held * ln_fraction, p.heads * multiplier, p.busy_free * ln_fraction + p.quad * f64::min(1.0, multiplier)]
}
