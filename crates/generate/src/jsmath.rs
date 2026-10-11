//! The math functions of the TypeScript runtime, with V8's results.
//!
//! V8 implements `Math.exp`, `log`, `log1p`, `log2`, `sin`, `cos` and `atan2` with fdlibm, and
//! `Math.hypot` with its own scaled sum. The system libm gives other last bits for some inputs,
//! and the last bits reach the receipt, so these are ported instead. `conformance/generate/jsmath`
//! compares them with Node's `Math` bit for bit.

/// Fused multiply-add. Node on arm64 is built with clang, which fuses `a * b + c` within one
/// expression; the ports below fuse exactly the expressions it fuses.
fn fma(a: f64, b: f64, c: f64) -> f64 {
    a.mul_add(b, c)
}

fn hi(x: f64) -> i32 {
    (x.to_bits() >> 32) as i32
}

fn lo(x: f64) -> u32 {
    x.to_bits() as u32
}

fn with_hi(x: f64, high: i32) -> f64 {
    f64::from_bits(((high as u32 as u64) << 32) | (x.to_bits() & 0xffff_ffff))
}

fn with_lo(x: f64, low: u32) -> f64 {
    f64::from_bits((x.to_bits() & 0xffff_ffff_0000_0000) | low as u64)
}

fn from_words(high: i32, low: u32) -> f64 {
    f64::from_bits(((high as u32 as u64) << 32) | low as u64)
}

pub fn exp(mut x: f64) -> f64 {
    const HALF: [f64; 2] = [0.5, -0.5];
    const O_THRESHOLD: f64 = 7.09782712893383973096e+02;
    const U_THRESHOLD: f64 = -7.45133219101941108420e+02;
    const LN2_HI: [f64; 2] = [6.93147180369123816490e-01, -6.93147180369123816490e-01];
    const LN2_LO: [f64; 2] = [1.90821492927058770002e-10, -1.90821492927058770002e-10];
    const INV_LN2: f64 = 1.44269504088896338700e+00;
    const P1: f64 = 1.66666666666666019037e-01;
    const P2: f64 = -2.77777777770155933842e-03;
    const P3: f64 = 6.61375632143793436117e-05;
    const P4: f64 = -1.65339022054652515390e-06;
    const P5: f64 = 4.13813679705723846039e-08;
    const E: f64 = 2.718281828459045;
    const TWOM1000: f64 = 9.33263618503218878990e-302;
    const TWO1023: f64 = 8.988465674311579539e307;
    let (mut hi_part, mut lo_part, mut k) = (0.0, 0.0, 0i32);
    let hx0 = hi(x) as u32;
    let xsb = ((hx0 >> 31) & 1) as usize;
    let hx = hx0 & 0x7fff_ffff;
    if hx >= 0x4086_2E42 {
        if hx >= 0x7ff0_0000 {
            if ((hx & 0xfffff) | lo(x)) != 0 {
                return x + x;
            }
            return if xsb == 0 { x } else { 0.0 };
        }
        if x > O_THRESHOLD {
            return f64::INFINITY;
        }
        if x < U_THRESHOLD {
            return 0.0;
        }
    }
    if hx > 0x3fd6_2e42 {
        if hx < 0x3FF0_A2B2 {
            if x == 1.0 {
                return E;
            }
            hi_part = x - LN2_HI[xsb];
            lo_part = LN2_LO[xsb];
            k = 1 - xsb as i32 - xsb as i32;
        } else {
            k = fma(INV_LN2, x, HALF[xsb]) as i32;
            let t = k as f64;
            hi_part = fma(-t, LN2_HI[0], x);
            lo_part = t * LN2_LO[0];
        }
        x = hi_part - lo_part;
    } else if hx < 0x3e30_0000 {
        if 1.0e+300 + x > 1.0 {
            return 1.0 + x;
        }
    }
    let t = x * x;
    let twopk = if k >= -1021 { from_words(0x3ff0_0000 + (k << 20), 0) } else { from_words(0x3ff0_0000 + ((k + 1000) << 20), 0) };
    let c = fma(-t, fma(t, fma(t, fma(t, fma(t, P5, P4), P3), P2), P1), x);
    if k == 0 {
        return 1.0 - ((x * c) / (c - 2.0) - x);
    }
    let y = 1.0 - ((lo_part - (x * c) / (2.0 - c)) - hi_part);
    if k >= -1021 {
        if k == 1024 {
            return y * 2.0 * TWO1023;
        }
        y * twopk
    } else {
        y * twopk * TWOM1000
    }
}

const LN2_HI: f64 = 6.93147180369123816490e-01;
const LN2_LO: f64 = 1.90821492927058770002e-10;
const TWO54: f64 = 1.80143985094819840000e+16;
const LG1: f64 = 6.666666666666735130e-01;
const LG2: f64 = 3.999999999940941908e-01;
const LG3: f64 = 2.857142874366239149e-01;
const LG4: f64 = 2.222219843214978396e-01;
const LG5: f64 = 1.818357216161805012e-01;
const LG6: f64 = 1.531383769920937332e-01;
const LG7: f64 = 1.479819860511658591e-01;

pub fn log(mut x: f64) -> f64 {
    let mut hx = hi(x);
    let lx = lo(x);
    let mut k = 0i32;
    if hx < 0x0010_0000 {
        if ((hx & 0x7fff_ffff) as u32 | lx) == 0 {
            return f64::NEG_INFINITY;
        }
        if hx < 0 {
            return f64::NAN;
        }
        k -= 54;
        x *= TWO54;
        hx = hi(x);
    }
    if hx >= 0x7ff0_0000 {
        return x + x;
    }
    k += (hx >> 20) - 1023;
    hx &= 0x000f_ffff;
    let i = (hx + 0x95f64) & 0x10_0000;
    x = with_hi(x, hx | (i ^ 0x3ff0_0000));
    k += i >> 20;
    let f = x - 1.0;
    if (0x000f_ffff & (2 + hx)) < 3 {
        if f == 0.0 {
            if k == 0 {
                return 0.0;
            }
            let dk = k as f64;
            return fma(dk, LN2_HI, dk * LN2_LO);
        }
        let r = f * f * fma(-0.33333333333333333, f, 0.5);
        if k == 0 {
            return f - r;
        }
        let dk = k as f64;
        return fma(dk, LN2_HI, -(fma(-dk, LN2_LO, r) - f));
    }
    let s = f / (2.0 + f);
    let dk = k as f64;
    let z = s * s;
    let mut i = hx - 0x6147a;
    let w = z * z;
    let j = 0x6b851 - hx;
    let t1 = w * fma(w, fma(w, LG6, LG4), LG2);
    let t2 = z * fma(w, fma(w, fma(w, LG7, LG5), LG3), LG1);
    i |= j;
    let r = t2 + t1;
    if i > 0 {
        let hfsq = 0.5 * f * f;
        if k == 0 {
            f - fma(-s, hfsq + r, hfsq)
        } else {
            fma(dk, LN2_HI, -((hfsq - fma(s, hfsq + r, dk * LN2_LO)) - f))
        }
    } else if k == 0 {
        fma(-s, f - r, f)
    } else {
        fma(dk, LN2_HI, -((fma(s, f - r, -(dk * LN2_LO))) - f))
    }
}

pub fn log1p(x: f64) -> f64 {
    const LP: [f64; 7] = [
        6.666666666666735130e-01,
        3.999999999940941908e-01,
        2.857142874366239149e-01,
        2.222219843214978396e-01,
        1.818357216161805012e-01,
        1.531383769920937332e-01,
        1.479819860511658591e-01,
    ];
    let hx = hi(x);
    let ax = hx & 0x7fff_ffff;
    let mut k = 1i32;
    let (mut f, mut hu, mut c) = (0.0, 0i32, 0.0);
    if hx < 0x3FDA_827A {
        if ax >= 0x3ff0_0000 {
            return if x == -1.0 { f64::NEG_INFINITY } else { f64::NAN };
        }
        if ax < 0x3e20_0000 {
            if TWO54 + x > 0.0 && ax < 0x3c90_0000 {
                return x;
            }
            return fma(-(x * x), 0.5, x);
        }
        if hx > 0 || hx <= 0xbfd2_bec4u32 as i32 {
            k = 0;
            f = x;
            hu = 1;
        }
    }
    if hx >= 0x7ff0_0000 {
        return x + x;
    }
    if k != 0 {
        let mut u;
        if hx < 0x4340_0000 {
            u = 1.0 + x;
            hu = hi(u);
            k = (hu >> 20) - 1023;
            c = if k > 0 { 1.0 - (u - x) } else { x - (u - 1.0) };
            c /= u;
        } else {
            u = x;
            hu = hi(u);
            k = (hu >> 20) - 1023;
            c = 0.0;
        }
        hu &= 0x000f_ffff;
        if hu < 0x6a09e {
            u = with_hi(u, hu | 0x3ff0_0000);
        } else {
            k += 1;
            u = with_hi(u, hu | 0x3fe0_0000);
            hu = (0x0010_0000 - hu) >> 2;
        }
        f = u - 1.0;
    }
    let hfsq = 0.5 * f * f;
    let kf = k as f64;
    if hu == 0 {
        if f == 0.0 {
            if k == 0 {
                return 0.0;
            }
            c = fma(kf, LN2_LO, c);
            return fma(kf, LN2_HI, c);
        }
        let r = hfsq * fma(-0.66666666666666666, f, 1.0);
        if k == 0 {
            return f - r;
        }
        return fma(kf, LN2_HI, -((r - fma(kf, LN2_LO, c)) - f));
    }
    let s = f / (2.0 + f);
    let z = s * s;
    let r = z * fma(z, fma(z, fma(z, fma(z, fma(z, fma(z, LP[6], LP[5]), LP[4]), LP[3]), LP[2]), LP[1]), LP[0]);
    if k == 0 {
        f - fma(-s, hfsq + r, hfsq)
    } else {
        fma(kf, LN2_HI, -((hfsq - fma(s, hfsq + r, fma(kf, LN2_LO, c))) - f))
    }
}

pub fn log2(mut x: f64) -> f64 {
    const IVLN2_HI: f64 = 1.44269504072144627571e+00;
    const IVLN2_LO: f64 = 1.67517131648865118353e-10;
    let mut hx = hi(x);
    let lx = lo(x);
    let mut k = 0i32;
    if hx < 0x0010_0000 {
        if ((hx & 0x7fff_ffff) as u32 | lx) == 0 {
            return f64::NEG_INFINITY;
        }
        if hx < 0 {
            return f64::NAN;
        }
        k -= 54;
        x *= TWO54;
        hx = hi(x);
    }
    if hx >= 0x7ff0_0000 {
        return x + x;
    }
    if hx == 0x3ff0_0000 && lx == 0 {
        return 0.0;
    }
    k += (hx >> 20) - 1023;
    hx &= 0x000f_ffff;
    let i = (hx + 0x95f64) & 0x10_0000;
    x = with_hi(x, hx | (i ^ 0x3ff0_0000));
    k += i >> 20;
    let y = k as f64;
    let f = x - 1.0;
    let hfsq = 0.5 * f * f;
    let r = {
        let s = f / (2.0 + f);
        let z = s * s;
        let w = z * z;
        let t1 = w * fma(w, fma(w, LG6, LG4), LG2);
        let t2 = z * fma(w, fma(w, fma(w, LG7, LG5), LG3), LG1);
        s * (hfsq + (t2 + t1))
    };
    let hi_part = with_lo(f - hfsq, 0);
    let lo_part = (f - hi_part) - hfsq + r;
    let mut val_hi = hi_part * IVLN2_HI;
    let mut val_lo = fma(lo_part + hi_part, IVLN2_LO, lo_part * IVLN2_HI);
    let w = y + val_hi;
    val_lo += (y - w) + val_hi;
    val_hi = w;
    val_lo + val_hi
}

const PIO2_1: f64 = 1.57079632673412561417e+00;
const PIO2_1T: f64 = 6.07710050650619224932e-11;
const PIO2_2: f64 = 6.07710050630396597660e-11;
const PIO2_2T: f64 = 2.02226624879595063154e-21;
const PIO2_3: f64 = 2.02226624871116645580e-21;
const PIO2_3T: f64 = 8.47842766036889956997e-32;
const INVPIO2: f64 = 6.36619772367581382433e-01;
const NPIO2_HW: [i32; 32] = [
    0x3FF921FB, 0x400921FB, 0x4012D97C, 0x401921FB, 0x401F6A7A, 0x4022D97C, 0x4025FDBB, 0x402921FB, 0x402C463A, 0x402F6A7A, 0x4031475C, 0x4032D97C, 0x40346B9C, 0x4035FDBB, 0x40378FDB, 0x403921FB,
    0x403AB41B, 0x403C463A, 0x403DD85A, 0x403F6A7A, 0x40407E4C, 0x4041475C, 0x4042106C, 0x4042D97C, 0x4043A28C, 0x40446B9C, 0x404534AC, 0x4045FDBB, 0x4046C6CB, 0x40478FDB, 0x404858EB, 0x404921FB,
];

/// Reduces `x` below 2^19 * pi/2 to `y[0] + y[1]` in [-pi/4, pi/4] and returns the quadrant.
/// Larger arguments (never reached by the runtime) are not reduced the fdlibm way.
fn rem_pio2(x: f64) -> Option<(i32, f64, f64)> {
    let hx = hi(x);
    let ix = hx & 0x7fff_ffff;
    if ix <= 0x3fe9_21fb {
        return Some((0, x, 0.0));
    }
    if ix < 0x4002_d97c {
        if hx > 0 {
            let mut z = x - PIO2_1;
            let (y0, y1);
            if ix != 0x3ff9_21fb {
                y0 = z - PIO2_1T;
                y1 = (z - y0) - PIO2_1T;
            } else {
                z -= PIO2_2;
                y0 = z - PIO2_2T;
                y1 = (z - y0) - PIO2_2T;
            }
            return Some((1, y0, y1));
        }
        let mut z = x + PIO2_1;
        let (y0, y1);
        if ix != 0x3ff9_21fb {
            y0 = z + PIO2_1T;
            y1 = (z - y0) + PIO2_1T;
        } else {
            z += PIO2_2;
            y0 = z + PIO2_2T;
            y1 = (z - y0) + PIO2_2T;
        }
        return Some((-1, y0, y1));
    }
    if ix <= 0x4139_21fb {
        let t = x.abs();
        let n = fma(t, INVPIO2, 0.5) as i32;
        let fn_ = n as f64;
        let mut r = fma(-fn_, PIO2_1, t);
        let mut w = fn_ * PIO2_1T;
        let mut y0;
        if n < 32 && ix != NPIO2_HW[(n - 1) as usize] {
            y0 = r - w;
        } else {
            let j = ix >> 20;
            y0 = r - w;
            let mut i = j - ((hi(y0) >> 20) & 0x7ff);
            if i > 16 {
                let t = r;
                w = fn_ * PIO2_2;
                r = t - w;
                w = fma(fn_, PIO2_2T, -((t - r) - w));
                y0 = r - w;
                i = j - ((hi(y0) >> 20) & 0x7ff);
                if i > 49 {
                    let t = r;
                    w = fn_ * PIO2_3;
                    r = t - w;
                    w = fma(fn_, PIO2_3T, -((t - r) - w));
                    y0 = r - w;
                }
            }
        }
        let y1 = (r - y0) - w;
        return Some(if hx < 0 { (-n, -y0, -y1) } else { (n, y0, y1) });
    }
    None
}

fn kernel_sin(x: f64, y: f64, iy: i32) -> f64 {
    const S1: f64 = -1.66666666666666324348e-01;
    const S2: f64 = 8.33333333332248946124e-03;
    const S3: f64 = -1.98412698298579493134e-04;
    const S4: f64 = 2.75573137070700676789e-06;
    const S5: f64 = -2.50507602534068634195e-08;
    const S6: f64 = 1.58969099521155010221e-10;
    let ix = hi(x) & 0x7fff_ffff;
    if ix < 0x3e40_0000 && x as i32 == 0 {
        return x;
    }
    let z = x * x;
    let v = z * x;
    let r = fma(z, fma(z, fma(z, fma(z, S6, S5), S4), S3), S2);
    if iy == 0 {
        fma(v, fma(z, r, S1), x)
    } else {
        x - fma(-v, S1, fma(z, fma(0.5, y, -(v * r)), -y))
    }
}

fn kernel_cos(x: f64, y: f64) -> f64 {
    const C1: f64 = 4.16666666666666019037e-02;
    const C2: f64 = -1.38888888888741095749e-03;
    const C3: f64 = 2.48015872894767294178e-05;
    const C4: f64 = -2.75573143513906633035e-07;
    const C5: f64 = 2.08757232129817482790e-09;
    const C6: f64 = -1.13596475577881948265e-11;
    let ix = hi(x) & 0x7fff_ffff;
    if ix < 0x3e40_0000 && x as i32 == 0 {
        return 1.0;
    }
    let z = x * x;
    let r = z * fma(z, fma(z, fma(z, fma(z, fma(z, C6, C5), C4), C3), C2), C1);
    if ix < 0x3FD3_3333 {
        return 1.0 - fma(0.5, z, -fma(z, r, -(x * y)));
    }
    let qx = if ix > 0x3fe9_0000 { 0.28125 } else { from_words(ix - 0x0020_0000, 0) };
    let hz = fma(0.5, z, -qx);
    let a = 1.0 - qx;
    a - (hz - fma(z, r, -(x * y)))
}

pub fn sin(x: f64) -> f64 {
    let ix = hi(x) & 0x7fff_ffff;
    if ix <= 0x3fe9_21fb {
        return kernel_sin(x, 0.0, 0);
    }
    if ix >= 0x7ff0_0000 {
        return x - x;
    }
    match rem_pio2(x) {
        Some((n, y0, y1)) => match n & 3 {
            0 => kernel_sin(y0, y1, 1),
            1 => kernel_cos(y0, y1),
            2 => -kernel_sin(y0, y1, 1),
            _ => -kernel_cos(y0, y1),
        },
        None => x.sin(),
    }
}

pub fn cos(x: f64) -> f64 {
    let ix = hi(x) & 0x7fff_ffff;
    if ix <= 0x3fe9_21fb {
        return kernel_cos(x, 0.0);
    }
    if ix >= 0x7ff0_0000 {
        return x - x;
    }
    match rem_pio2(x) {
        Some((n, y0, y1)) => match n & 3 {
            0 => kernel_cos(y0, y1),
            1 => -kernel_sin(y0, y1, 1),
            2 => -kernel_cos(y0, y1),
            _ => kernel_sin(y0, y1, 1),
        },
        None => x.cos(),
    }
}

pub fn atan(x: f64) -> f64 {
    const ATANHI: [f64; 4] = [4.63647609000806093515e-01, 7.85398163397448278999e-01, 9.82793723247329054082e-01, 1.57079632679489655800e+00];
    const ATANLO: [f64; 4] = [2.26987774529616870924e-17, 3.06161699786838301793e-17, 1.39033110312309984516e-17, 6.12323399573676603587e-17];
    const AT: [f64; 11] = [
        3.33333333333329318027e-01,
        -1.99999999998764832476e-01,
        1.42857142725034663711e-01,
        -1.11111104054623557880e-01,
        9.09088713343650656196e-02,
        -7.69187620504482999495e-02,
        6.66107313738753120669e-02,
        -5.83357013379057348645e-02,
        4.97687799461593236017e-02,
        -3.65315727442169155270e-02,
        1.62858201153657823623e-02,
    ];
    let hx = hi(x);
    let ix = hx & 0x7fff_ffff;
    let id: i32;
    let mut x = x;
    if ix >= 0x4410_0000 {
        if ix > 0x7ff0_0000 || (ix == 0x7ff0_0000 && lo(x) != 0) {
            return x + x;
        }
        return if hx > 0 { ATANHI[3] + ATANLO[3] } else { -ATANHI[3] - ATANLO[3] };
    }
    if ix < 0x3fdc_0000 {
        if ix < 0x3e20_0000 && 1.0e300 + x > 1.0 {
            return x;
        }
        id = -1;
    } else {
        x = x.abs();
        if ix < 0x3ff3_0000 {
            if ix < 0x3fe6_0000 {
                id = 0;
                x = fma(2.0, x, -1.0) / (2.0 + x);
            } else {
                id = 1;
                x = (x - 1.0) / (x + 1.0);
            }
        } else if ix < 0x4003_8000 {
            id = 2;
            x = (x - 1.5) / fma(1.5, x, 1.0);
        } else {
            id = 3;
            x = -1.0 / x;
        }
    }
    let z = x * x;
    let w = z * z;
    let s1 = z * fma(w, fma(w, fma(w, fma(w, fma(w, AT[10], AT[8]), AT[6]), AT[4]), AT[2]), AT[0]);
    let s2 = w * fma(w, fma(w, fma(w, fma(w, AT[9], AT[7]), AT[5]), AT[3]), AT[1]);
    if id < 0 {
        return fma(-x, s1 + s2, x);
    }
    let id = id as usize;
    let z = ATANHI[id] - (fma(x, s1 + s2, -ATANLO[id]) - x);
    if hx < 0 { -z } else { z }
}

pub fn atan2(y: f64, x: f64) -> f64 {
    const TINY: f64 = 1.0e-300;
    const PI_O_4: f64 = 7.8539816339744827900E-01;
    const PI_O_2: f64 = 1.5707963267948965580E+00;
    const PI: f64 = 3.1415926535897931160E+00;
    const PI_LO: f64 = 1.2246467991473531772E-16;
    if x.is_nan() || y.is_nan() {
        return x + y;
    }
    let hx = hi(x);
    let ix = hx & 0x7fff_ffff;
    let lx = lo(x);
    let hy = hi(y);
    let iy = hy & 0x7fff_ffff;
    let ly = lo(y);
    if (hx.wrapping_sub(0x3ff0_0000) | lx as i32) == 0 {
        return atan(y);
    }
    let mut m = ((hy >> 31) & 1) | ((hx >> 30) & 2);
    if (iy as u32 | ly) == 0 {
        return match m {
            0 | 1 => y,
            2 => PI + TINY,
            _ => -PI - TINY,
        };
    }
    if (ix as u32 | lx) == 0 {
        return if hy < 0 { -PI_O_2 - TINY } else { PI_O_2 + TINY };
    }
    if ix == 0x7ff0_0000 {
        if iy == 0x7ff0_0000 {
            return match m {
                0 => PI_O_4 + TINY,
                1 => -PI_O_4 - TINY,
                2 => 3.0 * PI_O_4 + TINY,
                _ => -3.0 * PI_O_4 - TINY,
            };
        }
        return match m {
            0 => 0.0,
            1 => -0.0,
            2 => PI + TINY,
            _ => -PI - TINY,
        };
    }
    if iy == 0x7ff0_0000 {
        return if hy < 0 { -PI_O_2 - TINY } else { PI_O_2 + TINY };
    }
    let k = (iy - ix) >> 20;
    let z = if k > 60 {
        m &= 1;
        PI_O_2 + 0.5 * PI_LO
    } else if hx < 0 && k < -60 {
        0.0
    } else {
        atan((y / x).abs())
    };
    match m {
        0 => z,
        1 => -z,
        2 => PI - (z - PI_LO),
        _ => (z - PI_LO) - PI,
    }
}

/// V8's `Math.hypot`: the values are scaled by the largest and summed with Kahan compensation.
pub fn hypot(values: &[f64]) -> f64 {
    let mut max = 0.0f64;
    let mut nan = false;
    for &v in values {
        if v.is_infinite() {
            return f64::INFINITY;
        }
        if v.is_nan() {
            nan = true;
        }
        max = max.max(v.abs());
    }
    if nan {
        return f64::NAN;
    }
    if max == 0.0 {
        return 0.0;
    }
    let (mut sum, mut compensation) = (0.0, 0.0);
    for &v in values {
        let n = v.abs() / max;
        let summand = n * n - compensation;
        let preliminary = sum + summand;
        compensation = (preliminary - sum) - summand;
        sum = preliminary;
    }
    f64::sqrt(sum) * max
}
