//! The Ensomi chart generator's host pipeline as a state machine without IO.
//!
//! [`Generator`] takes the parsed `runtime.json` ([`Models`]), PCM and a [`Request`], and yields
//! [`Step`]s: run a named graph on named tensors, enter a progress phase, or the finished chart,
//! `.osu` text and receipt. The caller runs the graph and passes the outputs back.
mod arrange;
mod audio;
pub mod capabilities;
mod controls;
mod export;
mod fft;
pub mod fitter;
mod generate;
mod grid;
mod json;
pub mod jsmath;
mod machine;
mod math;
mod models;
mod prior;
mod r2;
mod random;
mod rows;
mod strain;

pub use audio::{resampler_files, Samples};
pub use export::{Chart, ChartObject};
pub use generate::{generate, Audio, Controls, GenerationResult, Options, Pattern, Receipt, Request, Section, Trace, TraceFn};
pub use grid::Grid;
pub use machine::{Data, Generator, Phase, Step, Tensor, Tensors};
pub use models::Models;
pub use random::{Draw, RandomSource, ReplayRandom, SeededRandom};

/// Written into receipts and the `.osu` generator comment.
pub const RUNTIME_NAME: &str = "ensomi-runtime";
pub const RUNTIME_VERSION: &str = env!("CARGO_PKG_VERSION");
