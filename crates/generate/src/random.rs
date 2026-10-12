use crate::math::{argmax, f32r};
use std::cell::Cell;
use std::rc::Rc;

/// The three kinds of draw the generator makes. Names identify each draw, for replay.
pub trait RandomSource {
    fn algorithm(&self) -> &str;
    fn reset(&mut self, seed: u32);
    /// Single-sample multinomial by an exponential race; returns the index.
    fn multinomial(&mut self, probabilities: &[f32], name: &str) -> Result<usize, String>;
    /// Gumbel maximum over log probabilities; returns the index.
    fn gumbel(&mut self, logits: &[f64], name: &str) -> Result<usize, String>;
    /// Fair 0 or 1, consumed even when no release follows.
    fn orientation(&mut self, name: &str) -> Result<u32, String>;
}

/// Mulberry32; 53-bit uniforms combine two outputs. Seeds are reduced modulo 2^32.
pub struct SeededRandom {
    state: u32,
}

impl SeededRandom {
    pub fn new(seed: u32) -> Self {
        Self { state: seed }
    }

    fn uint32(&mut self) -> u32 {
        self.state = self.state.wrapping_add(0x6d2b79f5);
        let mut t = self.state;
        t = (t ^ (t >> 15)).wrapping_mul(t | 1);
        t ^= t.wrapping_add((t ^ (t >> 7)).wrapping_mul(t | 61));
        t ^ (t >> 14)
    }

    fn uniform(&mut self) -> f64 {
        let high = (self.uint32() >> 5) as f64;
        let low = (self.uint32() >> 6) as f64;
        f64::min(1.0 - f64::EPSILON / 2.0, (high * 67108864.0 + low + 0.5) / 9007199254740992.0)
    }
}

impl RandomSource for SeededRandom {
    fn algorithm(&self) -> &str {
        "mulberry32-u53-v1"
    }

    fn reset(&mut self, seed: u32) {
        self.state = seed;
    }

    fn multinomial(&mut self, p: &[f32], _name: &str) -> Result<usize, String> {
        let (mut k, mut best) = (0, f64::NEG_INFINITY);
        for i in 0..p.len() {
            let score = f32r(p[i] as f64 / f32r(-libm::log(self.uniform())));
            if i == 0 || score > best {
                k = i;
                best = score;
            }
        }
        Ok(k)
    }

    fn gumbel(&mut self, logits: &[f64], _name: &str) -> Result<usize, String> {
        let (mut k, mut best) = (0, f64::NEG_INFINITY);
        for i in 0..logits.len() {
            let score = logits[i] - libm::log(-libm::log(self.uniform()));
            if i == 0 || score > best {
                k = i;
                best = score;
            }
        }
        Ok(k)
    }

    fn orientation(&mut self, _name: &str) -> Result<u32, String> {
        Ok(self.uint32() & 1)
    }
}

/// One recorded draw: the variates and the choice they led to.
pub struct Draw {
    pub name: String,
    pub kind: String,
    pub values: Vec<f64>,
    pub chosen: usize,
}

/// Replays the variates recorded from the Python release and checks every choice against the record.
pub struct ReplayRandom {
    draws: Vec<Draw>,
    position: Rc<Cell<usize>>,
}

impl ReplayRandom {
    pub fn new(draws: Vec<Draw>) -> Self {
        Self { draws, position: Rc::new(Cell::new(0)) }
    }

    /// How many draws have been consumed; readable after the generator owns the source.
    pub fn position(&self) -> Rc<Cell<usize>> {
        self.position.clone()
    }

    pub fn recorded(&self) -> usize {
        self.draws.len()
    }

    fn draw(&mut self, name: &str, kind: &str) -> Result<&Draw, String> {
        let at = self.position.get();
        self.position.set(at + 1);
        match self.draws.get(at) {
            Some(row) if row.name == name && row.kind == kind => Ok(row),
            other => Err(format!("random replay: draw {at} wanted {name} {kind}, recorded {}", other.map_or("nothing".to_string(), |r| format!("{} {}", r.name, r.kind)))),
        }
    }

    fn check(row: &Draw, chosen: usize, scores: &[f64], at: usize) -> Result<usize, String> {
        if chosen == row.chosen {
            return Ok(chosen);
        }
        Err(format!(
            "sample: {} draw {at}: recorded {}, replay chose {chosen}; replay scores {} and {}, margin {}",
            row.name,
            row.chosen,
            scores[row.chosen],
            scores[chosen],
            scores[chosen] - scores[row.chosen]
        ))
    }
}

impl RandomSource for ReplayRandom {
    fn algorithm(&self) -> &str {
        "python-recorded-draws"
    }

    fn reset(&mut self, _seed: u32) {}

    fn multinomial(&mut self, probabilities: &[f32], name: &str) -> Result<usize, String> {
        let at = self.position.get();
        let row = self.draw(name, "exponential")?;
        let scores: Vec<f64> = probabilities.iter().zip(&row.values).map(|(&p, &v)| f32r(p as f64 / v)).collect();
        Self::check(row, argmax(&scores), &scores, at)
    }

    fn gumbel(&mut self, logits: &[f64], name: &str) -> Result<usize, String> {
        let at = self.position.get();
        let row = self.draw(name, "uniform")?;
        let scores: Vec<f64> = logits.iter().zip(&row.values).map(|(&p, &v)| p - libm::log(-libm::log(f64::max(v, 2.2250738585072014e-308)))).collect();
        Self::check(row, argmax(&scores), &scores, at)
    }

    fn orientation(&mut self, name: &str) -> Result<u32, String> {
        let at = self.position.get();
        let row = self.draw(name, "orientation")?;
        let chosen = row.values[0] as usize;
        Self::check(row, chosen, &[0.0, 0.0], at).map(|c| c as u32)
    }
}
