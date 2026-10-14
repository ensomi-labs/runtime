//! audio_rows/rowsgen.py: song encoder, GRU, copy mixture, quiet rests and section tilt. Beats are the grid's whole
//! beats (`Grid::whole_beats`), so every position lies on its own segment's beat lines; a beat that the next segment
//! cuts short keeps only its positions at least a sixteenth of a beat before it (`available_positions`).
use crate::grid::Grid;
use crate::machine::{Io, Tensor};
use crate::math::{clip, f32r, interp, modp, round, sum};
use crate::models::Models;
use crate::random::RandomSource;
use crate::strain::{StrainState, REFERENCE_LANES};

pub struct RowsSection {
    pub start_ms: f64,
    pub end_ms: f64,
    pub star: f64,
}

pub struct RowsResult {
    pub head_ms: Vec<f64>,
    pub request: f64,
    pub dropped: usize,
}

pub fn star_request(models: &Models, star: f64) -> f64 {
    let map = &models.rows.record.star_map;
    interp(star, &map.star, &map.request)
}

const TEMPERATURE: f32 = 1.0;
const QUIET_RESTS: f64 = 0.4;
const SECTION_GAIN: f64 = 3.0;

struct SongInputs {
    nb: usize,
    edges: Vec<f64>,
    position_ms: Vec<f64>,
    extra: Vec<f32>,
    target: Vec<f64>,
}

fn song_inputs(models: &Models, grid: &Grid, song_ms: f64, request: f64, sections: &[RowsSection]) -> SongInputs {
    let record = &models.rows.record;
    let positions = &models.positions;
    let b0 = grid.beat(0.0).floor();
    let nb = f64::max((grid.beat(song_ms)).ceil() - b0, 1.0) as usize;
    let edges: Vec<f64> = (0..=nb).map(|i| grid.time(b0 + i as f64)).collect();
    let mut position_ms = vec![0.0; nb * 24];
    let mut extra = vec![0f32; nb * 9];
    let mut target = vec![f64::NAN; nb];
    let standard = |x: f64| (x - record.label_mean) / record.label_std;
    for b in 0..nb {
        let time = clip(edges[b], 0.0, song_ms);
        extra[b * 9 + modp(b0 + b as f64, 4.0) as usize] = 1.0;
        extra[b * 9 + 4] = (time / song_ms) as f32;
        extra[b * 9 + 5] = (grid.bpm(time) / 120.0) as f32;
        extra[b * 9 + 6] = standard(request) as f32;
        for p in 0..24 {
            position_ms[b * 24 + p] = grid.time(b0 + b as f64 + positions[p] / 48.0);
        }
        for section in sections {
            if time >= section.start_ms && time < section.end_ms {
                let value = star_request(models, section.star);
                extra[b * 9 + 7] = standard(value) as f32;
                extra[b * 9 + 8] = 1.0;
                target[b] = value;
            }
        }
    }
    SongInputs { nb, edges, position_ms, extra, target }
}

/// rowsgen.available_positions: [nb * 24] positions a figure may use, inside [0, T - 2 ms] and at least a sixteenth of
/// its segment's beat before the next beat or the next segment starts, whichever is first. A whole beat's last position
/// (15/16) is exactly that far from the next beat, so only a beat cut short by the next segment loses positions, and
/// none lands nearer that segment's first one; a segment under half a beat long, which has no beat of its own, stops
/// the beat before it all the same. 1e-6 ms absorbs the float error at 15/16.
pub(crate) fn available_positions(grid: &Grid, edges: &[f64], position_ms: &[f64], song_ms: f64) -> Vec<bool> {
    let mut available = vec![false; position_ms.len()];
    for b in 0..edges.len() - 1 {
        let s = grid.segment(edges[b]);
        let next_offset = grid.offsets.get(s + 1).copied().unwrap_or(f64::INFINITY);
        let end = f64::min(edges[b + 1], next_offset) - grid.lengths[s] / 16.0 + 1e-6;
        for p in 0..24 {
            let x = position_ms[b * 24 + p];
            available[b * 24 + p] = x >= 0.0 && x <= song_ms - 2.0 && x <= end;
        }
    }
    available
}

/// How quiet each beat is relative to the song, as a rank in [0, 1].
fn quiet_rank(mel: &[f32], beat_frames: &[i64]) -> Vec<f64> {
    let window = 4usize;
    let nf = mel.len() / 128;
    let nb = beat_frames.len() / 2;
    let mut cs = vec![0f64; nf + 1];
    for f in 0..nf {
        let mut total = 0.0;
        for m in 0..128 {
            total += mel[f * 128 + m] as f64;
        }
        cs[f + 1] = cs[f] + f32r(total / 128.0);
    }
    let intensity: Vec<f64> = (0..nb)
        .map(|b| {
            let lo = beat_frames[b.saturating_sub(window) * 2] as usize;
            let hi = beat_frames[(b + window).min(nb - 1) * 2 + 1] as usize;
            (cs[hi] - cs[lo]) / f64::max(hi as f64 - lo as f64, 1.0)
        })
        .collect();
    let mut order: Vec<usize> = (0..nb).collect();
    order.sort_by(|&a, &b| crate::fitter::order(intensity[a] - intensity[b]));
    let mut ranks = vec![0f64; nb];
    for (r, &b) in order.iter().enumerate() {
        ranks[b] = r as f64 / f64::max(nb as f64 - 1.0, 1.0);
    }
    ranks
}

fn clean_heads(heads: &[f64], song_ms: f64) -> Vec<f64> {
    let mut rounded: Vec<f64> = heads.iter().map(|&t| round(t)).collect();
    rounded.sort_by(|a, b| crate::fitter::order(a - b));
    let mut out: Vec<f64> = Vec::new();
    for t in rounded {
        if t >= 0.0 && t <= song_ms - 2.0 && out.last().is_none_or(|&last| t - last >= 2.0) {
            out.push(t);
        }
    }
    out
}

fn tensor_slice(data: &[f32], b: usize, size: usize, shape: &[usize]) -> Tensor {
    Tensor::f32(shape, data[b * size..(b + 1) * size].to_vec())
}

pub async fn decode_rows(io: &Io, models: &Models, features: &[f32], grid: &Grid, song_ms: f64, star: f64, random: &mut dyn RandomSource, sections: &[RowsSection]) -> Result<RowsResult, String> {
    let cfg = &models.rows.config;
    let record = &models.rows.record;
    let (v_size, lags, heads, key) = (cfg.vocab, cfg.lags, cfg.heads, cfg.key);
    let request = star_request(models, star);
    let grid = &grid.whole_beats();
    let SongInputs { nb, edges, position_ms, extra, target } = song_inputs(models, grid, song_ms, request, sections);
    let nf = features.len() / 130;
    let mut mel = vec![0f32; nf * 128];
    for f in 0..nf {
        mel[f * 128..(f + 1) * 128].copy_from_slice(&features[f * 130..f * 130 + 128]);
    }
    let pos_frames: Vec<i64> = position_ms.iter().map(|&x| clip((x / 10.0).trunc(), 0.0, nf as f64 - 1.0) as i64).collect();
    let bf: Vec<i64> = edges.iter().map(|&x| clip((x / 10.0).trunc(), 0.0, nf as f64) as i64).collect();
    let mut beat_frames = vec![0i64; nb * 2];
    for b in 0..nb {
        beat_frames[b * 2] = bf[b];
        beat_frames[b * 2 + 1] = (nf as i64).min(bf[b + 1].max(bf[b] + 1));
    }
    let available = available_positions(grid, &edges, &position_ms, song_ms);
    let quiet = quiet_rank(&mel, &beat_frames);
    let mut encoded = io
        .run(
            "rows_encoder",
            vec![
                ("mel", Tensor::f32(&[1, nf, 128], mel)),
                ("pos_frames", Tensor::i64(&[1, nb, 24], pos_frames)),
                ("beat_frames", Tensor::i64(&[1, nb, 2], beat_frames)),
                ("extra", Tensor::f32(&[1, nb, 9], extra.clone())),
                ("valid", Tensor::bool(&[1, nb], vec![1; nb])),
            ],
        )
        .await?;
    let (enc_pos, enc_context, enc_query, enc_key) = (encoded.take_f32("pos", nb * 24 * cfg.enc)?, encoded.take_f32("context", nb * cfg.ctx)?, encoded.take_f32("query", nb * heads * key)?, encoded.take_f32("key", nb * heads * key)?);
    let mut ids = vec![-1i32; nb];
    let mut figures = vec![0u32; nb];
    let mut tilts = vec![0f32; nb];
    let mut wh = vec![0f64; nb];
    let counts: Vec<u32> = record.vocab.iter().map(|f| f.count_ones()).collect();
    let mut hidden = vec![0f32; cfg.hidden];
    let mut prev = v_size as i64;
    let mut strain = StrainState::default();
    let (mut n_heads, mut section_start) = (0usize, 0usize);
    let steer = SECTION_GAIN != 0.0 && target.iter().any(|x| !x.is_nan());
    for b in 0..nb {
        let mut back_ids = vec![-1i32; lags];
        let mut back_keys = vec![0f32; lags * heads * key];
        let mut eq = vec![0f32; lags * 2];
        let mut ok = vec![0u8; lags];
        let is_quiet = quiet[b] < QUIET_RESTS;
        for lag in 0..lags {
            let j = b as i64 - 1 - lag as i64;
            let jj = j.max(0) as usize;
            if j >= 0 {
                back_ids[lag] = ids[j as usize];
            }
            back_keys[lag * heads * key..(lag + 1) * heads * key].copy_from_slice(&enc_key[jj * heads * key..(jj + 1) * heads * key]);
            for d in 1..=2i64 {
                let before = if b as i64 - d >= 0 { ids[(b as i64 - d) as usize] } else { -1 };
                let bj = if jj as i64 - d >= 0 { ids[(jj as i64 - d) as usize] } else { -1 };
                eq[lag * 2 + d as usize - 1] = (j >= 0 && bj == before && before >= 0) as u8 as f32;
            }
            ok[lag] = (j >= 0 && (is_quiet || back_ids[lag] != 0)) as u8;
        }
        let mut out = io
            .run(
                "rows_step",
                vec![
                    ("context", tensor_slice(&enc_context, b, cfg.ctx, &[1, cfg.ctx])),
                    ("pos", tensor_slice(&enc_pos, b, 24 * cfg.enc, &[1, 24, cfg.enc])),
                    ("query", tensor_slice(&enc_query, b, heads * key, &[1, heads, key])),
                    ("extra", Tensor::f32(&[1, 9], extra[b * 9..(b + 1) * 9].to_vec())),
                    ("prev", Tensor::i64(&[1], vec![prev])),
                    ("hidden", Tensor::f32(&[1, 1, cfg.hidden], std::mem::take(&mut hidden))),
                    ("back_keys", Tensor::f32(&[1, lags, heads, key], back_keys)),
                    ("eq", Tensor::f32(&[1, lags, 2], eq)),
                    ("back_ok", Tensor::bool(&[1, lags], ok.clone())),
                    ("temperature", Tensor::f32(&[], vec![TEMPERATURE])),
                ],
            )
            .await?;
        hidden = out.take_f32("hidden_out", cfg.hidden)?;
        let gate = out.take_f32("gate", heads + 1)?;
        let att = out.take_f32("attention", heads * (lags + 1))?;
        let logp = out.take_f32("logp", v_size)?;
        let mut pgen = vec![0f32; v_size];
        let mut p = vec![0f32; v_size];
        let mut ph = vec![0f32; v_size];
        for v in 0..v_size {
            pgen[v] = libm::exp(logp[v] as f64) as f32;
            p[v] = f32r(gate[0] as f64 * pgen[v] as f64) as f32;
        }
        for hd in 0..heads {
            let base = hd * (lags + 1);
            let generate = att[base + lags] as f64;
            let weight = gate[hd + 1] as f64;
            for v in 0..v_size {
                ph[v] = f32r(generate * pgen[v] as f64) as f32;
            }
            for lag in 0..lags {
                if ok[lag] != 0 {
                    let at = back_ids[lag] as usize;
                    ph[at] = f32r(ph[at] as f64 + att[base + lag] as f64) as f32;
                }
            }
            for v in 0..v_size {
                p[v] = f32r(p[v] as f64 + f32r(weight * ph[v] as f64)) as f32;
            }
        }
        let mut forbidden = 0u32;
        for pos in 0..24 {
            if !available[b * 24 + pos] {
                forbidden |= 1 << pos;
            }
        }
        for v in 0..v_size {
            if record.vocab[v] & forbidden != 0 {
                p[v] = 0.0;
            }
        }
        if steer && !target[b].is_nan() {
            if b == 0 || target[b - 1] != target[b] {
                section_start = b;
            }
            let a = (section_start as i64).min(b as i64 - 4).max(0) as usize;
            let ta = clip(edges[a], 0.0, song_ms);
            let tb = clip(edges[b], 0.0, song_ms);
            if b > a && tb > ta {
                let realised = libm::log(f64::max(sum(&wh[a..b]) / ((tb - ta) / 1000.0), 0.001));
                tilts[b] = clip(SECTION_GAIN * (target[b] - realised), -2.0, 2.0) as f32;
                for v in 0..v_size {
                    p[v] = f32r(p[v] as f64 * f32r(libm::exp(f32r(tilts[b] as f64 * counts[v] as f64)))) as f32;
                }
            }
        }
        let total = f32r(p.iter().fold(0.0, |a, &x| a + x as f64));
        let probability: Vec<f32> = p.iter().map(|&x| f32r(x as f64 / total) as f32).collect();
        let chosen = random.multinomial(&probability, &format!("rows/{b}/figure"))?;
        ids[b] = chosen as i32;
        figures[b] = record.vocab[chosen];
        prev = chosen as i64;
        if steer {
            for pos in 0..24 {
                if figures[b] & (1 << pos) != 0 {
                    let mut q = [0.0; 4];
                    q[REFERENCE_LANES[n_heads % 4]] = 1.0;
                    let (next, charge) = strain.append(round(position_ms[b * 24 + pos] * 1000.0) / 1000.0, &q);
                    strain = next;
                    n_heads += 1;
                    wh[b] += charge;
                }
            }
        }
    }
    let mut times = Vec::new();
    for b in 0..nb {
        for p in 0..24 {
            if figures[b] & (1 << p) != 0 {
                times.push(round(position_ms[b * 24 + p] * 1000.0) / 1000.0);
            }
        }
    }
    let mut raw_heads = crate::fitter::unique(times);
    raw_heads.sort_by(|a, b| crate::fitter::order(a - b));
    let head_ms = clean_heads(&raw_heads, song_ms);
    let dropped = raw_heads.len() - head_ms.len();
    Ok(RowsResult { head_ms, request, dropped })
}
