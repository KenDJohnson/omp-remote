use std::fmt;

use futures_channel::mpsc;
use omp_control_client::{ClientBuildError, ClientConfig, ClientEvent, ClientRunner, RequestError};
use omp_control_protocol::{ClientCapabilities, ClientDescriptor, ClientPlatform, LeaseHolderId};

use crate::AppActions;

pub fn client_descriptor() -> ClientDescriptor {
    ClientDescriptor {
        name: "OMP Remote".to_owned(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
        platform: platform_kind(),
        capabilities: ClientCapabilities::default(),
    }
}

pub fn start_client(
    config: ClientConfig,
) -> Result<(AppActions, mpsc::Receiver<ClientEvent>), StartClientError> {
    start_client_inner(config)
}

#[cfg(not(target_arch = "wasm32"))]
fn start_client_inner(
    config: ClientConfig,
) -> Result<(AppActions, mpsc::Receiver<ClientEvent>), StartClientError> {
    let (handle, runner) = ClientRunner::new(
        config,
        omp_control_client::NativeCredentialStore::new("dev.ohmypi.omp-remote"),
    )?;
    let events = handle.events()?;
    let holder = LeaseHolderId::new(format!("ui-{}", uuid::Uuid::new_v4()))
        .expect("UUID lease holder IDs are non-empty");
    drop(runner.spawn_native());
    Ok((AppActions::new(handle, holder), events))
}

#[cfg(target_arch = "wasm32")]
fn start_client_inner(
    config: ClientConfig,
) -> Result<(AppActions, mpsc::Receiver<ClientEvent>), StartClientError> {
    let (handle, runner) = ClientRunner::new(
        config,
        omp_control_client::BrowserCredentialStore::new("omp-remote:credentials:v1"),
    )?;
    let events = handle.events()?;
    let holder = LeaseHolderId::new(format!("ui-{}", uuid::Uuid::new_v4()))
        .expect("UUID lease holder IDs are non-empty");
    drop(runner.spawn_browser());
    Ok((AppActions::new(handle, holder), events))
}

pub fn initial_pairing_link() -> Option<String> {
    initial_pairing_link_inner()
}

#[cfg(target_arch = "wasm32")]
fn initial_pairing_link_inner() -> Option<String> {
    let hash = web_sys::window()?.location().hash().ok()?;
    let payload = hash.strip_prefix('#').unwrap_or(&hash);
    (!payload.is_empty()).then(|| payload.to_owned())
}

#[cfg(not(target_arch = "wasm32"))]
fn initial_pairing_link_inner() -> Option<String> {
    std::env::args().skip(1).find(|argument| {
        argument.starts_with("omp-remote://pair#")
            || argument.starts_with("https://") && argument.contains('#')
    })
}

#[cfg(target_arch = "wasm32")]
const fn platform_kind() -> ClientPlatform {
    ClientPlatform::Web
}

#[cfg(any(target_os = "android", target_os = "ios"))]
const fn platform_kind() -> ClientPlatform {
    ClientPlatform::Mobile
}

#[cfg(all(
    not(target_arch = "wasm32"),
    not(any(target_os = "android", target_os = "ios"))
))]
const fn platform_kind() -> ClientPlatform {
    ClientPlatform::Desktop
}

#[derive(Debug)]
pub enum StartClientError {
    Build(ClientBuildError),
    Events(RequestError),
}

impl fmt::Display for StartClientError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Build(error) => error.fmt(formatter),
            Self::Events(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for StartClientError {}

impl From<ClientBuildError> for StartClientError {
    fn from(error: ClientBuildError) -> Self {
        Self::Build(error)
    }
}

impl From<RequestError> for StartClientError {
    fn from(error: RequestError) -> Self {
        Self::Events(error)
    }
}
