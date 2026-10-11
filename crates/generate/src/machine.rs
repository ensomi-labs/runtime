//! The generator as a state machine without IO.
//!
//! The pipeline is written as an async function that awaits [`Io::run`] wherever the TypeScript
//! runtime awaits a graph. [`Generator`] polls it with no runtime and no waker: the future suspends
//! at each request, and the caller runs the graph and resumes with the outputs.
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

    pub fn as_f32(&self) -> &[f32] {
        match &self.data {
            Data::F32(v) => v,
            _ => panic!("tensor is not float32"),
        }
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

pub enum Step {
    /// Run `graph` on `inputs` and pass the outputs to the next call.
    Run { graph: String, inputs: Tensors },
    /// The generator entered a phase; call again with no outputs.
    Progress(Phase),
    Done(Box<crate::generate::GenerationResult>),
}

enum Request {
    Run(String, Tensors),
    Progress(Phase),
}

#[derive(Default)]
struct Mailbox {
    request: Option<Request>,
    response: Option<Tensors>,
}

#[derive(Clone, Default)]
pub struct Io(Rc<RefCell<Mailbox>>);

struct Suspend {
    io: Io,
    request: Option<Request>,
    /// Whether to wait for graph outputs; a progress step resumes on the next call.
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
}

/// The outputs of one graph run.
pub struct Outputs(Tensors);

impl Outputs {
    pub fn take(&mut self, name: &str) -> Result<Tensor, String> {
        let at = self.0.iter().position(|(n, _)| n == name).ok_or_else(|| format!("graph output {name} is missing"))?;
        Ok(self.0.swap_remove(at).1)
    }

    pub fn take_f32(&mut self, name: &str) -> Result<Vec<f32>, String> {
        match self.take(name)?.data {
            Data::F32(v) => Ok(v),
            _ => Err(format!("graph output {name} is not float32")),
        }
    }
}

pub struct Generator {
    future: Pin<Box<dyn Future<Output = Result<crate::generate::GenerationResult, String>>>>,
    io: Io,
}

impl Generator {
    pub fn new(io: Io, future: impl Future<Output = Result<crate::generate::GenerationResult, String>> + 'static) -> Self {
        Self { future: Box::pin(future), io }
    }

    /// Advances to the next step. Pass the outputs of the graph that the previous step asked for,
    /// and nothing after a progress step or on the first call.
    pub fn step(&mut self, outputs: Option<Tensors>) -> Result<Step, String> {
        self.io.0.borrow_mut().response = outputs;
        let mut cx = Context::from_waker(Waker::noop());
        match self.future.as_mut().poll(&mut cx) {
            Poll::Ready(result) => result.map(|r| Step::Done(Box::new(r))),
            Poll::Pending => match self.io.0.borrow_mut().request.take() {
                Some(Request::Run(graph, inputs)) => Ok(Step::Run { graph, inputs }),
                Some(Request::Progress(phase)) => Ok(Step::Progress(phase)),
                None => Err("the generator was resumed without the outputs it asked for".to_string()),
            },
        }
    }
}
