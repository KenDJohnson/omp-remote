use std::{
    fmt,
    io::{self, Cursor, Write},
    num::NonZeroU32,
};

use serde::{Serialize, de::DeserializeOwned};

pub const DEFAULT_PRE_AUTH_FRAME_BYTES: u32 = 16 * 1_024;
pub const DEFAULT_POST_AUTH_FRAME_BYTES: u32 = 1_024 * 1_024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameLimits {
    pre_auth: NonZeroU32,
    post_auth: NonZeroU32,
}

impl FrameLimits {
    pub fn new(pre_auth: NonZeroU32, post_auth: NonZeroU32) -> Result<Self, FrameLimitError> {
        if pre_auth > post_auth {
            return Err(FrameLimitError::PreAuthExceedsPostAuth);
        }
        Ok(Self {
            pre_auth,
            post_auth,
        })
    }

    #[must_use]
    pub fn pre_auth(self) -> NonZeroU32 {
        self.pre_auth
    }

    #[must_use]
    pub fn post_auth(self) -> NonZeroU32 {
        self.post_auth
    }

    #[must_use]
    pub fn codec(self, phase: ConnectionPhase) -> CborCodec {
        let limit = match phase {
            ConnectionPhase::PreAuth => self.pre_auth,
            ConnectionPhase::Authenticated => self.post_auth,
        };
        CborCodec::new(limit)
    }
}

impl Default for FrameLimits {
    fn default() -> Self {
        Self::new(
            NonZeroU32::new(DEFAULT_PRE_AUTH_FRAME_BYTES)
                .expect("the default pre-auth frame limit is non-zero"),
            NonZeroU32::new(DEFAULT_POST_AUTH_FRAME_BYTES)
                .expect("the default post-auth frame limit is non-zero"),
        )
        .expect("default frame limits are ordered")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectionPhase {
    PreAuth,
    Authenticated,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameLimitError {
    PreAuthExceedsPostAuth,
}

impl fmt::Display for FrameLimitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("pre-auth frame limit cannot exceed post-auth frame limit")
    }
}

impl std::error::Error for FrameLimitError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CborCodec {
    max_frame_bytes: NonZeroU32,
}

impl CborCodec {
    #[must_use]
    pub fn new(max_frame_bytes: NonZeroU32) -> Self {
        Self { max_frame_bytes }
    }

    #[must_use]
    pub fn max_frame_bytes(self) -> NonZeroU32 {
        self.max_frame_bytes
    }

    pub fn encode<T>(&self, value: &T) -> Result<Vec<u8>, CborCodecError>
    where
        T: Serialize,
    {
        let mut writer = LimitedWriter::new(self.max_frame_bytes.get() as usize);
        match ciborium::ser::into_writer(value, &mut writer) {
            Ok(()) => Ok(writer.output),
            Err(_) if writer.exceeded => Err(CborCodecError::FrameTooLarge {
                limit: self.max_frame_bytes.get(),
            }),
            Err(error) => Err(CborCodecError::Encode(error.to_string())),
        }
    }

    pub fn decode<T>(&self, bytes: &[u8]) -> Result<T, CborCodecError>
    where
        T: DeserializeOwned,
    {
        if bytes.len() > self.max_frame_bytes.get() as usize {
            return Err(CborCodecError::FrameTooLarge {
                limit: self.max_frame_bytes.get(),
            });
        }
        let mut reader = Cursor::new(bytes);
        let decoded = ciborium::de::from_reader(&mut reader)
            .map_err(|error| CborCodecError::Decode(error.to_string()))?;
        if reader.position() != bytes.len() as u64 {
            return Err(CborCodecError::TrailingData);
        }
        Ok(decoded)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CborCodecError {
    FrameTooLarge { limit: u32 },
    Encode(String),
    Decode(String),
    TrailingData,
}

impl fmt::Display for CborCodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FrameTooLarge { limit } => {
                write!(formatter, "CBOR frame exceeds the {limit}-byte limit")
            }
            Self::Encode(error) => write!(formatter, "failed to encode CBOR frame: {error}"),
            Self::Decode(error) => write!(formatter, "failed to decode CBOR frame: {error}"),
            Self::TrailingData => {
                formatter.write_str("CBOR WebSocket message contains more than one frame")
            }
        }
    }
}

impl std::error::Error for CborCodecError {}

#[derive(Debug)]
struct LimitedWriter {
    output: Vec<u8>,
    limit: usize,
    exceeded: bool,
}

impl LimitedWriter {
    fn new(limit: usize) -> Self {
        Self {
            output: Vec::new(),
            limit,
            exceeded: false,
        }
    }
}

impl Write for LimitedWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        if buffer.len() > self.limit.saturating_sub(self.output.len()) {
            self.exceeded = true;
            return Err(io::Error::new(
                io::ErrorKind::FileTooLarge,
                "CBOR frame limit exceeded",
            ));
        }
        self.output.extend_from_slice(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
