use std::{fmt, future::Future, time::Duration};

use omp_control_protocol::TlsIdentityHint;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SocketTarget {
    pub endpoint: String,
    pub tls_identity: TlsIdentityHint,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SocketEvent {
    Binary(Vec<u8>),
    Closed { reason: Option<String> },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransportError {
    message: String,
}

impl TransportError {
    #[must_use]
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for TransportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for TransportError {}

pub trait BinaryWebSocket {
    type SendFuture<'a>: Future<Output = Result<(), TransportError>>
    where
        Self: 'a;
    type ReceiveFuture<'a>: Future<Output = Result<SocketEvent, TransportError>>
    where
        Self: 'a;
    type CloseFuture<'a>: Future<Output = Result<(), TransportError>>
    where
        Self: 'a;

    fn send_binary(&mut self, bytes: Vec<u8>) -> Self::SendFuture<'_>;
    fn receive(&mut self) -> Self::ReceiveFuture<'_>;
    fn close(&mut self) -> Self::CloseFuture<'_>;
}

pub trait WebSocketAdapter {
    type Socket: BinaryWebSocket;
    type ConnectFuture<'a>: Future<Output = Result<Self::Socket, TransportError>>
    where
        Self: 'a;
    type SleepFuture<'a>: Future<Output = ()>
    where
        Self: 'a;

    fn connect<'a>(&'a self, target: &'a SocketTarget) -> Self::ConnectFuture<'a>;
    fn sleep(&self, duration: Duration) -> Self::SleepFuture<'_>;
}
