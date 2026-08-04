use std::fmt;

use omp_rpc::{AvailableSlashCommand, ServerMessage};
use serde::{Deserialize, Serialize};

macro_rules! string_identifier {
    ($name:ident, $label:literal) => {
        #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, IdentifierError> {
                let value = value.into();
                if value.is_empty() {
                    return Err(IdentifierError($label));
                }
                Ok(Self(value))
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

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                self.as_str()
            }
        }
    };
}

string_identifier!(AgentId, "agent ID");
string_identifier!(RunId, "run ID");
string_identifier!(LeaseHolderId, "lease holder ID");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IdentifierError(&'static str);

impl fmt::Display for IdentifierError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} cannot be empty", self.0)
    }
}

impl std::error::Error for IdentifierError {}

#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct StateRevision(pub u64);

#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct EventSequence(pub u64);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum AgentLifecycle {
    Starting,
    Idle,
    Running,
    Stopping,
    Stopped,
    Interrupted,
    Failed { reason: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum RunLifecycle {
    Queued,
    Running,
    Completed,
    Aborted,
    Interrupted,
    Failed { reason: String },
}

impl RunLifecycle {
    #[must_use]
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Aborted | Self::Interrupted | Self::Failed { .. }
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSummary {
    pub session_id: String,
    pub session_file: Option<String>,
    pub name: Option<String>,
    pub message_count: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunSnapshot {
    pub run_id: RunId,
    pub lifecycle: RunLifecycle,
    pub started_at_ms: u64,
    pub ended_at_ms: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InteractionLease {
    pub holder: LeaseHolderId,
    pub expires_at_ms: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum InteractionState {
    #[default]
    Unclaimed,
    Leased {
        lease: InteractionLease,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSnapshot {
    pub agent_id: AgentId,
    pub revision: StateRevision,
    pub event_sequence: EventSequence,
    pub lifecycle: AgentLifecycle,
    pub session: Option<SessionSummary>,
    pub active_run: Option<RunSnapshot>,
    pub recent_runs: Vec<RunSnapshot>,
    pub interaction: InteractionState,
    pub available_commands: Vec<AvailableSlashCommand>,
}

impl AgentSnapshot {
    #[must_use]
    pub fn initial(agent_id: AgentId) -> Self {
        Self {
            agent_id,
            revision: StateRevision::default(),
            event_sequence: EventSequence::default(),
            lifecycle: AgentLifecycle::Starting,
            session: None,
            active_run: None,
            recent_runs: Vec::new(),
            interaction: InteractionState::Unclaimed,
            available_commands: Vec::new(),
        }
    }

    pub fn apply_update(&mut self, update: AgentUpdate) -> Result<(), DeltaApplyError> {
        match update {
            AgentUpdate::Delta {
                event_sequence,
                delta,
            } => self.apply_delta(delta, event_sequence),
            AgentUpdate::Event {
                agent_id,
                event_sequence,
                ..
            } => {
                self.validate_update_cursor(&agent_id, event_sequence)?;
                self.event_sequence = event_sequence;
                Ok(())
            }
        }
    }

    pub fn apply_delta(
        &mut self,
        delta: StateDelta,
        event_sequence: EventSequence,
    ) -> Result<(), DeltaApplyError> {
        self.validate_update_cursor(&delta.agent_id, event_sequence)?;
        if delta.base_revision != self.revision {
            return Err(DeltaApplyError::RevisionMismatch {
                local: self.revision,
                base: delta.base_revision,
            });
        }
        if delta.revision.0 != delta.base_revision.0.saturating_add(1) {
            return Err(DeltaApplyError::NonConsecutiveRevision {
                base: delta.base_revision,
                revision: delta.revision,
            });
        }

        apply_change(self, delta.change);
        self.revision = delta.revision;
        self.event_sequence = event_sequence;
        Ok(())
    }

    fn validate_update_cursor(
        &self,
        agent_id: &AgentId,
        event_sequence: EventSequence,
    ) -> Result<(), DeltaApplyError> {
        if *agent_id != self.agent_id {
            return Err(DeltaApplyError::WrongAgent {
                expected: self.agent_id.clone(),
                actual: agent_id.clone(),
            });
        }
        if event_sequence <= self.event_sequence {
            return Err(DeltaApplyError::StaleEventSequence {
                local: self.event_sequence,
                incoming: event_sequence,
            });
        }
        Ok(())
    }
}

pub(crate) fn apply_change(snapshot: &mut AgentSnapshot, change: AgentStateChange) {
    match change {
        AgentStateChange::LifecycleChanged(lifecycle) => snapshot.lifecycle = lifecycle,
        AgentStateChange::SessionChanged(session) => snapshot.session = Some(session),
        AgentStateChange::SessionCleared => snapshot.session = None,
        AgentStateChange::RunUpserted(run) => {
            snapshot
                .recent_runs
                .retain(|candidate| candidate.run_id != run.run_id);
            if run.lifecycle.is_terminal() {
                if snapshot
                    .active_run
                    .as_ref()
                    .is_some_and(|active| active.run_id == run.run_id)
                {
                    snapshot.active_run = None;
                }
                snapshot.recent_runs.push(run);
            } else {
                snapshot.active_run = Some(run);
            }
        }
        AgentStateChange::RunRemoved(run_id) => {
            if snapshot
                .active_run
                .as_ref()
                .is_some_and(|active| active.run_id == run_id)
            {
                snapshot.active_run = None;
            }
            snapshot
                .recent_runs
                .retain(|candidate| candidate.run_id != run_id);
        }
        AgentStateChange::InteractionChanged(interaction) => {
            snapshot.interaction = interaction;
        }
        AgentStateChange::AvailableCommandsChanged(commands) => {
            snapshot.available_commands = commands;
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StateDelta {
    pub agent_id: AgentId,
    pub base_revision: StateRevision,
    pub revision: StateRevision,
    pub change: AgentStateChange,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "change", content = "value", rename_all = "snake_case")]
pub enum AgentStateChange {
    LifecycleChanged(AgentLifecycle),
    SessionChanged(SessionSummary),
    SessionCleared,
    RunUpserted(RunSnapshot),
    RunRemoved(RunId),
    InteractionChanged(InteractionState),
    AvailableCommandsChanged(Vec<AvailableSlashCommand>),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AgentUpdate {
    Delta {
        event_sequence: EventSequence,
        delta: StateDelta,
    },
    Event {
        agent_id: AgentId,
        event_sequence: EventSequence,
        event: ServerMessage,
    },
}

impl AgentUpdate {
    #[must_use]
    pub fn event_sequence(&self) -> EventSequence {
        match self {
            Self::Delta { event_sequence, .. } | Self::Event { event_sequence, .. } => {
                *event_sequence
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubscriptionCursor {
    pub agent_id: AgentId,
    pub revision: StateRevision,
    pub event_sequence: EventSequence,
}

#[derive(Clone, Debug, PartialEq)]
pub enum SubscriptionStart {
    Snapshot(AgentSnapshot),
    Replay(Vec<AgentUpdate>),
    ResyncRequired(AgentSnapshot),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeltaApplyError {
    WrongAgent {
        expected: AgentId,
        actual: AgentId,
    },
    RevisionMismatch {
        local: StateRevision,
        base: StateRevision,
    },
    NonConsecutiveRevision {
        base: StateRevision,
        revision: StateRevision,
    },
    StaleEventSequence {
        local: EventSequence,
        incoming: EventSequence,
    },
}

impl fmt::Display for DeltaApplyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongAgent { expected, actual } => {
                write!(
                    formatter,
                    "delta is for agent {actual}, expected {expected}"
                )
            }
            Self::RevisionMismatch { local, base } => write!(
                formatter,
                "delta base revision {} does not match local revision {}",
                base.0, local.0
            ),
            Self::NonConsecutiveRevision { base, revision } => write!(
                formatter,
                "delta revision {} does not immediately follow base {}",
                revision.0, base.0
            ),
            Self::StaleEventSequence { local, incoming } => write!(
                formatter,
                "event sequence {} does not advance local sequence {}",
                incoming.0, local.0
            ),
        }
    }
}

impl std::error::Error for DeltaApplyError {}
