use std::{fmt, num::NonZeroU32};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use omp_control_protocol::{CborCodec, CborCodecError, PairingBundle};

pub const MAX_PAIRING_PAYLOAD_BYTES: u32 = 64 * 1_024;
const MAX_PAIRING_PAYLOAD_TEXT_BYTES: usize = (MAX_PAIRING_PAYLOAD_BYTES as usize).div_ceil(3) * 4;

pub fn decode_pairing_link(value: &str) -> Result<PairingBundle, PairingLinkError> {
    let value = value.trim();
    let payload = value
        .rsplit_once('#')
        .map_or(value, |(_, fragment)| fragment);
    if payload.is_empty() {
        return Err(PairingLinkError::MissingPayload);
    }
    if payload.len() > MAX_PAIRING_PAYLOAD_TEXT_BYTES {
        return Err(PairingLinkError::PayloadTooLarge);
    }
    let bytes = URL_SAFE_NO_PAD
        .decode(payload)
        .map_err(PairingLinkError::InvalidBase64)?;
    CborCodec::new(
        NonZeroU32::new(MAX_PAIRING_PAYLOAD_BYTES).expect("the pairing payload limit is non-zero"),
    )
    .decode(&bytes)
    .map_err(PairingLinkError::InvalidCbor)
}

#[derive(Debug)]
pub enum PairingLinkError {
    MissingPayload,
    PayloadTooLarge,
    InvalidBase64(base64::DecodeError),
    InvalidCbor(CborCodecError),
}

impl fmt::Display for PairingLinkError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingPayload => formatter.write_str("pairing link has no payload"),
            Self::PayloadTooLarge => formatter.write_str("pairing payload exceeds 64 KiB"),
            Self::InvalidBase64(_) => formatter.write_str("pairing payload is not valid base64url"),
            Self::InvalidCbor(_) => formatter.write_str("pairing payload is not valid CBOR"),
        }
    }
}

impl std::error::Error for PairingLinkError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidBase64(error) => Some(error),
            Self::InvalidCbor(error) => Some(error),
            Self::MissingPayload | Self::PayloadTooLarge => None,
        }
    }
}
