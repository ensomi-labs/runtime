//! The helpers of the TypeScript runtime's `math.ts`, with JavaScript's semantics.
use crate::jsmath;

/// `Math.fround`.
pub fn f32r(x: f64) -> f64 {
    x as f32 as f64
}

/// Ties to even; every other value as `Math.round`.
pub fn round(x: f64) -> f64 {
    let lo = x.floor();
    let d = x - lo;
    if d == 0.5 {
        lo + if lo % 2.0 == 0.0 { 0.0 } else { 1.0 }
    } else if d > 0.5 {
        lo + 1.0
    } else {
        lo
    }
}

pub fn sign(x: f64) -> f64 {
    if x > 0.0 {
        1.0
    } else if x < 0.0 {
        -1.0
    } else {
        x
    }
}

/// `Math.max` for two values: NaN wins.
pub fn max(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else if a > b {
        a
    } else {
        b
    }
}

/// `Math.min` for two values: NaN wins.
pub fn min(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else if a < b {
        a
    } else {
        b
    }
}

pub fn sum(a: &[f64]) -> f64 {
    let mut s = 0.0;
    for &x in a {
        s += x;
    }
    s
}

pub fn mean(a: &[f64]) -> f64 {
    sum(a) / a.len() as f64
}

pub fn norm(a: &[f64]) -> f64 {
    let mut s = 0.0;
    for &x in a {
        s += x * x;
    }
    s.sqrt()
}

fn sorted(a: &[f64]) -> Vec<f64> {
    let mut s = a.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    s
}

pub fn median(a: &[f64]) -> f64 {
    let s = sorted(a);
    let n = s.len();
    if n == 0 {
        f64::NAN
    } else if n % 2 == 1 {
        s[n >> 1]
    } else {
        (s[n / 2 - 1] + s[n / 2]) / 2.0
    }
}

pub fn quantile(a: &[f64], q: f64) -> f64 {
    if a.is_empty() {
        return f64::NAN;
    }
    let s = sorted(a);
    let i = (s.len() as f64 - 1.0) * q;
    let j = i.floor();
    let j0 = j as usize;
    s[j0] + (s[(j0 + 1).min(s.len() - 1)] - s[j0]) * (i - j)
}

pub fn modp(x: f64, p: f64) -> f64 {
    ((x % p) + p) % p
}

pub fn clip(x: f64, lo: f64, hi: f64) -> f64 {
    max(lo, min(hi, x))
}

/// The first maximum.
pub fn argmax(a: &[f64]) -> usize {
    let mut k = 0;
    for i in 1..a.len() {
        if a[i] > a[k] {
            k = i;
        }
    }
    k
}

pub fn lower_bound(a: &[f64], x: f64) -> usize {
    let (mut l, mut r) = (0, a.len());
    while l < r {
        let m = (l + r) >> 1;
        if a[m] < x {
            l = m + 1;
        } else {
            r = m;
        }
    }
    l
}

pub fn interp(x: f64, xp: &[f64], fp: &[f64]) -> f64 {
    let i = lower_bound(xp, x);
    if i == 0 {
        return fp[0];
    }
    if i == xp.len() {
        return fp[i - 1];
    }
    fp[i - 1] + (fp[i] - fp[i - 1]) * (x - xp[i - 1]) / (xp[i] - xp[i - 1])
}

/// Weighted least squares line: intercept and slope.
pub fn line(x: &[f64], y: &[f64], weights: Option<&[f64]>) -> (f64, f64) {
    let (mut sw, mut sx, mut sy) = (0.0, 0.0, 0.0);
    for i in 0..x.len() {
        let w = weights.map_or(1.0, |w| w[i]);
        sw += w;
        sx += w * x[i];
        sy += w * y[i];
    }
    let (mx, my) = (sx / sw, sy / sw);
    let (mut xx, mut xy) = (0.0, 0.0);
    for i in 0..x.len() {
        let w = weights.map_or(1.0, |w| w[i]);
        xx += w * ((x[i] - mx) * (x[i] - mx));
        xy += w * (x[i] - mx) * (y[i] - my);
    }
    let b = xy / xx;
    (my - b * mx, b)
}

/// `2 ** e` for an integer `e`.
pub fn pow2(e: f64) -> f64 {
    2f64.powi(e as i32)
}

/// Rounds through IEEE binary16, matching the feature cache.
pub fn half(x: f64) -> f64 {
    let s = if x < 0.0 { -1.0 } else { 1.0 };
    let a = x.abs();
    if a == 0.0 {
        return x;
    }
    if a < pow2(-14.0) {
        return s * round(a / pow2(-24.0)) * pow2(-24.0);
    }
    let e = jsmath::log2(a).floor();
    s * round(a / pow2(e - 10.0)) * pow2(e - 10.0)
}
