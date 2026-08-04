#![forbid(unsafe_code)]
#![doc = "Supervised ownership of one `omp --mode rpc` child process."]

mod config;
mod error;
mod runtime;
mod state;

pub use config::RuntimeConfig;
pub use error::{RequestError, RuntimeSpawnError};
pub use runtime::OmpRuntime;
pub use state::{
    PromptCompletion, PromptPhase, PromptStatus, RuntimeEvent, RuntimeExit, RuntimeStatus,
};
