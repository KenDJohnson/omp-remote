use omp_control_client::{ReplicatedState, ReplicationEffect, ReplicationError};
use omp_control_protocol::{
    AgentId, AgentLifecycle, AgentSnapshot, AgentStateChange, DeltaEnvelope, EventEnvelope,
    EventSequence, ServerFrame, StateDelta, StateRevision, StateSnapshot, UiInteractionEnvelope,
};
use omp_rpc::{ExtensionUiRequest, ExtensionUiRequestFrame, ServerMessage, SessionEvent};

fn agent_id() -> AgentId {
    AgentId::new("agent-1").unwrap()
}

fn snapshot(revision: u64, sequence: u64, lifecycle: AgentLifecycle) -> AgentSnapshot {
    let mut snapshot = AgentSnapshot::initial(agent_id());
    snapshot.revision = StateRevision(revision);
    snapshot.event_sequence = EventSequence(sequence);
    snapshot.lifecycle = lifecycle;
    snapshot
}

fn lifecycle_delta(
    base: u64,
    revision: u64,
    sequence: u64,
    lifecycle: AgentLifecycle,
) -> ServerFrame {
    ServerFrame::Delta(DeltaEnvelope {
        event_sequence: EventSequence(sequence),
        delta: StateDelta {
            agent_id: agent_id(),
            base_revision: StateRevision(base),
            revision: StateRevision(revision),
            change: AgentStateChange::LifecycleChanged(lifecycle),
        },
    })
}

#[test]
fn snapshots_deltas_events_and_interactions_share_one_contiguous_cursor() {
    let mut state = ReplicatedState::default();
    state
        .apply(ServerFrame::Snapshot(StateSnapshot {
            agents: vec![snapshot(3, 7, AgentLifecycle::Starting)],
        }))
        .unwrap();
    state
        .apply(lifecycle_delta(3, 4, 8, AgentLifecycle::Idle))
        .unwrap();
    state
        .apply(ServerFrame::Event(EventEnvelope {
            agent_id: agent_id(),
            event_sequence: EventSequence(9),
            event: ServerMessage::SessionEvent(SessionEvent::AgentStart),
        }))
        .unwrap();
    let effects = state
        .apply(ServerFrame::InteractionRequest(UiInteractionEnvelope {
            agent_id: agent_id(),
            event_sequence: EventSequence(10),
            request: ExtensionUiRequestFrame::Request {
                id: "ui-1".to_owned(),
                request: ExtensionUiRequest::Confirm {
                    title: "Continue?".to_owned(),
                    message: "Proceed".to_owned(),
                    timeout: None,
                },
            },
        }))
        .unwrap();

    assert!(matches!(
        effects.as_slice(),
        [ReplicationEffect::Interaction(_)]
    ));
    let cursor = state.cursor(&agent_id()).unwrap();
    assert_eq!(cursor.revision, StateRevision(4));
    assert_eq!(cursor.event_sequence, EventSequence(10));
}

#[test]
fn gaps_fail_closed_and_a_fresh_snapshot_replaces_local_state() {
    let mut state = ReplicatedState::default();
    state
        .apply(ServerFrame::Snapshot(StateSnapshot {
            agents: vec![snapshot(1, 1, AgentLifecycle::Starting)],
        }))
        .unwrap();

    assert_eq!(
        state
            .apply(lifecycle_delta(1, 2, 3, AgentLifecycle::Running))
            .unwrap_err(),
        ReplicationError::NonContiguousEventSequence {
            local: EventSequence(1),
            incoming: EventSequence(3),
        }
    );
    assert_eq!(state.agent(&agent_id()).unwrap().revision, StateRevision(1));
    assert!(state.is_resync_required(&agent_id()));
    assert_eq!(state.cursor(&agent_id()), None);
    assert_eq!(
        state
            .apply(lifecycle_delta(1, 2, 2, AgentLifecycle::Running))
            .unwrap_err(),
        ReplicationError::ResyncPending(agent_id())
    );

    state
        .apply(ServerFrame::Snapshot(StateSnapshot {
            agents: vec![snapshot(8, 20, AgentLifecycle::Interrupted)],
        }))
        .unwrap();
    assert!(!state.is_resync_required(&agent_id()));
    state
        .apply(lifecycle_delta(8, 9, 21, AgentLifecycle::Idle))
        .unwrap();

    let agent = state.agent(&agent_id()).unwrap();
    assert_eq!(agent.revision, StateRevision(9));
    assert_eq!(agent.event_sequence, EventSequence(21));
    assert_eq!(agent.lifecycle, AgentLifecycle::Idle);
}
