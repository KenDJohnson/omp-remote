use omp_control_client::ReplicatedState;
#[cfg(target_arch = "wasm32")]
use omp_control_client::{BrowserWebSocketAdapter, SocketTarget, WebSocketAdapter};
use omp_control_protocol::{
    AgentId, AgentLifecycle, AgentSnapshot, AgentStateChange, ConnectionPhase, DeltaEnvelope,
    EventSequence, FrameLimits, ServerFrame, StateDelta, StateRevision, StateSnapshot,
};

#[cfg(target_arch = "wasm32")]
use omp_control_protocol::TlsIdentityHint;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::wasm_bindgen_test;

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn native_and_browser_targets_converge_from_identical_cbor_frames() {
    let agent_id = AgentId::new("shared-agent").unwrap();
    let frames = vec![
        ServerFrame::Snapshot(StateSnapshot {
            agents: vec![AgentSnapshot::initial(agent_id.clone())],
        }),
        delta(&agent_id, 0, 1, 1, AgentLifecycle::Idle),
        delta(&agent_id, 1, 2, 2, AgentLifecycle::Running),
    ];
    let codec = FrameLimits::default().codec(ConnectionPhase::Authenticated);
    let mut state = ReplicatedState::default();
    for frame in frames {
        let encoded = codec.encode(&frame).unwrap();
        state
            .apply(codec.decode::<ServerFrame>(&encoded).unwrap())
            .unwrap();
    }

    let agent = state.agent(&agent_id).unwrap();
    assert_eq!(agent.revision, StateRevision(2));
    assert_eq!(agent.event_sequence, EventSequence(2));
    assert_eq!(agent.lifecycle, AgentLifecycle::Running);
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen_test]
async fn browser_adapter_refuses_native_certificate_pins() {
    let result = BrowserWebSocketAdapter
        .connect(&SocketTarget {
            endpoint: "wss://example.com/control".to_owned(),
            tls_identity: TlsIdentityHint::Sha256Fingerprint("00".repeat(32)),
        })
        .await;
    let Err(error) = result else {
        panic!("browser unexpectedly accepted a native certificate pin")
    };
    assert!(
        error
            .to_string()
            .contains("do not expose certificate pinning")
    );
}

fn delta(
    agent_id: &AgentId,
    base: u64,
    revision: u64,
    sequence: u64,
    lifecycle: AgentLifecycle,
) -> ServerFrame {
    ServerFrame::Delta(DeltaEnvelope {
        event_sequence: EventSequence(sequence),
        delta: StateDelta {
            agent_id: agent_id.clone(),
            base_revision: StateRevision(base),
            revision: StateRevision(revision),
            change: AgentStateChange::LifecycleChanged(lifecycle),
        },
    })
}
