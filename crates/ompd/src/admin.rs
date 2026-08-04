use std::{
    fmt,
    num::{NonZeroU32, NonZeroU64},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::Arc,
};

use omp_control_protocol::{CborCodec, DeviceScopes};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{UnixListener, UnixStream},
};

use crate::{create_pairing_output, persistence::Store, transport::TransportConfig};

const ADMIN_FRAME_LIMIT: u32 = 64 * 1_024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
enum AdminRequest {
    Pair {
        name: String,
        expires_ms: NonZeroU64,
        scopes: DeviceScopes,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum AdminResponse {
    Pair {
        native_link: String,
        browser_link: String,
        terminal_qr: String,
        expires_at_ms: u64,
    },
    Error {
        message: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PairingLinks {
    pub native_link: String,
    pub browser_link: String,
    pub terminal_qr: String,
    pub expires_at_ms: u64,
}

impl PairingLinks {
    #[must_use]
    pub fn human_output(&self) -> String {
        format!(
            "{}\nNative app: {}\nBrowser: {}\nExpires at: {} ms UTC\n",
            self.terminal_qr, self.native_link, self.browser_link, self.expires_at_ms
        )
    }
}

pub async fn serve_admin_socket(
    path: impl Into<PathBuf>,
    store: Arc<Mutex<Store>>,
    transport: TransportConfig,
) -> Result<(), AdminError> {
    let path = path.into();
    let listener = UnixListener::bind(&path).map_err(|error| AdminError::Bind {
        path: path.clone(),
        error,
    })?;
    let _socket_guard = AdminSocketGuard(path.clone());
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).map_err(|error| {
        AdminError::Permissions {
            path: path.clone(),
            error,
        }
    })?;
    loop {
        let (stream, _) = listener.accept().await?;
        let store = Arc::clone(&store);
        let transport = transport.clone();
        tokio::spawn(async move {
            let _ = handle_admin_connection(stream, store, transport).await;
        });
    }
}

pub async fn request_pairing(
    path: impl AsRef<Path>,
    name: impl Into<String>,
    expires_ms: NonZeroU64,
    scopes: DeviceScopes,
) -> Result<PairingLinks, AdminError> {
    let mut stream =
        UnixStream::connect(path.as_ref())
            .await
            .map_err(|error| AdminError::Connect {
                path: path.as_ref().to_owned(),
                error,
            })?;
    write_frame(
        &mut stream,
        &AdminRequest::Pair {
            name: name.into(),
            expires_ms,
            scopes,
        },
    )
    .await?;
    match read_frame::<AdminResponse>(&mut stream).await? {
        AdminResponse::Pair {
            native_link,
            browser_link,
            terminal_qr,
            expires_at_ms,
        } => Ok(PairingLinks {
            native_link,
            browser_link,
            terminal_qr,
            expires_at_ms,
        }),
        AdminResponse::Error { message } => Err(AdminError::Rejected(message)),
    }
}

async fn handle_admin_connection(
    mut stream: UnixStream,
    store: Arc<Mutex<Store>>,
    transport: TransportConfig,
) -> Result<(), AdminError> {
    let request = read_frame::<AdminRequest>(&mut stream).await?;
    let response = match request {
        AdminRequest::Pair {
            name,
            expires_ms,
            scopes,
        } => {
            let result = store
                .lock()
                .create_pairing_output(&transport, name, scopes, expires_ms);
            match result {
                Ok(output) => AdminResponse::Pair {
                    native_link: output.native_link,
                    browser_link: output.browser_link,
                    terminal_qr: output.terminal_qr,
                    expires_at_ms: output.bundle.expires_at_ms,
                },
                Err(error) => AdminResponse::Error {
                    message: error.to_string(),
                },
            }
        }
    };
    write_frame(&mut stream, &response).await
}

trait StorePairingExt {
    fn create_pairing_output(
        &self,
        transport: &TransportConfig,
        name: String,
        scopes: DeviceScopes,
        expires_ms: NonZeroU64,
    ) -> Result<crate::PairingOutput, crate::PairingOutputError>;
}

impl StorePairingExt for Store {
    fn create_pairing_output(
        &self,
        transport: &TransportConfig,
        name: String,
        scopes: DeviceScopes,
        expires_ms: NonZeroU64,
    ) -> Result<crate::PairingOutput, crate::PairingOutputError> {
        create_pairing_output(
            self,
            transport,
            name,
            scopes,
            crate::unix_time_ms(),
            expires_ms,
        )
    }
}

async fn write_frame<T: Serialize>(stream: &mut UnixStream, value: &T) -> Result<(), AdminError> {
    let bytes = codec().encode(value)?;
    let length = u32::try_from(bytes.len()).map_err(|_| AdminError::FrameTooLarge)?;
    stream.write_u32(length).await?;
    stream.write_all(&bytes).await?;
    stream.flush().await?;
    Ok(())
}

async fn read_frame<T: serde::de::DeserializeOwned>(
    stream: &mut UnixStream,
) -> Result<T, AdminError> {
    let length = stream.read_u32().await?;
    if length > ADMIN_FRAME_LIMIT {
        return Err(AdminError::FrameTooLarge);
    }
    let mut bytes = vec![0_u8; length as usize];
    stream.read_exact(&mut bytes).await?;
    codec().decode(&bytes).map_err(AdminError::Codec)
}

fn codec() -> CborCodec {
    CborCodec::new(NonZeroU32::new(ADMIN_FRAME_LIMIT).expect("the admin frame limit is non-zero"))
}

struct AdminSocketGuard(PathBuf);

impl Drop for AdminSocketGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
#[derive(Debug)]
pub enum AdminError {
    Bind {
        path: PathBuf,
        error: std::io::Error,
    },
    Permissions {
        path: PathBuf,
        error: std::io::Error,
    },
    Connect {
        path: PathBuf,
        error: std::io::Error,
    },
    Io(std::io::Error),
    Codec(omp_control_protocol::CborCodecError),
    FrameTooLarge,
    Rejected(String),
}

impl fmt::Display for AdminError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bind { path, error } => write!(
                formatter,
                "cannot bind local admin socket {}: {error}; remove a stale socket or stop the running daemon",
                path.display()
            ),
            Self::Permissions { path, error } => write!(
                formatter,
                "cannot restrict local admin socket {} to its owner: {error}",
                path.display()
            ),
            Self::Connect { path, error } => write!(
                formatter,
                "cannot connect to daemon admin socket {}: {error}",
                path.display()
            ),
            Self::Io(error) => error.fmt(formatter),
            Self::Codec(error) => error.fmt(formatter),
            Self::FrameTooLarge => formatter.write_str("local admin frame exceeds 64 KiB"),
            Self::Rejected(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for AdminError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Bind { error, .. }
            | Self::Permissions { error, .. }
            | Self::Connect { error, .. }
            | Self::Io(error) => Some(error),
            Self::Codec(error) => Some(error),
            Self::FrameTooLarge | Self::Rejected(_) => None,
        }
    }
}

impl From<std::io::Error> for AdminError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<omp_control_protocol::CborCodecError> for AdminError {
    fn from(error: omp_control_protocol::CborCodecError) -> Self {
        Self::Codec(error)
    }
}
