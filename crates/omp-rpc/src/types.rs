use std::{fmt, num::NonZeroU64};

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use serde_json::{Map, Value};
use serde_repr::{Deserialize_repr, Serialize_repr};

pub type JsonObject = Map<String, Value>;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextUsage {
    pub tokens: u64,
    pub context_window: u64,
    pub percent: f64,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RequestId(String);

impl RequestId {
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    #[must_use]
    pub fn into_inner(self) -> String {
        self.0
    }
}

impl fmt::Display for RequestId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl From<String> for RequestId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for RequestId {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl AsRef<str> for RequestId {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize_repr, Deserialize_repr)]
#[repr(u8)]
pub enum ProtocolVersion {
    V1 = 1,
    V2 = 2,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ProtocolV2;

impl Serialize for ProtocolV2 {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_u8(ProtocolVersion::V2 as u8)
    }
}

impl<'de> Deserialize<'de> for ProtocolV2 {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = u8::deserialize(deserializer)?;
        if value == ProtocolVersion::V2 as u8 {
            Ok(Self)
        } else {
            Err(de::Error::invalid_value(
                de::Unexpected::Unsigned(u64::from(value)),
                &"OMP RPC protocol version 2",
            ))
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StreamingBehavior {
    Steer,
    FollowUp,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum QueueMode {
    All,
    OneAtATime,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InterruptMode {
    Immediate,
    Wait,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SubagentSubscriptionLevel {
    Off,
    Progress,
    Events,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentSource {
    Bundled,
    User,
    Project,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StructuredSubagentSchemaMode {
    Permissive,
    Strict,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageAttribution {
    User,
    Agent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ToolLoadMode {
    Essential,
    Discoverable,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadyFrame {
    max_frame_bytes: NonZeroU64,
    max_reassembled_frame_bytes: NonZeroU64,
    advertises_capabilities: bool,
}

impl ReadyFrame {
    pub const DEFAULT_MAX_FRAME_BYTES: u64 = 1_048_576;
    pub const DEFAULT_MAX_REASSEMBLED_FRAME_BYTES: u64 = 67_108_864;

    pub fn new(
        max_frame_bytes: NonZeroU64,
        max_reassembled_frame_bytes: NonZeroU64,
    ) -> Result<Self, ReadyFrameError> {
        if max_reassembled_frame_bytes < max_frame_bytes {
            return Err(ReadyFrameError::ReassemblyLimitBelowFrameLimit);
        }
        Ok(Self {
            max_frame_bytes,
            max_reassembled_frame_bytes,
            advertises_capabilities: true,
        })
    }

    #[must_use]
    pub fn legacy() -> Self {
        Self {
            max_frame_bytes: NonZeroU64::new(Self::DEFAULT_MAX_FRAME_BYTES)
                .expect("the protocol frame limit is non-zero"),
            max_reassembled_frame_bytes: NonZeroU64::new(Self::DEFAULT_MAX_REASSEMBLED_FRAME_BYTES)
                .expect("the protocol reassembly limit is non-zero"),
            advertises_capabilities: false,
        }
    }

    #[must_use]
    pub fn advertises_capabilities(&self) -> bool {
        self.advertises_capabilities
    }

    #[must_use]
    pub fn max_frame_bytes(&self) -> NonZeroU64 {
        self.max_frame_bytes
    }

    #[must_use]
    pub fn max_reassembled_frame_bytes(&self) -> NonZeroU64 {
        self.max_reassembled_frame_bytes
    }
}

impl Default for ReadyFrame {
    fn default() -> Self {
        Self {
            max_frame_bytes: NonZeroU64::new(Self::DEFAULT_MAX_FRAME_BYTES)
                .expect("the protocol frame limit is non-zero"),
            max_reassembled_frame_bytes: NonZeroU64::new(Self::DEFAULT_MAX_REASSEMBLED_FRAME_BYTES)
                .expect("the protocol reassembly limit is non-zero"),
            advertises_capabilities: true,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadyFrameError {
    ReassemblyLimitBelowFrameLimit,
}

impl fmt::Display for ReadyFrameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ReassemblyLimitBelowFrameLimit => {
                formatter.write_str("RPC reassembly limit is smaller than the frame limit")
            }
        }
    }
}

impl std::error::Error for ReadyFrameError {}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ReadyFrameRef<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    protocol_version: Option<ProtocolVersion>,
    #[serde(skip_serializing_if = "Option::is_none")]
    supported_protocol_versions: Option<[ProtocolVersion; 2]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_frame_bytes: Option<&'a NonZeroU64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_reassembled_frame_bytes: Option<&'a NonZeroU64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReadyFrameWire {
    #[serde(default)]
    protocol_version: Option<ProtocolVersion>,
    #[serde(default)]
    supported_protocol_versions: Option<[ProtocolVersion; 2]>,
    #[serde(default)]
    max_frame_bytes: Option<NonZeroU64>,
    #[serde(default)]
    max_reassembled_frame_bytes: Option<NonZeroU64>,
}

impl Serialize for ReadyFrame {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let advertised = self.advertises_capabilities;
        ReadyFrameRef {
            protocol_version: advertised.then_some(ProtocolVersion::V1),
            supported_protocol_versions: advertised
                .then_some([ProtocolVersion::V1, ProtocolVersion::V2]),
            max_frame_bytes: advertised.then_some(&self.max_frame_bytes),
            max_reassembled_frame_bytes: advertised.then_some(&self.max_reassembled_frame_bytes),
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ReadyFrame {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = ReadyFrameWire::deserialize(deserializer)?;
        match (
            wire.protocol_version,
            wire.supported_protocol_versions,
            wire.max_frame_bytes,
            wire.max_reassembled_frame_bytes,
        ) {
            (None, None, None, None) => Ok(Self::legacy()),
            (
                Some(ProtocolVersion::V1),
                Some([ProtocolVersion::V1, ProtocolVersion::V2]),
                Some(max_frame_bytes),
                Some(max_reassembled_frame_bytes),
            ) => Self::new(max_frame_bytes, max_reassembled_frame_bytes).map_err(de::Error::custom),
            (Some(protocol_version), _, _, _) if protocol_version != ProtocolVersion::V1 => {
                Err(de::Error::custom("ready frame protocolVersion must be 1"))
            }
            (_, Some(supported), _, _)
                if supported != [ProtocolVersion::V1, ProtocolVersion::V2] =>
            {
                Err(de::Error::custom(
                    "ready frame supportedProtocolVersions must be [1, 2]",
                ))
            }
            _ => Err(de::Error::custom(
                "ready frame capabilities must be either complete or absent",
            )),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChunkFrame {
    chunk_id: String,
    index: u32,
    count: NonZeroU64,
    byte_length: NonZeroU64,
    data: String,
}

impl ChunkFrame {
    pub const MAX_CHUNK_ID_CHARS: usize = 128;
    pub const MAX_COUNT: u64 = 256;
    pub const MIN_BYTE_LENGTH: u64 = ReadyFrame::DEFAULT_MAX_FRAME_BYTES;
    pub const MAX_BYTE_LENGTH: u64 = ReadyFrame::DEFAULT_MAX_REASSEMBLED_FRAME_BYTES;
    pub const MAX_PAYLOAD_BYTES: usize = 256 * 1024;

    pub fn new(
        chunk_id: impl Into<String>,
        index: u32,
        count: NonZeroU64,
        byte_length: NonZeroU64,
        data: impl Into<String>,
    ) -> Result<Self, ChunkFrameError> {
        if count.get() < 2 || count.get() > Self::MAX_COUNT {
            return Err(ChunkFrameError::InvalidCount);
        }
        if u64::from(index) >= count.get() {
            return Err(ChunkFrameError::IndexOutOfBounds);
        }
        let chunk_id = chunk_id.into();
        if chunk_id.is_empty() {
            return Err(ChunkFrameError::EmptyChunkId);
        }
        if chunk_id.chars().count() > Self::MAX_CHUNK_ID_CHARS {
            return Err(ChunkFrameError::ChunkIdTooLong);
        }
        if !(Self::MIN_BYTE_LENGTH..=Self::MAX_BYTE_LENGTH).contains(&byte_length.get()) {
            return Err(ChunkFrameError::InvalidByteLength);
        }
        let data = data.into();
        if data.is_empty() {
            return Err(ChunkFrameError::EmptyData);
        }
        let decoded = BASE64
            .decode(&data)
            .map_err(|_| ChunkFrameError::InvalidData)?;
        if decoded.len() > Self::MAX_PAYLOAD_BYTES {
            return Err(ChunkFrameError::PayloadTooLarge);
        }
        if BASE64.encode(decoded) != data {
            return Err(ChunkFrameError::InvalidData);
        }
        Ok(Self {
            chunk_id,
            index,
            count,
            byte_length,
            data,
        })
    }

    #[must_use]
    pub fn chunk_id(&self) -> &str {
        &self.chunk_id
    }

    #[must_use]
    pub fn index(&self) -> u32 {
        self.index
    }

    #[must_use]
    pub fn count(&self) -> NonZeroU64 {
        self.count
    }

    #[must_use]
    pub fn byte_length(&self) -> NonZeroU64 {
        self.byte_length
    }

    #[must_use]
    pub fn data(&self) -> &str {
        &self.data
    }

    pub(crate) fn append_decoded_data(&self, output: &mut Vec<u8>) {
        BASE64
            .decode_vec(self.data.as_bytes(), output)
            .expect("validated RPC chunk data remains valid base64");
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChunkFrameError {
    EmptyChunkId,
    ChunkIdTooLong,
    InvalidCount,
    IndexOutOfBounds,
    InvalidByteLength,
    EmptyData,
    InvalidData,
    PayloadTooLarge,
}

impl fmt::Display for ChunkFrameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyChunkId => formatter.write_str("RPC chunk ID cannot be empty"),
            Self::ChunkIdTooLong => formatter.write_str("RPC chunk ID exceeds 128 characters"),
            Self::InvalidCount => formatter.write_str("RPC chunk count must be between 2 and 256"),
            Self::IndexOutOfBounds => formatter.write_str("RPC chunk index must be below count"),
            Self::InvalidByteLength => {
                formatter.write_str("RPC chunk byte length must be between 1 MiB and 64 MiB")
            }
            Self::EmptyData => formatter.write_str("RPC chunk data cannot be empty"),
            Self::InvalidData => formatter.write_str("RPC chunk data must be canonical base64"),
            Self::PayloadTooLarge => formatter.write_str("RPC chunk payload exceeds 256 KiB"),
        }
    }
}

impl std::error::Error for ChunkFrameError {}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChunkFrameWire {
    chunk_id: String,
    index: u32,
    count: NonZeroU64,
    byte_length: NonZeroU64,
    data: String,
}

impl<'de> Deserialize<'de> for ChunkFrame {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = ChunkFrameWire::deserialize(deserializer)?;
        Self::new(
            wire.chunk_id,
            wire.index,
            wire.count,
            wire.byte_length,
            wire.data,
        )
        .map_err(de::Error::custom)
    }
}
