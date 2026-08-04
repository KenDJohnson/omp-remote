use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

use omp_control_protocol::{
    AgentId, AgentSnapshot, DeltaApplyError, EventEnvelope, EventSequence, ServerFrame,
    StateSnapshot, SubscriptionCursor, UiInteractionEnvelope,
};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ReplicatedState {
    agents: BTreeMap<AgentId, AgentSnapshot>,
    resync_required: BTreeSet<AgentId>,
}

impl ReplicatedState {
    #[must_use]
    pub fn agents(&self) -> &BTreeMap<AgentId, AgentSnapshot> {
        &self.agents
    }

    #[must_use]
    pub fn agent(&self, agent_id: &AgentId) -> Option<&AgentSnapshot> {
        self.agents.get(agent_id)
    }

    #[must_use]
    pub fn cursor(&self, agent_id: &AgentId) -> Option<SubscriptionCursor> {
        if self.resync_required.contains(agent_id) {
            return None;
        }
        self.agents
            .get(agent_id)
            .map(|snapshot| SubscriptionCursor {
                agent_id: snapshot.agent_id.clone(),
                revision: snapshot.revision,
                event_sequence: snapshot.event_sequence,
            })
    }

    #[must_use]
    pub fn is_resync_required(&self, agent_id: &AgentId) -> bool {
        self.resync_required.contains(agent_id)
    }

    pub fn apply(
        &mut self,
        frame: ServerFrame,
    ) -> Result<Vec<ReplicationEffect>, ReplicationError> {
        match frame {
            ServerFrame::Snapshot(snapshot) => Ok(self.apply_snapshot(snapshot)),
            ServerFrame::Delta(envelope) => {
                let agent_id = envelope.delta.agent_id.clone();
                let result = (|| {
                    let snapshot = self
                        .agents
                        .get_mut(&agent_id)
                        .ok_or_else(|| ReplicationError::MissingSnapshot(agent_id.clone()))?;
                    if self.resync_required.contains(&agent_id) {
                        return Err(ReplicationError::ResyncPending(agent_id.clone()));
                    }
                    require_next_sequence(snapshot.event_sequence, envelope.event_sequence)?;
                    snapshot
                        .apply_delta(envelope.delta, envelope.event_sequence)
                        .map_err(ReplicationError::Delta)
                })();
                if let Err(error) = result {
                    self.resync_required.insert(agent_id);
                    return Err(error);
                }
                Ok(vec![ReplicationEffect::StateChanged(agent_id)])
            }
            ServerFrame::Event(envelope) => self.apply_event(envelope),
            ServerFrame::InteractionRequest(envelope) => self.apply_interaction(envelope),
            ServerFrame::ReplayGap(gap) => {
                self.resync_required.insert(gap.agent_id.clone());
                Ok(vec![ReplicationEffect::ResyncRequired(gap.agent_id)])
            }
            _ => Ok(Vec::new()),
        }
    }

    fn apply_snapshot(&mut self, snapshot: StateSnapshot) -> Vec<ReplicationEffect> {
        let mut effects = Vec::with_capacity(snapshot.agents.len());
        for agent in snapshot.agents {
            let agent_id = agent.agent_id.clone();
            self.resync_required.remove(&agent_id);
            self.agents.insert(agent_id.clone(), agent);
            effects.push(ReplicationEffect::StateChanged(agent_id));
        }
        effects
    }

    fn apply_event(
        &mut self,
        envelope: EventEnvelope,
    ) -> Result<Vec<ReplicationEffect>, ReplicationError> {
        let agent_id = envelope.agent_id.clone();
        let result = (|| {
            let snapshot = self
                .agents
                .get_mut(&agent_id)
                .ok_or_else(|| ReplicationError::MissingSnapshot(agent_id.clone()))?;
            if self.resync_required.contains(&agent_id) {
                return Err(ReplicationError::ResyncPending(agent_id.clone()));
            }
            require_next_sequence(snapshot.event_sequence, envelope.event_sequence)?;
            snapshot.event_sequence = envelope.event_sequence;
            Ok(())
        })();
        if let Err(error) = result {
            self.resync_required.insert(agent_id);
            return Err(error);
        }
        Ok(vec![ReplicationEffect::Event(envelope)])
    }

    fn apply_interaction(
        &mut self,
        envelope: UiInteractionEnvelope,
    ) -> Result<Vec<ReplicationEffect>, ReplicationError> {
        let agent_id = envelope.agent_id.clone();
        let result = (|| {
            let snapshot = self
                .agents
                .get_mut(&agent_id)
                .ok_or_else(|| ReplicationError::MissingSnapshot(agent_id.clone()))?;
            if self.resync_required.contains(&agent_id) {
                return Err(ReplicationError::ResyncPending(agent_id.clone()));
            }
            require_next_sequence(snapshot.event_sequence, envelope.event_sequence)?;
            snapshot.event_sequence = envelope.event_sequence;
            Ok(())
        })();
        if let Err(error) = result {
            self.resync_required.insert(agent_id);
            return Err(error);
        }
        Ok(vec![ReplicationEffect::Interaction(envelope)])
    }
}

fn require_next_sequence(
    current: EventSequence,
    incoming: EventSequence,
) -> Result<(), ReplicationError> {
    let expected = current
        .0
        .checked_add(1)
        .ok_or(ReplicationError::EventSequenceExhausted)?;
    if incoming.0 != expected {
        return Err(ReplicationError::NonContiguousEventSequence {
            local: current,
            incoming,
        });
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq)]
pub enum ReplicationEffect {
    StateChanged(AgentId),
    Event(EventEnvelope),
    Interaction(UiInteractionEnvelope),
    ResyncRequired(AgentId),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReplicationError {
    MissingSnapshot(AgentId),
    ResyncPending(AgentId),
    NonContiguousEventSequence {
        local: EventSequence,
        incoming: EventSequence,
    },
    EventSequenceExhausted,
    Delta(DeltaApplyError),
}

impl ReplicationError {
    #[must_use]
    pub fn agent_id(&self) -> Option<&AgentId> {
        match self {
            Self::MissingSnapshot(agent_id) | Self::ResyncPending(agent_id) => Some(agent_id),
            Self::Delta(DeltaApplyError::WrongAgent { actual, .. }) => Some(actual),
            Self::NonContiguousEventSequence { .. }
            | Self::EventSequenceExhausted
            | Self::Delta(_) => None,
        }
    }
}

impl fmt::Display for ReplicationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingSnapshot(agent_id) => {
                write!(
                    formatter,
                    "received an update for {agent_id} before its snapshot"
                )
            }
            Self::ResyncPending(agent_id) => {
                write!(formatter, "updates for {agent_id} require a fresh snapshot")
            }
            Self::NonContiguousEventSequence { local, incoming } => write!(
                formatter,
                "event sequence {} does not immediately follow local sequence {}",
                incoming.0, local.0
            ),
            Self::EventSequenceExhausted => {
                formatter.write_str("event sequence cannot advance beyond u64::MAX")
            }
            Self::Delta(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for ReplicationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Delta(error) => Some(error),
            _ => None,
        }
    }
}
