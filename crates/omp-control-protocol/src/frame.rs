use std::{collections::BTreeSet, fmt, num::NonZeroU64};

use omp_rpc::{
    ExtensionUiRequestFrame, ExtensionUiResponseFrame, ImageContent, ServerMessage,
    StreamingBehavior,
};
use serde::{Deserialize, Serialize};

use crate::{
    AgentId, AgentSnapshot, ConnectionId, DeviceId, EventSequence, InteractionLease, LeaseHolderId,
    OperationId, PairingId, RequestId, RunId, ServerId, StateDelta, StateRevision,
    SubscriptionCursor,
};

pub const CAPABILITY_STATE_DELTAS: &str = "state_deltas";
pub const CAPABILITY_EVENT_REPLAY: &str = "event_replay";
pub const CAPABILITY_INTERACTION_LEASES: &str = "interaction_leases";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ProtocolVersion(pub u16);

impl ProtocolVersion {
    pub const V1: Self = Self(1);
    pub const CURRENT: Self = Self::V1;
}

pub const SUPPORTED_PROTOCOL_VERSIONS: &[ProtocolVersion] = &[ProtocolVersion::V1];

pub fn negotiate_version(
    client_versions: &[ProtocolVersion],
) -> Result<ProtocolVersion, ProtocolNegotiationError> {
    SUPPORTED_PROTOCOL_VERSIONS
        .iter()
        .rev()
        .copied()
        .find(|version| client_versions.contains(version))
        .ok_or(ProtocolNegotiationError::UnsupportedVersions)
}

pub fn negotiate_client_hello(
    frame: &ClientFrame,
) -> Result<(&ClientHello, ProtocolVersion), ProtocolNegotiationError> {
    let ClientFrame::Hello(hello) = frame else {
        return Err(ProtocolNegotiationError::ExpectedHello);
    };
    let version = negotiate_version(&hello.supported_versions)?;
    Ok((hello, version))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProtocolNegotiationError {
    UnsupportedVersions,
    ExpectedHello,
}

impl fmt::Display for ProtocolNegotiationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedVersions => formatter
                .write_str("client and server have no supported protocol version in common"),
            Self::ExpectedHello => formatter.write_str("first client frame must be hello"),
        }
    }
}

impl std::error::Error for ProtocolNegotiationError {}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum ClientFrame {
    Hello(ClientHello),
    Request(RequestEnvelope),
    Subscribe(SubscribeRequest),
    Unsubscribe(UnsubscribeRequest),
    UiResponse(UiResponseEnvelope),
    Ping(Ping),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum ServerFrame {
    Welcome(ServerWelcome),
    Response(ResponseEnvelope),
    Snapshot(StateSnapshot),
    Delta(DeltaEnvelope),
    Event(EventEnvelope),
    ReplayGap(ReplayGap),
    InteractionRequest(UiInteractionEnvelope),
    Error(ProtocolError),
    Pong(Pong),
    ServerShutdown(ServerShutdown),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientHello {
    pub supported_versions: Vec<ProtocolVersion>,
    pub client: ClientDescriptor,
    pub authentication: ClientAuthentication,
    #[serde(default)]
    pub resume: ResumeState,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientDescriptor {
    pub name: String,
    pub version: String,
    pub platform: ClientPlatform,
    #[serde(default)]
    pub capabilities: ClientCapabilities,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClientPlatform {
    Web,
    Mobile,
    Desktop,
    Cli,
    Service,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientCapabilities {
    #[serde(default)]
    pub requested: BTreeSet<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "method", rename_all = "snake_case")]
pub enum ClientAuthentication {
    Pair {
        pairing_id: PairingId,
        secret: PairingSecret,
        device: DeviceDescriptor,
    },
    Device {
        device_id: DeviceId,
        token: DeviceToken,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceDescriptor {
    pub name: String,
    pub platform: ClientPlatform,
}

macro_rules! secret_string {
    ($name:ident) => {
        #[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            #[must_use]
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            #[must_use]
            pub fn expose_secret(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(concat!(stringify!($name), "(**redacted**)"))
            }
        }
    };
}

secret_string!(PairingSecret);
secret_string!(DeviceToken);

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResumeState {
    #[serde(default)]
    pub subscriptions: Vec<SubscriptionCursor>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerWelcome {
    pub protocol_version: ProtocolVersion,
    pub server_id: ServerId,
    pub connection_id: ConnectionId,
    pub device_id: DeviceId,
    pub capabilities: ServerCapabilities,
    pub heartbeat_interval_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issued_credential: Option<DeviceCredential>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerCapabilities {
    #[serde(default)]
    pub enabled: BTreeSet<String>,
    pub max_frame_bytes: u32,
}

impl ServerCapabilities {
    #[must_use]
    pub fn negotiate(
        available: &BTreeSet<String>,
        requested: &ClientCapabilities,
        max_frame_bytes: u32,
    ) -> Self {
        Self {
            enabled: available
                .intersection(&requested.requested)
                .cloned()
                .collect(),
            max_frame_bytes,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestEnvelope {
    pub request_id: RequestId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation_id: Option<OperationId>,
    pub request: ControlRequest,
}

impl RequestEnvelope {
    pub fn validate(&self) -> Result<(), RequestValidationError> {
        if self.request.is_mutating() && self.operation_id.is_none() {
            return Err(RequestValidationError::MissingOperationId);
        }
        if !self.request.is_mutating() && self.operation_id.is_some() {
            return Err(RequestValidationError::UnexpectedOperationId);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestValidationError {
    MissingOperationId,
    UnexpectedOperationId,
}

impl fmt::Display for RequestValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingOperationId => {
                formatter.write_str("mutating requests require an operation ID")
            }
            Self::UnexpectedOperationId => {
                formatter.write_str("read-only requests cannot carry an operation ID")
            }
        }
    }
}

impl std::error::Error for RequestValidationError {}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum ControlRequest {
    ListAgents,
    GetAgent {
        agent_id: AgentId,
    },
    LaunchAgent {
        agent_id: AgentId,
    },
    StopAgent {
        agent_id: AgentId,
    },
    Prompt {
        agent_id: AgentId,
        message: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        images: Vec<ImageContent>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        streaming_behavior: Option<StreamingBehavior>,
    },
    Steer {
        agent_id: AgentId,
        message: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        images: Vec<ImageContent>,
    },
    FollowUp {
        agent_id: AgentId,
        message: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        images: Vec<ImageContent>,
    },
    Abort {
        agent_id: AgentId,
    },
    SwitchSession {
        agent_id: AgentId,
        session_path: String,
    },
    RespondToUi {
        agent_id: AgentId,
        holder: LeaseHolderId,
        response: ExtensionUiResponseFrame,
    },
    AcquireInteractionLease {
        agent_id: AgentId,
        holder: LeaseHolderId,
        ttl_ms: NonZeroU64,
    },
    ReleaseInteractionLease {
        agent_id: AgentId,
        holder: LeaseHolderId,
    },
}

impl ControlRequest {
    #[must_use]
    pub fn is_mutating(&self) -> bool {
        !matches!(self, Self::ListAgents | Self::GetAgent { .. })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResponseEnvelope {
    pub request_id: RequestId,
    pub outcome: ResponseOutcome,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", content = "value", rename_all = "snake_case")]
pub enum ResponseOutcome {
    Success(Box<ControlResponse>),
    Error(ProtocolError),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum ControlResponse {
    Accepted,
    Agents { agents: Vec<AgentSnapshot> },
    Agent { agent: Box<AgentSnapshot> },
    PromptAccepted { run_id: RunId },
    InteractionLease { lease: InteractionLease },
    InteractionReleased,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubscribeRequest {
    pub agent_id: AgentId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<SubscriptionCursor>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnsubscribeRequest {
    pub agent_id: AgentId,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UiResponseEnvelope {
    pub agent_id: AgentId,
    pub request_id: RequestId,
    pub operation_id: OperationId,
    pub holder: LeaseHolderId,
    pub response: ExtensionUiResponseFrame,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UiInteractionEnvelope {
    pub agent_id: AgentId,
    pub event_sequence: EventSequence,
    pub request: ExtensionUiRequestFrame,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StateSnapshot {
    pub agents: Vec<AgentSnapshot>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeltaEnvelope {
    pub event_sequence: EventSequence,
    pub delta: StateDelta,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventEnvelope {
    pub agent_id: AgentId,
    pub event_sequence: EventSequence,
    pub event: ServerMessage,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplayGap {
    pub agent_id: AgentId,
    pub current_revision: StateRevision,
    pub current_event_sequence: EventSequence,
    pub reason: ReplayGapReason,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReplayGapReason {
    BufferExpired,
    NonContiguousCursor,
    SlowConsumer,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolError {
    pub code: String,
    pub message: String,
    #[serde(default)]
    pub retryable: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Ping {
    pub nonce: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Pong {
    pub nonce: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerShutdown {
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reconnect_after_ms: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceScopes {
    pub observe: bool,
    pub prompt: bool,
    pub mutate_session: bool,
    pub stop_agent: bool,
    pub answer_ui: bool,
    pub administer_devices: bool,
}

impl DeviceScopes {
    #[must_use]
    pub const fn all() -> Self {
        Self {
            observe: true,
            prompt: true,
            mutate_session: true,
            stop_agent: true,
            answer_ui: true,
            administer_devices: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceCredential {
    pub server_id: ServerId,
    pub device_id: DeviceId,
    pub token: DeviceToken,
    pub scopes: DeviceScopes,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairingBundle {
    pub format_version: u16,
    pub server_id: ServerId,
    pub endpoint: String,
    pub pairing_id: PairingId,
    pub secret: PairingSecret,
    pub expires_at_ms: u64,
    pub tls_identity: TlsIdentityHint,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum TlsIdentityHint {
    PubliclyTrusted,
    InsecureDevelopment,
    Sha256Fingerprint(String),
}
