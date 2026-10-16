//! The Ensomi chart generator's host pipeline as a state machine without IO.
//!
//! [`Generator`] takes the parsed `runtime.json` ([`Models`]), PCM and a [`Request`], and yields
//! [`Step`]s: run a named graph on named tensors, enter a progress phase, commit a page, or the finished
//! chart, `.osu` text and receipt. The caller runs the graph and passes the outputs back. [`prepare`] runs the
//! stages that read only the audio, [`generate_prepared`] the rest; [`Scheduler`] drives both for play.
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
mod machine;
mod math;
mod models;
pub mod package;
mod prepared;
mod prior;
mod r2;
mod random;
mod rows;
mod schedule;
mod strain;
#[cfg(test)]
mod streaming_tests;
#[cfg(test)]
mod tests;

pub use audio::{resampler_files, Samples};
pub use export::{chart_tokens, Chart, ChartObject, Page, Token, TokenKind};
pub use generate::{generate, generate_prepared, prepare, Audio, Commit, Controls, GenerationResult, Options, Pattern, Receipt, Request, Section, Trace, TraceFn};
pub use grid::Grid;
pub use machine::{Data, Generator, Phase, Step, Tensor, Tensors};
pub use models::{Models, BEATTHIS_GRAPH, GRAPHS};
pub use prepared::Prepared;
pub use random::{Draw, RandomSource, ReplayRandom, SeededRandom};
pub use schedule::{Action, Event, Lead, Mode, Scheduler, Source};

/// Written into receipts and the `.osu` generator comment.
pub const RUNTIME_NAME: &str = "ensomi-runtime";
pub const RUNTIME_VERSION: &str = env!("CARGO_PKG_VERSION");
