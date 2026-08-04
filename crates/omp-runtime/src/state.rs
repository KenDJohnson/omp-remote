use std::sync::Arc;

use omp_rpc::{RequestId, ServerMessage};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeExit {
    pub code: Option<i32>,
    pub success: bool,
    pub forced: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RuntimeStatus {
    Running { process_id: u32 },
    Exited(RuntimeExit),
    Failed(Arc<str>),
}

#[derive(Clone, Debug, PartialEq)]
pub enum RuntimeEvent {
    Frame(ServerMessage),
    Stderr(Arc<str>),
    Exited(RuntimeExit),
    Failed(Arc<str>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PromptCompletion {
    Local,
    Agent,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PromptPhase {
    Submitted,
    Running,
    Completed(PromptCompletion),
    Failed(Arc<str>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PromptStatus {
    pub request_id: RequestId,
    pub phase: PromptPhase,
}
