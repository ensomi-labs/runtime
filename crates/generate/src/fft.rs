use crate::jsmath;
use std::f64::consts::PI;

struct Stage {
    n: usize,
    r: usize,
    cos: Vec<f64>,
    sin: Vec<f64>,
    wcos: Vec<f64>,
    wsin: Vec<f64>,
}

/// Mixed-radix Cooley-Tukey FFT (the release uses lengths 1024 and 960), iterative and in place, with
/// the twiddles and summation order of the recursive decimation-in-time form.
pub struct Fft {
    n: usize,
    pub re: Vec<f64>,
    pub im: Vec<f64>,
    order: Vec<usize>,
    stages: Vec<Stage>,
    rr: Vec<f64>,
    ii: Vec<f64>,
}

fn radix(size: usize) -> usize {
    let mut r = 2;
    while size % r != 0 && r < size {
        r += 1;
    }
    r
}

fn fill(order: &mut [usize], size: usize, offset: usize, stride: usize, at: usize) {
    if size == 1 {
        order[at] = offset;
        return;
    }
    let r = radix(size);
    let m = size / r;
    for j in 0..r {
        fill(order, m, offset + j * stride, stride * r, at + j * m);
    }
}

impl Fft {
    pub fn new(n: usize) -> Self {
        let mut order = vec![0; n];
        fill(&mut order, n, 0, 1, 0);
        let mut stages = Vec::new();
        let mut size = n;
        while size > 1 {
            let r = radix(size);
            let m = size / r;
            let cos: Vec<f64> = (0..size).map(|k| jsmath::cos(-2.0 * PI * k as f64 / size as f64)).collect();
            let sin: Vec<f64> = (0..size).map(|k| jsmath::sin(-2.0 * PI * k as f64 / size as f64)).collect();
            let (mut wcos, mut wsin) = (Vec::new(), Vec::new());
            if r > 2 {
                for k in 0..m {
                    for l in 0..r {
                        for j in 0..r {
                            let t = j * (k + l * m) % size;
                            wcos.push(cos[t]);
                            wsin.push(sin[t]);
                        }
                    }
                }
            }
            stages.insert(0, Stage { n: size, r, cos, sin, wcos, wsin });
            size /= r;
        }
        let max_radix = stages.iter().map(|s| s.r).max().unwrap_or(1);
        Self { n, re: vec![0.0; n], im: vec![0.0; n], order, stages, rr: vec![0.0; max_radix], ii: vec![0.0; max_radix] }
    }

    pub fn run(&mut self, input: &[f64]) {
        let n_total = self.n;
        for i in 0..n_total {
            self.re[i] = input[self.order[i]];
            self.im[i] = 0.0;
        }
        for stage in &self.stages {
            let (n, r) = (stage.n, stage.r);
            let m = n / r;
            let mut base = 0;
            while base < n_total {
                if r == 2 {
                    // The j = 0 term has twiddle (1, -0): it contributes 0 + x exactly.
                    for k in 0..m {
                        let (p, q) = (base + k, base + k + m);
                        let (a, b, c, d) = (0.0 + self.re[p], 0.0 + self.im[p], self.re[q], self.im[q]);
                        self.re[p] = a + (c * stage.cos[k] - d * stage.sin[k]);
                        self.im[p] = b + (c * stage.sin[k] + d * stage.cos[k]);
                        self.re[q] = a + (c * stage.cos[k + m] - d * stage.sin[k + m]);
                        self.im[q] = b + (c * stage.sin[k + m] + d * stage.cos[k + m]);
                    }
                } else {
                    let mut w = 0;
                    for k in 0..m {
                        for j in 0..r {
                            self.rr[j] = self.re[base + j * m + k];
                            self.ii[j] = self.im[base + j * m + k];
                        }
                        for l in 0..r {
                            let (mut ar, mut ai) = (0.0, 0.0);
                            for j in 0..r {
                                ar += self.rr[j] * stage.wcos[w] - self.ii[j] * stage.wsin[w];
                                ai += self.rr[j] * stage.wsin[w] + self.ii[j] * stage.wcos[w];
                                w += 1;
                            }
                            self.re[base + k + l * m] = ar;
                            self.im[base + k + l * m] = ai;
                        }
                    }
                }
                base += n;
            }
        }
    }
}
