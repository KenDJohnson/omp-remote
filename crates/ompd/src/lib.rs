#![forbid(unsafe_code)]
#![doc = "Secure OMP daemon orchestration and persistence."]

mod admin;
mod controller;
mod pairing;
mod server;
mod transport;

pub use admin::{AdminError, PairingLinks, request_pairing, serve_admin_socket};
pub use controller::{ControllerError, DaemonController, unix_time_ms};
pub use pairing::{PairingOutput, PairingOutputError, create_pairing_output};
pub mod persistence;
pub use server::{DaemonServer, ServerError, ServerSessionConfig};
pub use transport::{TlsMode, TransportConfig, TransportConfigError, load_rustls_config};
