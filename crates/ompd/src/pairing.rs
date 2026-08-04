use std::{
    fmt,
    num::{NonZeroU32, NonZeroU64},
};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use omp_control_protocol::{CborCodec, DeviceScopes, PairingBundle};
use qrcode::{QrCode, render::unicode::Dense1x2};

use crate::{
    persistence::{Store, StoreError},
    transport::{TransportConfig, TransportConfigError},
};

const PAIRING_BUNDLE_VERSION: u16 = 1;
const PAIRING_BUNDLE_LIMIT: u32 = 64 * 1_024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PairingOutput {
    pub bundle: PairingBundle,
    pub native_link: String,
    pub browser_link: String,
    pub terminal_qr: String,
}

impl PairingOutput {
    #[must_use]
    pub fn human_output(&self) -> String {
        format!(
            "{}\nNative app: {}\nBrowser: {}\nExpires at: {} ms UTC\n",
            self.terminal_qr, self.native_link, self.browser_link, self.bundle.expires_at_ms
        )
    }
}

pub fn create_pairing_output(
    store: &Store,
    transport: &TransportConfig,
    requested_name: impl Into<String>,
    scopes: DeviceScopes,
    now_ms: u64,
    ttl_ms: NonZeroU64,
) -> Result<PairingOutput, PairingOutputError> {
    transport.validate()?;
    let grant = store.create_pairing(requested_name, scopes, now_ms, ttl_ms)?;
    let bundle = PairingBundle {
        format_version: PAIRING_BUNDLE_VERSION,
        server_id: store.server_id().clone(),
        endpoint: transport.public_endpoint.clone(),
        pairing_id: grant.pairing_id,
        secret: grant.secret,
        expires_at_ms: grant.expires_at_ms,
        tls_identity: transport.tls_identity_hint()?,
    };
    let payload = pairing_codec().encode(&bundle)?;
    let payload = URL_SAFE_NO_PAD.encode(payload);
    let native_link = format!("omp-remote://pair#{payload}");
    let browser_link = format!(
        "{}#{}",
        browser_pair_url(&transport.public_endpoint)?,
        payload
    );
    let terminal_qr = QrCode::new(native_link.as_bytes())?
        .render::<Dense1x2>()
        .quiet_zone(true)
        .build();
    Ok(PairingOutput {
        bundle,
        native_link,
        browser_link,
        terminal_qr,
    })
}

fn pairing_codec() -> CborCodec {
    CborCodec::new(
        NonZeroU32::new(PAIRING_BUNDLE_LIMIT).expect("the pairing bundle limit is non-zero"),
    )
}

fn browser_pair_url(endpoint: &str) -> Result<String, PairingOutputError> {
    let origin = endpoint
        .strip_suffix("/control")
        .ok_or(PairingOutputError::InvalidEndpoint)?;
    if let Some(origin) = origin.strip_prefix("wss://") {
        Ok(format!("https://{origin}/pair"))
    } else if let Some(origin) = origin.strip_prefix("ws://") {
        Ok(format!("http://{origin}/pair"))
    } else {
        Err(PairingOutputError::InvalidEndpoint)
    }
}

#[derive(Debug)]
pub enum PairingOutputError {
    Store(StoreError),
    Transport(TransportConfigError),
    Codec(omp_control_protocol::CborCodecError),
    Qr(qrcode::types::QrError),
    InvalidEndpoint,
}

impl fmt::Display for PairingOutputError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Store(error) => error.fmt(formatter),
            Self::Transport(error) => error.fmt(formatter),
            Self::Codec(error) => error.fmt(formatter),
            Self::Qr(error) => write!(formatter, "failed to render pairing QR code: {error}"),
            Self::InvalidEndpoint => formatter.write_str("invalid control endpoint for pairing"),
        }
    }
}

impl std::error::Error for PairingOutputError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Store(error) => Some(error),
            Self::Transport(error) => Some(error),
            Self::Codec(error) => Some(error),
            Self::Qr(error) => Some(error),
            _ => None,
        }
    }
}

impl From<StoreError> for PairingOutputError {
    fn from(error: StoreError) -> Self {
        Self::Store(error)
    }
}

impl From<TransportConfigError> for PairingOutputError {
    fn from(error: TransportConfigError) -> Self {
        Self::Transport(error)
    }
}

impl From<omp_control_protocol::CborCodecError> for PairingOutputError {
    fn from(error: omp_control_protocol::CborCodecError) -> Self {
        Self::Codec(error)
    }
}

impl From<qrcode::types::QrError> for PairingOutputError {
    fn from(error: qrcode::types::QrError) -> Self {
        Self::Qr(error)
    }
}
