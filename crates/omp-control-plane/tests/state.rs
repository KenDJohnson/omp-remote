use std::{num::NonZeroUsize, time::Duration};

use omp_control_plane::*;
use omp_rpc::{ServerMessage, SessionEvent};
use tokio::time;

fn agent_id() -> AgentId {
    AgentId::new("agent-1").unwrap()
}

fn config(replay_capacity: usize, subscriber_capacity: usize) -> AgentActorConfig {
    AgentActorConfig {
        replay_capacity: NonZeroUsize::new(replay_capacity).unwrap(),
        subscriber_capacity: NonZeroUsize::new(subscriber_capacity).unwrap(),
    }
}

#[tokio::test]
async fn snapshot_plus_deltas_converges_to_authoritative_state() {
    let agent = AgentHandle::spawn(agent_id(), config(16, 16));
    let mut subscription = agent.subscribe(None).await.unwrap();
    let mut reduced = match subscription.start().clone() {
        SubscriptionStart::Snapshot(snapshot) => snapshot,
        other => panic!("expected initial snapshot, got {other:?}"),
    };

    agent.set_lifecycle(AgentLifecycle::Idle).await.unwrap();
    agent
        .set_session(SessionSummary {
            session_id: "session-1".into(),
            session_file: Some("/tmp/session.jsonl".into()),
            name: Some("demo".into()),
            message_count: 3,
        })
        .await
        .unwrap();
    let run_id = RunId::new("run-1").unwrap();
    agent
        .upsert_run(RunSnapshot {
            run_id: run_id.clone(),
            lifecycle: RunLifecycle::Running,
            started_at_ms: 1_000,
            ended_at_ms: None,
        })
        .await
        .unwrap();
    agent
        .upsert_run(RunSnapshot {
            run_id,
            lifecycle: RunLifecycle::Completed,
            started_at_ms: 1_000,
            ended_at_ms: Some(2_000),
        })
        .await
        .unwrap();

    for _ in 0..4 {
        let update = time::timeout(Duration::from_secs(1), subscription.recv())
            .await
            .unwrap()
            .unwrap();
        reduced.apply_update(update).unwrap();
    }

    assert_eq!(reduced, agent.snapshot().await.unwrap());
    assert_eq!(reduced.revision, StateRevision(4));
    assert_eq!(reduced.event_sequence, EventSequence(4));
}

#[tokio::test]
async fn subscribers_receive_identical_ordered_revisions() {
    let agent = AgentHandle::spawn(agent_id(), config(16, 16));
    let mut first = agent.subscribe(None).await.unwrap();
    let mut second = agent.subscribe(None).await.unwrap();

    agent.set_lifecycle(AgentLifecycle::Idle).await.unwrap();
    agent.set_lifecycle(AgentLifecycle::Running).await.unwrap();
    agent.set_lifecycle(AgentLifecycle::Stopping).await.unwrap();

    let mut first_updates = Vec::new();
    let mut second_updates = Vec::new();
    for _ in 0..3 {
        first_updates.push(first.recv().await.unwrap());
        second_updates.push(second.recv().await.unwrap());
    }
    assert_eq!(first_updates, second_updates);
    assert_eq!(
        first_updates
            .iter()
            .map(AgentUpdate::event_sequence)
            .collect::<Vec<_>>(),
        vec![EventSequence(1), EventSequence(2), EventSequence(3)]
    );
}

#[tokio::test]
async fn replay_gaps_and_missed_revisions_require_resynchronization() {
    let agent = AgentHandle::spawn(agent_id(), config(2, 8));
    let mut live = agent.subscribe(None).await.unwrap();
    let initial = match live.start().clone() {
        SubscriptionStart::Snapshot(snapshot) => snapshot,
        other => panic!("expected initial snapshot, got {other:?}"),
    };
    let cursor = SubscriptionCursor {
        agent_id: initial.agent_id.clone(),
        revision: initial.revision,
        event_sequence: initial.event_sequence,
    };

    agent.set_lifecycle(AgentLifecycle::Idle).await.unwrap();
    agent.set_lifecycle(AgentLifecycle::Running).await.unwrap();
    agent.set_lifecycle(AgentLifecycle::Stopping).await.unwrap();

    let _first = live.recv().await.unwrap();
    let second = live.recv().await.unwrap();
    let mut stale = initial;
    assert!(matches!(
        stale.apply_update(second),
        Err(DeltaApplyError::RevisionMismatch { .. })
    ));

    let resumed = agent.subscribe(Some(cursor)).await.unwrap();
    assert!(matches!(
        resumed.start(),
        SubscriptionStart::ResyncRequired(snapshot)
            if snapshot.revision == StateRevision(3)
    ));
}

#[tokio::test]
async fn event_sequences_advance_without_changing_state_revision() {
    let agent = AgentHandle::spawn(agent_id(), config(8, 8));
    let sequence = agent
        .publish_event(ServerMessage::SessionEvent(SessionEvent::AgentStart))
        .await
        .unwrap();
    let snapshot = agent.snapshot().await.unwrap();

    assert_eq!(sequence, EventSequence(1));
    assert_eq!(snapshot.event_sequence, EventSequence(1));
    assert_eq!(snapshot.revision, StateRevision(0));
}

#[tokio::test]
async fn interaction_lease_is_exclusive_and_expires() {
    let agent = AgentHandle::spawn(agent_id(), config(8, 8));
    let first = LeaseHolderId::new("client-1").unwrap();
    let second = LeaseHolderId::new("client-2").unwrap();
    let lease = agent
        .acquire_interaction_lease(
            first.clone(),
            10_000,
            std::num::NonZeroU64::new(20).unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(lease.expires_at_ms, 10_020);
    assert!(matches!(
        agent
            .acquire_interaction_lease(second, 10_001, std::num::NonZeroU64::new(20).unwrap())
            .await,
        Err(AgentError::InteractionLease(
            InteractionLeaseError::HeldByOther(_)
        ))
    ));

    time::sleep(Duration::from_millis(40)).await;
    assert_eq!(
        agent.snapshot().await.unwrap().interaction,
        InteractionState::Unclaimed
    );
    assert!(matches!(
        agent.release_interaction_lease(first).await,
        Err(AgentError::InteractionLease(InteractionLeaseError::NotHeld))
    ));
}

#[tokio::test]
async fn slow_subscriber_receives_an_explicit_resync_error() {
    let agent = AgentHandle::spawn(agent_id(), config(8, 1));
    let mut subscription = agent.subscribe(None).await.unwrap();
    agent.set_lifecycle(AgentLifecycle::Idle).await.unwrap();
    agent.set_lifecycle(AgentLifecycle::Running).await.unwrap();

    assert_eq!(
        subscription.recv().await,
        Err(SubscriptionError::ResyncRequired)
    );
}
