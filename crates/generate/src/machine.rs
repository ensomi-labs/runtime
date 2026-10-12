//! The generator as a state machine without IO.
//!
//! The pipeline is written as an async function that awaits [`Io::run`] wherever the TypeScript
//! runtime awaits a graph. [`Generator`] polls it with no runtime and no waker: the future suspends
//! at each request, and the caller runs the graph and resumes with the outputs. Work that needs no
//! graph outputs can be left in [`Io::background`], for a caller that runs graphs asynchronously to
//! do with [`Generator::work`] while a graph runs.
use crate::export::Page;
use crate::generate::GenerationResult;
use std::cell::RefCell;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll, Waker};

#[derive(Clone, Debug, PartialEq)]
pub enum Data {
    F32(Vec<f32>),
    I64(Vec<i64>),
    /// One byte per element, 0 or 1.
    Bool(Vec<u8>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Tensor {
    pub shape: Vec<usize>,
    pub data: Data,
}

impl Tensor {
    pub fn f32(shape: &[usize], data: Vec<f32>) -> Self {
        Self { shape: shape.to_vec(), data: Data::F32(data) }
    }

    pub fn i64(shape: &[usize], data: Vec<i64>) -> Self {
        Self { shape: shape.to_vec(), data: Data::I64(data) }
    }

    pub fn bool(shape: &[usize], data: Vec<u8>) -> Self {
        Self { shape: shape.to_vec(), data: Data::Bool(data) }
    }
}

/// Named tensors, as the graphs name them.
pub type Tensors = Vec<(String, Tensor)>;

/// The generation phases of EnsomiProtocol's `GenerationProgress`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Loading = 1,
    Timing = 2,
    Rows = 3,
    Arrangement = 4,
    Export = 5,
}

/// A step of a generator whose result is a `T`: a chart, or the [`crate::Prepared`] audio stages.
pub enum Step<T = GenerationResult> {
    /// Run `graph` on `inputs` and pass the outputs to the next call.
    Run { graph: String, inputs: Tensors },
    /// The generator entered a phase; call again with no outputs.
    Progress(Phase),
    /// Part of the chart is final (a generation that commits per head row); call again with no outputs.
    Commit(Page),
    Done(Box<T>),
}

enum Request {
    Run(String, Tensors),
    Progress(Phase),
    Commit(Page),
}

#[derive(Default)]
struct Mailbox {
    request: Option<Request>,
    response: Option<Tensors>,
    /// Does a slice of the background work and returns whether any is left.
    background: Option<Box<dyn FnMut() -> bool>>,
}

#[derive(Clone, Default)]
pub struct Io(Rc<RefCell<Mailbox>>);

struct Suspend {
    io: Io,
    request: Option<Request>,
    /// Whether to wait for graph outputs; a progress or commit step resumes on the next call.
    wait: bool,
}

impl Future for Suspend {
    type Output = Tensors;

    fn poll(mut self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Tensors> {
        if let Some(request) = self.request.take() {
            self.io.0.borrow_mut().request = Some(request);
            return Poll::Pending;
        }
        if !self.wait {
            return Poll::Ready(Vec::new());
        }
        match self.io.0.borrow_mut().response.take() {
            Some(response) => Poll::Ready(response),
            None => Poll::Pending,
        }
    }
}

impl Io {
    pub async fn run(&self, graph: &str, inputs: Vec<(&str, Tensor)>) -> Result<Outputs, String> {
        let inputs = inputs.into_iter().map(|(name, tensor)| (name.to_string(), tensor)).collect();
        let outputs = Suspend { io: self.clone(), request: Some(Request::Run(graph.to_string(), inputs)), wait: true }.await;
        Ok(Outputs(outputs))
    }

    pub async fn progress(&self, phase: Phase) {
        Suspend { io: self.clone(), request: Some(Request::Progress(phase)), wait: false }.await;
    }

    pub async fn commit(&self, page: Page) {
        Suspend { io: self.clone(), request: Some(Request::Commit(page)), wait: false }.await;
    }

    /// Leaves work that needs no graph outputs for [`Generator::work`]. `slice` does a part of it
    /// and returns whether any is left. The pipeline finishes what is left after [`Io::end_background`].
    pub fn background(&self, slice: impl FnMut() -> bool + 'static) {
        self.0.borrow_mut().background = Some(Box::new(slice));
    }

    pub fn end_background(&self) {
        self.0.borrow_mut().background = None;
    }
}

/// The outputs of one graph run.
pub struct Outputs(Tensors);

impl Outputs {
    pub fn take(&mut self, name: &str) -> Result<Tensor, String> {
        let at = self.0.iter().position(|(n, _)| n == name).ok_or_else(|| format!("graph output {name} is missing"))?;
        Ok(self.0.swap_remove(at).1)
    }

    /// A float32 output of `len` values. The pipeline indexes these by the shapes it expects, so a graph
    /// that does not match its `runtime.json` fails here rather than out of bounds.
    pub fn take_f32(&mut self, name: &str, len: usize) -> Result<Vec<f32>, String> {
        match self.take(name)?.data {
            Data::F32(v) if v.len() == len => Ok(v),
            Data::F32(v) => Err(format!("graph output {name} has {} values where the runtime expects {len}", v.len())),
            _ => Err(format!("graph output {name} is not float32")),
        }
    }
}

pub struct Generator<T = GenerationResult> {
    future: Pin<Box<dyn Future<Output = Result<T, String>>>>,
    io: Io,
}

impl<T> Generator<T> {
    pub fn new(io: Io, future: impl Future<Output = Result<T, String>> + 'static) -> Self {
        Self { future: Box::pin(future), io }
    }

    /// Advances to the next step. Pass the outputs of the graph that the previous step asked for,
    /// and nothing after a progress or commit step or on the first call. Not to be called after `Done` or an error.
    pub fn step(&mut self, outputs: Option<Tensors>) -> Result<Step<T>, String> {
        self.io.0.borrow_mut().response = outputs;
        let mut cx = Context::from_waker(Waker::noop());
        match self.future.as_mut().poll(&mut cx) {
            Poll::Ready(result) => result.map(|r| Step::Done(Box::new(r))),
            Poll::Pending => match self.io.0.borrow_mut().request.take() {
                Some(Request::Run(graph, inputs)) => Ok(Step::Run { graph, inputs }),
                Some(Request::Progress(phase)) => Ok(Step::Progress(phase)),
                Some(Request::Commit(page)) => Ok(Step::Commit(page)),
                None => Err("the generator was resumed without the outputs it asked for".to_string()),
            },
        }
    }

    /// Does a slice of the work that needs no graph outputs, such as the Log-Mel spectrogram while
    /// BeatThis runs, and returns whether any is left. Call it after a [`Step::Run`], until the graph's
    /// outputs are ready or it returns false, then pass the outputs to [`Generator::step`] as usual. A
    /// caller that never calls it gets the same results, with that work done after the graph.
    pub fn work(&mut self) -> bool {
        let slice = self.io.0.borrow_mut().background.take();
        let Some(mut slice) = slice else { return false };
        let more = slice();
        if more {
            self.io.0.borrow_mut().background = Some(slice);
        }
        more
    }
}
