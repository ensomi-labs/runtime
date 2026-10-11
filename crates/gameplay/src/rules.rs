/// Judge difficulty. Names the half-window table in [`Windows::new`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Judge {
    A,
    B,
    C,
    D,
    E,
}

/// The gameplay ruleset: everything besides the chart and the inputs that decides judgement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ruleset {
    pub judge: Judge,
}

/// A hold's tail windows are the head windows times this.
pub const TAIL_LENIENCE: f64 = 1.5;

/// Inclusive symmetric half-windows in milliseconds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Windows {
    pub big_p: f64,
    pub p1: f64,
    pub p2: f64,
    pub p3: f64,
    pub g: f64,
}

impl Windows {
    pub fn new(judge: Judge) -> Self {
        let (big_p, p1, p2, p3) = match judge {
            Judge::A => (60.0, 105.0, 120.0, 135.0),
            Judge::B => (47.0, 63.0, 88.0, 118.0),
            Judge::C => (40.0, 56.0, 81.0, 111.0),
            Judge::D => (34.0, 50.0, 75.0, 105.0),
            Judge::E => (25.0, 50.0, 75.0, 105.0),
        };
        Self {
            big_p,
            p1,
            p2,
            p3,
            g: 200.0,
        }
    }

    /// The tier of a timing error, with every window scaled by `lenience`.
    pub fn classify(&self, error_ms: f64, lenience: f64) -> Tier {
        let error = error_ms.abs();
        if error <= self.big_p * lenience {
            Tier::BigP
        } else if error <= self.p1 * lenience {
            Tier::P1
        } else if error <= self.p2 * lenience {
            Tier::P2
        } else if error <= self.p3 * lenience {
            Tier::P3
        } else if error <= self.g * lenience {
            Tier::G
        } else {
            Tier::M
        }
    }
}

/// Malody tier, best to worst. `M` is a miss.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tier {
    BigP,
    P1,
    P2,
    P3,
    G,
    M,
}

/// What the player is shown for a tier: `BigP` is Perfect, `P1` to `G` are Good, `M` is Miss.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Judgement {
    Perfect,
    Good,
    Miss,
}

impl Tier {
    pub fn judgement(self) -> Judgement {
        match self {
            Tier::BigP => Judgement::Perfect,
            Tier::M => Judgement::Miss,
            _ => Judgement::Good,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TierCounts {
    pub big_p: u32,
    pub p1: u32,
    pub p2: u32,
    pub p3: u32,
    pub g: u32,
    pub m: u32,
}

impl TierCounts {
    pub fn judged(&self) -> u32 {
        self.big_p + self.p1 + self.p2 + self.p3 + self.g + self.m
    }

    pub fn perfect(&self) -> u32 {
        self.big_p
    }

    pub fn good(&self) -> u32 {
        self.p1 + self.p2 + self.p3 + self.g
    }

    pub fn miss(&self) -> u32 {
        self.m
    }

    pub(crate) fn increment(&mut self, tier: Tier) {
        match tier {
            Tier::BigP => self.big_p += 1,
            Tier::P1 => self.p1 += 1,
            Tier::P2 => self.p2 += 1,
            Tier::P3 => self.p3 += 1,
            Tier::G => self.g += 1,
            Tier::M => self.m += 1,
        }
    }
}

impl Ruleset {
    pub fn windows(&self) -> Windows {
        Windows::new(self.judge)
    }
}
