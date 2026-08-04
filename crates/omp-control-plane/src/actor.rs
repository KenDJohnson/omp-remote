use std::{
    collections::VecDeque,
    fmt,
    num::{NonZeroU64, NonZeroUsize},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use omp_rpc::{AvailableSlashCommand, ServerMessage};
use tokio::{
    sync::{mpsc, oneshot},
    time::{self, Instant},
};

use crate::{
    AgentId, AgentLifecycle, AgentSnapshot, AgentStateChange, AgentUpdate, EventSequence,
    InteractionLease, InteractionState, LeaseHolderId, RunId, RunSnapshot, SessionSummary,
    StateDelta, StateRevision, SubscriptionCursor, SubscriptionStart,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AgentActorConfig {
    pub replay_capacity: NonZeroUsize,
    pub subscriber_capacity: NonZeroUsize,
}

impl Default for AgentActorConfig {
    fn default() -> Self {
        Self {
            replay_capacity: NonZeroUsize::new(1_024)
                .expect("the default replay capacity is non-zero"),
            subscriber_capacity: NonZeroUsize::new(256)
                .expect("the default subscriber capacity is non-zero"),
        }
    }
}

#[derive(Clone, Debug)]
pub struct AgentHandle {
    agent_id: AgentId,
    command_tx: mpsc::Sender<ActorCommand>,
}

impl AgentHandle {
    #[must_use]
    pub fn spawn(agent_id: AgentId, config: AgentActorConfig) -> Self {
        Self::spawn_with_snapshot(AgentSnapshot::initial(agent_id), config)
    }

    #[must_use]
    pub fn spawn_with_snapshot(snapshot: AgentSnapshot, config: AgentActorConfig) -> Self {
        let agent_id = snapshot.agent_id.clone();
        let (command_tx, command_rx) = mpsc::channel(64);
        tokio::spawn(run_agent_actor(snapshot, config, command_rx));
        Self {
            agent_id,
            command_tx,
        }
    }

    #[must_use]
    pub fn agent_id(&self) -> &AgentId {
        &self.agent_id
    }

    pub async fn snapshot(&self) -> Result<AgentSnapshot, AgentError> {
        let (response_tx, response_rx) = oneshot::channel();
        self.send(ActorCommand::Snapshot { response_tx }).await?;
        response_rx.await.map_err(|_| AgentError::Stopped)
    }

    pub async fn set_lifecycle(&self, lifecycle: AgentLifecycle) -> Result<(), AgentError> {
        self.change(AgentStateChange::LifecycleChanged(lifecycle))
            .await
    }

    pub async fn set_session(&self, session: SessionSummary) -> Result<(), AgentError> {
        self.change(AgentStateChange::SessionChanged(session)).await
    }

    pub async fn clear_session(&self) -> Result<(), AgentError> {
        self.change(AgentStateChange::SessionCleared).await
    }

    pub async fn upsert_run(&self, run: RunSnapshot) -> Result<(), AgentError> {
        self.change(AgentStateChange::RunUpserted(run)).await
    }

    pub async fn remove_run(&self, run_id: RunId) -> Result<(), AgentError> {
        self.change(AgentStateChange::RunRemoved(run_id)).await
    }

    pub async fn set_available_commands(
        &self,
        commands: Vec<AvailableSlashCommand>,
    ) -> Result<(), AgentError> {
        self.change(AgentStateChange::AvailableCommandsChanged(commands))
            .await
    }

    pub async fn publish_event(&self, event: ServerMessage) -> Result<EventSequence, AgentError> {
        let (response_tx, response_rx) = oneshot::channel();
        self.send(ActorCommand::PublishEvent { event, response_tx })
            .await?;
        response_rx.await.map_err(|_| AgentError::Stopped)?
    }

    pub async fn subscribe(
        &self,
        cursor: Option<SubscriptionCursor>,
    ) -> Result<AgentSubscription, AgentError> {
        let (response_tx, response_rx) = oneshot::channel();
        self.send(ActorCommand::Subscribe {
            cursor,
            response_tx,
        })
        .await?;
        response_rx.await.map_err(|_| AgentError::Stopped)
    }

    pub async fn acquire_interaction_lease(
        &self,
        holder: LeaseHolderId,
        now_ms: u64,
        ttl_ms: NonZeroU64,
    ) -> Result<InteractionLease, AgentError> {
        let (response_tx, response_rx) = oneshot::channel();
        self.send(ActorCommand::AcquireLease {
            holder,
            now_ms,
            ttl_ms,
            response_tx,
        })
        .await?;
        response_rx.await.map_err(|_| AgentError::Stopped)?
    }

    pub async fn release_interaction_lease(&self, holder: LeaseHolderId) -> Result<(), AgentError> {
        let (response_tx, response_rx) = oneshot::channel();
        self.send(ActorCommand::ReleaseLease {
            holder,
            response_tx,
        })
        .await?;
        response_rx.await.map_err(|_| AgentError::Stopped)?
    }

    pub async fn shutdown(&self) -> Result<(), AgentError> {
        let (response_tx, response_rx) = oneshot::channel();
        self.send(ActorCommand::Shutdown { response_tx }).await?;
        response_rx.await.map_err(|_| AgentError::Stopped)
    }

    async fn change(&self, change: AgentStateChange) -> Result<(), AgentError> {
        let (response_tx, response_rx) = oneshot::channel();
        self.send(ActorCommand::Change {
            change,
            response_tx,
        })
        .await?;
        response_rx.await.map_err(|_| AgentError::Stopped)?
    }

    async fn send(&self, command: ActorCommand) -> Result<(), AgentError> {
        self.command_tx
            .send(command)
            .await
            .map_err(|_| AgentError::Stopped)
    }
}

#[derive(Debug)]
pub struct AgentSubscription {
    start: SubscriptionStart,
    updates: mpsc::Receiver<AgentUpdate>,
    resync_required: Arc<AtomicBool>,
}

impl AgentSubscription {
    #[must_use]
    pub fn start(&self) -> &SubscriptionStart {
        &self.start
    }

    #[must_use]
    pub fn into_start(self) -> SubscriptionStart {
        self.start
    }

    pub async fn recv(&mut self) -> Result<AgentUpdate, SubscriptionError> {
        if self.resync_required.load(Ordering::Acquire) {
            return Err(SubscriptionError::ResyncRequired);
        }
        match self.updates.recv().await {
            Some(update) => Ok(update),
            None if self.resync_required.load(Ordering::Acquire) => {
                Err(SubscriptionError::ResyncRequired)
            }
            None => Err(SubscriptionError::Closed),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubscriptionError {
    ResyncRequired,
    Closed,
}

impl fmt::Display for SubscriptionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ResyncRequired => formatter.write_str("subscriber fell behind; resync required"),
            Self::Closed => formatter.write_str("agent subscription closed"),
        }
    }
}

impl std::error::Error for SubscriptionError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InteractionLeaseError {
    HeldByOther(InteractionLease),
    NotHeld,
    NotHolder { current: LeaseHolderId },
    ExpiryOverflow,
    DurationTooLarge,
}

impl fmt::Display for InteractionLeaseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HeldByOther(lease) => write!(
                formatter,
                "interaction lease is held by {} until {}",
                lease.holder, lease.expires_at_ms
            ),
            Self::NotHeld => formatter.write_str("interaction lease is not held"),
            Self::NotHolder { current } => {
                write!(formatter, "interaction lease is held by {current}")
            }
            Self::ExpiryOverflow => formatter.write_str("interaction lease expiry overflowed"),
            Self::DurationTooLarge => {
                formatter.write_str("interaction lease duration is too large")
            }
        }
    }
}

impl std::error::Error for InteractionLeaseError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentError {
    Stopped,
    RevisionExhausted,
    EventSequenceExhausted,
    InteractionLease(InteractionLeaseError),
}

impl fmt::Display for AgentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Stopped => formatter.write_str("agent actor stopped"),
            Self::RevisionExhausted => formatter.write_str("agent state revision exhausted"),
            Self::EventSequenceExhausted => formatter.write_str("agent event sequence exhausted"),
            Self::InteractionLease(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for AgentError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InteractionLease(error) => Some(error),
            _ => None,
        }
    }
}

impl From<InteractionLeaseError> for AgentError {
    fn from(error: InteractionLeaseError) -> Self {
        Self::InteractionLease(error)
    }
}

#[derive(Debug)]
enum ActorCommand {
    Snapshot {
        response_tx: oneshot::Sender<AgentSnapshot>,
    },
    Change {
        change: AgentStateChange,
        response_tx: oneshot::Sender<Result<(), AgentError>>,
    },
    PublishEvent {
        event: ServerMessage,
        response_tx: oneshot::Sender<Result<EventSequence, AgentError>>,
    },
    Subscribe {
        cursor: Option<SubscriptionCursor>,
        response_tx: oneshot::Sender<AgentSubscription>,
    },
    AcquireLease {
        holder: LeaseHolderId,
        now_ms: u64,
        ttl_ms: NonZeroU64,
        response_tx: oneshot::Sender<Result<InteractionLease, AgentError>>,
    },
    ReleaseLease {
        holder: LeaseHolderId,
        response_tx: oneshot::Sender<Result<(), AgentError>>,
    },
    Shutdown {
        response_tx: oneshot::Sender<()>,
    },
}

#[derive(Debug)]
struct Subscriber {
    updates: mpsc::Sender<AgentUpdate>,
    resync_required: Arc<AtomicBool>,
}

#[derive(Debug)]
struct ActorState {
    snapshot: AgentSnapshot,
    replay: VecDeque<AgentUpdate>,
    replay_capacity: usize,
    subscriber_capacity: usize,
    subscribers: Vec<Subscriber>,
    lease_deadline: Option<Instant>,
}

async fn run_agent_actor(
    snapshot: AgentSnapshot,
    config: AgentActorConfig,
    mut command_rx: mpsc::Receiver<ActorCommand>,
) {
    let mut state = ActorState {
        snapshot,
        replay: VecDeque::with_capacity(config.replay_capacity.get()),
        replay_capacity: config.replay_capacity.get(),
        subscriber_capacity: config.subscriber_capacity.get(),
        subscribers: Vec::new(),
        lease_deadline: None,
    };

    loop {
        let deadline = state.lease_deadline.unwrap_or_else(Instant::now);
        tokio::select! {
            _ = time::sleep_until(deadline), if state.lease_deadline.is_some() => {
                state.lease_deadline = None;
                let _ = state.apply_change(AgentStateChange::InteractionChanged(
                    InteractionState::Unclaimed,
                ));
            }
            command = command_rx.recv() => {
                let Some(command) = command else {
                    return;
                };
                match command {
                    ActorCommand::Snapshot { response_tx } => {
                        let _ = response_tx.send(state.snapshot.clone());
                    }
                    ActorCommand::Change { change, response_tx } => {
                        let _ = response_tx.send(state.apply_change(change).map(|_| ()));
                    }
                    ActorCommand::PublishEvent { event, response_tx } => {
                        let _ = response_tx.send(state.publish_event(event));
                    }
                    ActorCommand::Subscribe { cursor, response_tx } => {
                        let _ = response_tx.send(state.subscribe(cursor));
                    }
                    ActorCommand::AcquireLease {
                        holder,
                        now_ms,
                        ttl_ms,
                        response_tx,
                    } => {
                        let _ = response_tx.send(state.acquire_lease(holder, now_ms, ttl_ms));
                    }
                    ActorCommand::ReleaseLease { holder, response_tx } => {
                        let _ = response_tx.send(state.release_lease(&holder));
                    }
                    ActorCommand::Shutdown { response_tx } => {
                        let _ = response_tx.send(());
                        return;
                    }
                }
            }
        }
    }
}

impl ActorState {
    fn apply_change(&mut self, change: AgentStateChange) -> Result<Option<StateDelta>, AgentError> {
        if !self.snapshot.would_change(&change) {
            return Ok(None);
        }

        let revision = StateRevision(
            self.snapshot
                .revision
                .0
                .checked_add(1)
                .ok_or(AgentError::RevisionExhausted)?,
        );
        let event_sequence = self.next_event_sequence()?;
        let delta = StateDelta {
            agent_id: self.snapshot.agent_id.clone(),
            base_revision: self.snapshot.revision,
            revision,
            change,
        };
        self.snapshot
            .apply_delta(delta.clone(), event_sequence)
            .expect("the actor constructs contiguous state deltas");
        self.record(AgentUpdate::Delta {
            event_sequence,
            delta: delta.clone(),
        });
        Ok(Some(delta))
    }

    fn publish_event(&mut self, event: ServerMessage) -> Result<EventSequence, AgentError> {
        let event_sequence = self.next_event_sequence()?;
        self.snapshot.event_sequence = event_sequence;
        self.record(AgentUpdate::Event {
            agent_id: self.snapshot.agent_id.clone(),
            event_sequence,
            event,
        });
        Ok(event_sequence)
    }

    fn next_event_sequence(&self) -> Result<EventSequence, AgentError> {
        self.snapshot
            .event_sequence
            .0
            .checked_add(1)
            .map(EventSequence)
            .ok_or(AgentError::EventSequenceExhausted)
    }

    fn record(&mut self, update: AgentUpdate) {
        if self.replay.len() == self.replay_capacity {
            self.replay.pop_front();
        }
        self.replay.push_back(update.clone());
        self.subscribers.retain(
            |subscriber| match subscriber.updates.try_send(update.clone()) {
                Ok(()) => true,
                Err(mpsc::error::TrySendError::Full(_)) => {
                    subscriber.resync_required.store(true, Ordering::Release);
                    false
                }
                Err(mpsc::error::TrySendError::Closed(_)) => false,
            },
        );
    }

    fn subscribe(&mut self, cursor: Option<SubscriptionCursor>) -> AgentSubscription {
        let (updates, receiver) = mpsc::channel(self.subscriber_capacity);
        let resync_required = Arc::new(AtomicBool::new(false));
        self.subscribers.push(Subscriber {
            updates,
            resync_required: resync_required.clone(),
        });
        let start = cursor.map_or_else(
            || SubscriptionStart::Snapshot(self.snapshot.clone()),
            |cursor| self.subscription_start(&cursor),
        );
        AgentSubscription {
            start,
            updates: receiver,
            resync_required,
        }
    }

    fn subscription_start(&self, cursor: &SubscriptionCursor) -> SubscriptionStart {
        if cursor.agent_id != self.snapshot.agent_id
            || cursor.revision > self.snapshot.revision
            || cursor.event_sequence > self.snapshot.event_sequence
        {
            return SubscriptionStart::ResyncRequired(self.snapshot.clone());
        }
        if cursor.revision == self.snapshot.revision
            && cursor.event_sequence == self.snapshot.event_sequence
        {
            return SubscriptionStart::Replay(Vec::new());
        }

        let updates: Vec<_> = self
            .replay
            .iter()
            .filter(|update| update.event_sequence() > cursor.event_sequence)
            .cloned()
            .collect();
        if replay_reaches_snapshot(cursor, &updates, &self.snapshot) {
            SubscriptionStart::Replay(updates)
        } else {
            SubscriptionStart::ResyncRequired(self.snapshot.clone())
        }
    }

    fn acquire_lease(
        &mut self,
        holder: LeaseHolderId,
        now_ms: u64,
        ttl_ms: NonZeroU64,
    ) -> Result<InteractionLease, AgentError> {
        if let InteractionState::Leased { lease } = &self.snapshot.interaction
            && lease.expires_at_ms > now_ms
            && lease.holder != holder
        {
            return Err(InteractionLeaseError::HeldByOther(lease.clone()).into());
        }
        let expires_at_ms = now_ms
            .checked_add(ttl_ms.get())
            .ok_or(InteractionLeaseError::ExpiryOverflow)?;
        let duration = Duration::from_millis(ttl_ms.get());
        let deadline = Instant::now()
            .checked_add(duration)
            .ok_or(InteractionLeaseError::DurationTooLarge)?;
        let lease = InteractionLease {
            holder,
            expires_at_ms,
        };
        self.apply_change(AgentStateChange::InteractionChanged(
            InteractionState::Leased {
                lease: lease.clone(),
            },
        ))?;
        self.lease_deadline = Some(deadline);
        Ok(lease)
    }

    fn release_lease(&mut self, holder: &LeaseHolderId) -> Result<(), AgentError> {
        match &self.snapshot.interaction {
            InteractionState::Unclaimed => return Err(InteractionLeaseError::NotHeld.into()),
            InteractionState::Leased { lease } if lease.holder != *holder => {
                return Err(InteractionLeaseError::NotHolder {
                    current: lease.holder.clone(),
                }
                .into());
            }
            InteractionState::Leased { .. } => {}
        }
        self.lease_deadline = None;
        self.apply_change(AgentStateChange::InteractionChanged(
            InteractionState::Unclaimed,
        ))?;
        Ok(())
    }
}

fn replay_reaches_snapshot(
    cursor: &SubscriptionCursor,
    updates: &[AgentUpdate],
    snapshot: &AgentSnapshot,
) -> bool {
    let Some(mut expected_sequence) = cursor.event_sequence.0.checked_add(1) else {
        return false;
    };
    let mut revision = cursor.revision;
    for update in updates {
        if update.event_sequence().0 != expected_sequence {
            return false;
        }
        match update {
            AgentUpdate::Delta { delta, .. } => {
                if delta.agent_id != cursor.agent_id || delta.base_revision != revision {
                    return false;
                }
                revision = delta.revision;
            }
            AgentUpdate::Event { agent_id, .. } if *agent_id != cursor.agent_id => return false,
            AgentUpdate::Event { .. } => {}
        }
        let Some(next) = expected_sequence.checked_add(1) else {
            return false;
        };
        expected_sequence = next;
    }
    revision == snapshot.revision
        && expected_sequence.checked_sub(1) == Some(snapshot.event_sequence.0)
}
