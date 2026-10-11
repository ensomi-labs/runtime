//! Compares `jsmath` with Node's `Math` on the cases written by `conformance/generate/jsmath_cases.mjs`.
use ensomi_generate::jsmath;

fn main() {
    let path = std::env::args().nth(1).expect("cases file");
    let bytes = std::fs::read(path).unwrap();
    let names = ["exp", "log", "log1p", "log2", "sin", "cos", "atan2", "hypot"];
    let (mut total, mut wrong, mut wrong_std, mut wrong_libm) = ([0u64; 8], [0u64; 8], [0u64; 8], [0u64; 8]);
    let mut shown = [0; 8];
    for record in bytes.chunks_exact(32) {
        let v: Vec<f64> = record.chunks_exact(8).map(|c| f64::from_le_bytes(c.try_into().unwrap())).collect();
        let (op, a, b, expected) = (v[0] as usize, v[1], v[2], v[3]);
        let actual = match op {
            0 => jsmath::exp(a),
            1 => jsmath::log(a),
            2 => jsmath::log1p(a),
            3 => jsmath::log2(a),
            4 => jsmath::sin(a),
            5 => jsmath::cos(a),
            6 => jsmath::atan2(a, b),
            _ => jsmath::hypot(&[a, b]),
        };
        total[op] += 1;
        let std_value = match op {
            0 => a.exp(),
            1 => a.ln(),
            2 => a.ln_1p(),
            3 => a.log2(),
            4 => a.sin(),
            5 => a.cos(),
            6 => a.atan2(b),
            _ => a.hypot(b),
        };
        let libm_value = match op {
            0 => libm::exp(a),
            1 => libm::log(a),
            2 => libm::log1p(a),
            3 => libm::log2(a),
            4 => libm::sin(a),
            5 => libm::cos(a),
            6 => libm::atan2(a, b),
            _ => libm::hypot(a, b),
        };
        let differs = |x: f64| x.to_bits() != expected.to_bits() && !(x.is_nan() && expected.is_nan());
        wrong_std[op] += differs(std_value) as u64;
        wrong_libm[op] += differs(libm_value) as u64;
        if actual.to_bits() != expected.to_bits() && !(actual.is_nan() && expected.is_nan()) {
            wrong[op] += 1;
            if shown[op] < 3 {
                shown[op] += 1;
                println!("{} differs at ({a:e}, {b:e}): ported {actual:e} node {expected:e}", names[op]);
            }
        }
    }
    for op in 0..8 {
        println!("{:6} {} cases, ported differs {}, std differs {}, libm crate differs {}", names[op], total[op], wrong[op], wrong_std[op], wrong_libm[op]);
    }
}
