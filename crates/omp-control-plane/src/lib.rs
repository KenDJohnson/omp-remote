#![forbid(unsafe_code)]
#![doc = "Authoritative agent state, subscriptions, replay, and interaction leases."]

mod actor;
mod registry;
mod state;

pub use actor::{
    AgentActorConfig, AgentError, AgentHandle, AgentSubscription, InteractionLeaseError,
    SubscriptionError,
};
pub use registry::{AgentRegistry, RegistryError};
pub use state::{
    AgentId, AgentLifecycle, AgentSnapshot, AgentStateChange, AgentUpdate, DeltaApplyError,
    EventSequence, IdentifierError, InteractionLease, InteractionState, LeaseHolderId, RunId,
    RunLifecycle, RunSnapshot, SessionSummary, StateDelta, StateRevision, SubscriptionCursor,
    SubscriptionStart,
};
