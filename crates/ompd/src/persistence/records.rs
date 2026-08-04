use omp_control_protocol::{
    AgentId, AgentLifecycle, ClientPlatform, DeviceId, DeviceScopes, EventSequence, OperationId,
    PairingId, PairingSecret, ResponseOutcome, RunId, ServerId, StateRevision,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentRecord {
    pub agent_id: AgentId,
    pub lifecycle: AgentLifecycle,
    pub process_id: Option<u32>,
    pub active_run_id: Option<RunId>,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceRecord {
    pub device_id: DeviceId,
    pub name: String,
    pub platform: ClientPlatform,
    pub scopes: DeviceScopes,
    pub created_at_ms: u64,
    pub last_seen_at_ms: Option<u64>,
    pub revoked_at_ms: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PairingGrant {
    pub pairing_id: PairingId,
    pub secret: PairingSecret,
    pub expires_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PairingRecord {
    pub pairing_id: PairingId,
    pub requested_name: String,
    pub scopes: DeviceScopes,
    pub created_at_ms: u64,
    pub expires_at_ms: u64,
    pub consumed_at_ms: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionResumeRecord {
    pub agent_id: AgentId,
    pub session_id: String,
    pub session_file: String,
    pub revision: StateRevision,
    pub event_sequence: EventSequence,
    pub updated_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub enum OperationClaim {
    Execute,
    InProgress,
    Completed(ResponseOutcome),
    Indeterminate,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OperationKey {
    pub device_id: DeviceId,
    pub operation_id: OperationId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StableServerIdentity {
    pub server_id: ServerId,
}
