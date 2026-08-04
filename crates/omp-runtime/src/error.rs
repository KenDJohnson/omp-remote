use std::{fmt, io, path::PathBuf, sync::Arc};

use omp_rpc::RequestId;

#[derive(Debug)]
pub enum RuntimeSpawnError {
    Spawn { program: PathBuf, source: io::Error },
    MissingPipe(&'static str),
    StartupTimedOut,
    StartupEof,
    StartupIo(io::Error),
    StartupProtocol(Arc<str>),
    ExpectedReadyFrame,
    ProtocolNegotiation(Arc<str>),
}

impl fmt::Display for RuntimeSpawnError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Spawn { program, .. } => {
                write!(formatter, "failed to spawn {}", program.display())
            }
            Self::MissingPipe(pipe) => write!(formatter, "spawned OMP process has no {pipe} pipe"),
            Self::StartupTimedOut => formatter.write_str("OMP RPC startup timed out"),
            Self::StartupEof => {
                formatter.write_str("OMP RPC process exited before its ready frame")
            }
            Self::StartupIo(_) => {
                formatter.write_str("failed to communicate with OMP during startup")
            }
            Self::StartupProtocol(message) => {
                write!(formatter, "invalid OMP startup frame: {message}")
            }
            Self::ExpectedReadyFrame => formatter.write_str("first OMP RPC frame was not ready"),
            Self::ProtocolNegotiation(message) => {
                write!(
                    formatter,
                    "OMP RPC protocol-v2 negotiation failed: {message}"
                )
            }
        }
    }
}

impl std::error::Error for RuntimeSpawnError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Spawn { source, .. } | Self::StartupIo(source) => Some(source),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RequestError {
    RuntimeStopped,
    TimedOut { request_id: RequestId },
    FrameTooLarge { bytes: usize, limit: usize },
    Transport(Arc<str>),
}

impl fmt::Display for RequestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RuntimeStopped => formatter.write_str("OMP runtime is not running"),
            Self::TimedOut { request_id } => write!(formatter, "request {request_id} timed out"),
            Self::FrameTooLarge { bytes, limit } => {
                write!(
                    formatter,
                    "RPC command is {bytes} bytes; limit is {limit} bytes"
                )
            }
            Self::Transport(message) => write!(formatter, "OMP transport failed: {message}"),
        }
    }
}

impl std::error::Error for RequestError {}
