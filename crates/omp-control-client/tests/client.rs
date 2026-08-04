#![cfg(not(target_arch = "wasm32"))]

use std::{future::Future, pin::Pin, sync::Arc, time::Duration};

use futures_channel::mpsc;
use futures_util::StreamExt;
use omp_control_client::{
    BinaryWebSocket, ClientConfig, ClientRunner, ConnectionStatus, CredentialStore,
    MemoryCredentialStore, SocketEvent, SocketTarget, TransportError, WebSocketAdapter,
};
use omp_control_protocol::{
    AgentId, AgentLifecycle, AgentSnapshot, AgentStateChange, CborCodec, ClientAuthentication,
    ClientCapabilities, ClientDescriptor, ClientFrame, ClientPlatform, ConnectionId,
    ConnectionPhase, ControlRequest, ControlResponse, DeltaEnvelope, DeviceCredential, DeviceId,
    DeviceScopes, DeviceToken, EventSequence, FrameLimits, LeaseHolderId, PairingBundle, PairingId,
    PairingSecret, ProtocolVersion, ReplayGap, ReplayGapReason, ResponseEnvelope, ResponseOutcome,
    RunId, ServerCapabilities, ServerFrame, ServerId, ServerWelcome, StateDelta, StateRevision,
    StateSnapshot, SubscribeRequest, TlsIdentityHint,
};
use omp_rpc::{ExtensionUiResponse, ExtensionUiResponseFrame};
use tokio::sync::{Mutex as AsyncMutex, mpsc as tokio_mpsc};
use tokio::time;

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

struct MockSocket {
    incoming: mpsc::UnboundedReceiver<Result<SocketEvent, TransportError>>,
    outgoing: mpsc::UnboundedSender<Vec<u8>>,
}

struct ServerConnection {
    incoming: mpsc::UnboundedReceiver<Vec<u8>>,
    outgoing: mpsc::UnboundedSender<Result<SocketEvent, TransportError>>,
}

fn socket_pair() -> (MockSocket, ServerConnection) {
    let (client_outgoing, server_incoming) = mpsc::unbounded();
    let (server_outgoing, client_incoming) = mpsc::unbounded();
    (
        MockSocket {
            incoming: client_incoming,
            outgoing: client_outgoing,
        },
        ServerConnection {
            incoming: server_incoming,
            outgoing: server_outgoing,
        },
    )
}

impl BinaryWebSocket for MockSocket {
    type SendFuture<'a> = BoxFuture<'a, Result<(), TransportError>>;
    type ReceiveFuture<'a> = BoxFuture<'a, Result<SocketEvent, TransportError>>;
    type CloseFuture<'a> = BoxFuture<'a, Result<(), TransportError>>;

    fn send_binary(&mut self, bytes: Vec<u8>) -> Self::SendFuture<'_> {
        Box::pin(async move {
            self.outgoing
                .unbounded_send(bytes)
                .map_err(|_| TransportError::new("mock server disconnected"))
        })
    }

    fn receive(&mut self) -> Self::ReceiveFuture<'_> {
        Box::pin(async move {
            self.incoming
                .next()
                .await
                .unwrap_or(Ok(SocketEvent::Closed { reason: None }))
        })
    }

    fn close(&mut self) -> Self::CloseFuture<'_> {
        Box::pin(async { Ok(()) })
    }
}

#[derive(Clone)]
struct MockAdapter {
    sockets: Arc<AsyncMutex<tokio_mpsc::UnboundedReceiver<MockSocket>>>,
}

impl WebSocketAdapter for MockAdapter {
    type Socket = MockSocket;
    type ConnectFuture<'a> = BoxFuture<'a, Result<Self::Socket, TransportError>>;
    type SleepFuture<'a> = tokio::time::Sleep;

    fn connect<'a>(&'a self, _target: &'a SocketTarget) -> Self::ConnectFuture<'a> {
        Box::pin(async move {
            self.sockets
                .lock()
                .await
                .recv()
                .await
                .ok_or_else(|| TransportError::new("no mock connection available"))
        })
    }

    fn sleep(&self, duration: Duration) -> Self::SleepFuture<'_> {
        time::sleep(duration)
    }
}

struct Fixture {
    adapter: MockAdapter,
    sockets: tokio_mpsc::UnboundedSender<MockSocket>,
    credential_store: MemoryCredentialStore,
    codec: CborCodec,
    server_id: ServerId,
    device_id: DeviceId,
}

impl Fixture {
    fn new() -> Self {
        let (sockets, socket_rx) = tokio_mpsc::unbounded_channel();
        let server_id = ServerId::new("server-1").unwrap();
        let device_id = DeviceId::new("device-1").unwrap();
        let credential_store = MemoryCredentialStore::default();
        credential_store
            .save(&DeviceCredential {
                server_id: server_id.clone(),
                device_id: device_id.clone(),
                token: DeviceToken::new("device-token"),
                scopes: DeviceScopes::all(),
            })
            .unwrap();
        Self {
            adapter: MockAdapter {
                sockets: Arc::new(AsyncMutex::new(socket_rx)),
            },
            sockets,
            credential_store,
            codec: FrameLimits::default().codec(ConnectionPhase::Authenticated),
            server_id,
            device_id,
        }
    }

    fn config(&self) -> ClientConfig {
        let mut config = ClientConfig::stored(
            SocketTarget {
                endpoint: "ws://127.0.0.1:1/control".to_owned(),
                tls_identity: TlsIdentityHint::InsecureDevelopment,
            },
            self.server_id.clone(),
            ClientDescriptor {
                name: "test-client".to_owned(),
                version: "1".to_owned(),
                platform: ClientPlatform::Desktop,
                capabilities: ClientCapabilities::default(),
            },
        );
        config.reconnect.initial_delay = Duration::from_millis(1);
        config.reconnect.maximum_delay = Duration::from_millis(2);
        config
    }

    fn welcome(&self) -> ServerFrame {
        ServerFrame::Welcome(ServerWelcome {
            protocol_version: ProtocolVersion::CURRENT,
            server_id: self.server_id.clone(),
            connection_id: ConnectionId::new("connection-1").unwrap(),
            device_id: self.device_id.clone(),
            capabilities: ServerCapabilities {
                enabled: Default::default(),
                max_frame_bytes: FrameLimits::default().post_auth().get(),
            },
            heartbeat_interval_ms: 1_000,
            issued_credential: None,
        })
    }

    fn connect(&self) -> ServerConnection {
        let (socket, server) = socket_pair();
        self.sockets.send(socket).unwrap();
        server
    }

    async fn receive_frame(&self, server: &mut ServerConnection) -> ClientFrame {
        let bytes = time::timeout(Duration::from_secs(1), server.incoming.next())
            .await
            .unwrap()
            .unwrap();
        self.codec.decode(&bytes).unwrap()
    }

    fn send(&self, server: &ServerConnection, frame: ServerFrame) {
        server
            .outgoing
            .unbounded_send(Ok(SocketEvent::Binary(self.codec.encode(&frame).unwrap())))
            .unwrap();
    }
}

#[tokio::test]
async fn pairing_persists_the_issued_credential_and_reconnects_as_the_device() {
    let fixture = Fixture::new();
    let credential_store = MemoryCredentialStore::default();
    let credential = DeviceCredential {
        server_id: fixture.server_id.clone(),
        device_id: fixture.device_id.clone(),
        token: DeviceToken::new("issued-token"),
        scopes: DeviceScopes::all(),
    };
    let mut config = ClientConfig::pairing(
        PairingBundle {
            format_version: 1,
            server_id: fixture.server_id.clone(),
            endpoint: "ws://127.0.0.1:1/control".to_owned(),
            pairing_id: PairingId::new("pairing-1").unwrap(),
            secret: PairingSecret::new("pairing-secret"),
            expires_at_ms: u64::MAX,
            tls_identity: TlsIdentityHint::InsecureDevelopment,
        },
        ClientDescriptor {
            name: "pairing-client".to_owned(),
            version: "1".to_owned(),
            platform: ClientPlatform::Desktop,
            capabilities: ClientCapabilities::default(),
        },
        "paired-device",
    )
    .unwrap();
    config.reconnect.initial_delay = Duration::from_millis(1);
    config.reconnect.maximum_delay = Duration::from_millis(2);

    let mut first = fixture.connect();
    let (handle, runner) = ClientRunner::new(config, credential_store.clone()).unwrap();
    let runner_task = tokio::spawn(runner.run(fixture.adapter.clone()));
    let ClientFrame::Hello(first_hello) = fixture.receive_frame(&mut first).await else {
        panic!("expected pairing hello")
    };
    assert!(matches!(
        first_hello.authentication,
        ClientAuthentication::Pair { .. }
    ));
    let ServerFrame::Welcome(mut welcome) = fixture.welcome() else {
        unreachable!()
    };
    welcome.issued_credential = Some(credential.clone());
    fixture.send(&first, ServerFrame::Welcome(welcome));
    wait_for_credential(&credential_store, &fixture.server_id).await;
    assert_eq!(
        credential_store.load(&fixture.server_id).unwrap(),
        Some(credential)
    );
    drop(first);

    let mut second = fixture.connect();
    let ClientFrame::Hello(second_hello) = fixture.receive_frame(&mut second).await else {
        panic!("expected device reconnect hello")
    };
    assert!(matches!(
        second_hello.authentication,
        ClientAuthentication::Device { .. }
    ));
    fixture.send(&second, fixture.welcome());
    wait_for_connected(&handle).await;

    handle.shutdown().await.unwrap();
    runner_task.await.unwrap().unwrap();
}

#[tokio::test]
async fn reconnect_retries_with_a_stable_operation_id_and_new_request_id() {
    let fixture = Fixture::new();
    let mut first = fixture.connect();
    let (handle, runner) =
        ClientRunner::new(fixture.config(), fixture.credential_store.clone()).unwrap();
    let runner_task = tokio::spawn(runner.run(fixture.adapter.clone()));

    assert!(matches!(
        fixture.receive_frame(&mut first).await,
        ClientFrame::Hello(_)
    ));
    fixture.send(&first, fixture.welcome());

    let request_handle = handle.clone();
    let request_task = tokio::spawn(async move {
        request_handle
            .request(ControlRequest::Prompt {
                agent_id: AgentId::new("agent-1").unwrap(),
                message: "hello".to_owned(),
                images: Vec::new(),
                streaming_behavior: None,
            })
            .await
    });
    let ClientFrame::Request(first_request) = fixture.receive_frame(&mut first).await else {
        panic!("expected first request")
    };
    let operation_id = first_request.operation_id.clone().unwrap();
    let first_request_id = first_request.request_id;
    drop(first);

    let mut second = fixture.connect();
    assert!(matches!(
        fixture.receive_frame(&mut second).await,
        ClientFrame::Hello(_)
    ));
    fixture.send(&second, fixture.welcome());
    let ClientFrame::Request(second_request) = fixture.receive_frame(&mut second).await else {
        panic!("expected retried request")
    };
    assert_eq!(second_request.operation_id, Some(operation_id));
    assert_ne!(second_request.request_id, first_request_id);

    fixture.send(
        &second,
        ServerFrame::Response(ResponseEnvelope {
            request_id: second_request.request_id,
            outcome: ResponseOutcome::Success(Box::new(ControlResponse::PromptAccepted {
                run_id: RunId::new("run-1").unwrap(),
            })),
        }),
    );
    assert!(matches!(
        request_task.await.unwrap().unwrap(),
        ControlResponse::PromptAccepted { .. }
    ));

    handle.shutdown().await.unwrap();
    runner_task.await.unwrap().unwrap();
}

#[tokio::test]
async fn ui_responses_retry_with_the_same_operation_id() {
    let fixture = Fixture::new();
    let mut first = fixture.connect();
    let (handle, runner) =
        ClientRunner::new(fixture.config(), fixture.credential_store.clone()).unwrap();
    let runner_task = tokio::spawn(runner.run(fixture.adapter.clone()));
    let _ = fixture.receive_frame(&mut first).await;
    fixture.send(&first, fixture.welcome());

    let response_handle = handle.clone();
    let response_task = tokio::spawn(async move {
        response_handle
            .respond_to_ui(
                AgentId::new("agent-1").unwrap(),
                LeaseHolderId::new("holder-1").unwrap(),
                ExtensionUiResponseFrame::Response {
                    id: "ui-1".to_owned(),
                    response: ExtensionUiResponse::Confirmed { confirmed: true },
                },
            )
            .await
    });
    let ClientFrame::UiResponse(first_response) = fixture.receive_frame(&mut first).await else {
        panic!("expected first UI response")
    };
    let operation_id = first_response.operation_id;
    let first_request_id = first_response.request_id;
    drop(first);

    let mut second = fixture.connect();
    let _ = fixture.receive_frame(&mut second).await;
    fixture.send(&second, fixture.welcome());
    let ClientFrame::UiResponse(second_response) = fixture.receive_frame(&mut second).await else {
        panic!("expected retried UI response")
    };
    assert_eq!(second_response.operation_id, operation_id);
    assert_ne!(second_response.request_id, first_request_id);
    fixture.send(
        &second,
        ServerFrame::Response(ResponseEnvelope {
            request_id: second_response.request_id,
            outcome: ResponseOutcome::Success(Box::new(ControlResponse::Accepted)),
        }),
    );
    assert_eq!(
        response_task.await.unwrap().unwrap(),
        ControlResponse::Accepted
    );

    handle.shutdown().await.unwrap();
    runner_task.await.unwrap().unwrap();
}

#[tokio::test]
async fn reconnect_uses_contiguous_replay_or_replaces_state_with_snapshot() {
    let fixture = Fixture::new();
    let agent_id = AgentId::new("agent-1").unwrap();
    let mut first = fixture.connect();
    let (handle, runner) =
        ClientRunner::new(fixture.config(), fixture.credential_store.clone()).unwrap();
    handle.subscribe(agent_id.clone()).unwrap();
    let runner_task = tokio::spawn(runner.run(fixture.adapter.clone()));

    assert!(matches!(
        fixture.receive_frame(&mut first).await,
        ClientFrame::Hello(_)
    ));
    fixture.send(&first, fixture.welcome());
    assert!(matches!(
        fixture.receive_frame(&mut first).await,
        ClientFrame::Subscribe(SubscribeRequest { cursor: None, .. })
    ));
    fixture.send(
        &first,
        ServerFrame::Snapshot(StateSnapshot {
            agents: vec![AgentSnapshot::initial(agent_id.clone())],
        }),
    );
    fixture.send(
        &first,
        lifecycle_delta(&agent_id, 0, 1, 1, AgentLifecycle::Idle),
    );
    wait_for_revision(&handle, &agent_id, StateRevision(1)).await;
    drop(first);

    let mut second = fixture.connect();
    let ClientFrame::Hello(hello) = fixture.receive_frame(&mut second).await else {
        panic!("expected reconnect hello")
    };
    assert_eq!(hello.resume.subscriptions.len(), 1);
    assert_eq!(hello.resume.subscriptions[0].revision, StateRevision(1));
    fixture.send(&second, fixture.welcome());
    let ClientFrame::Subscribe(subscription) = fixture.receive_frame(&mut second).await else {
        panic!("expected resumed subscription")
    };
    assert_eq!(subscription.cursor.unwrap().revision, StateRevision(1));
    fixture.send(
        &second,
        lifecycle_delta(&agent_id, 1, 2, 2, AgentLifecycle::Running),
    );
    wait_for_revision(&handle, &agent_id, StateRevision(2)).await;
    drop(second);

    let mut third = fixture.connect();
    assert!(matches!(
        fixture.receive_frame(&mut third).await,
        ClientFrame::Hello(_)
    ));
    fixture.send(&third, fixture.welcome());
    let _ = fixture.receive_frame(&mut third).await;
    fixture.send(
        &third,
        ServerFrame::ReplayGap(ReplayGap {
            agent_id: agent_id.clone(),
            current_revision: StateRevision(5),
            current_event_sequence: EventSequence(5),
            reason: ReplayGapReason::BufferExpired,
        }),
    );
    let mut replacement = AgentSnapshot::initial(agent_id.clone());
    replacement.revision = StateRevision(5);
    replacement.event_sequence = EventSequence(5);
    replacement.lifecycle = AgentLifecycle::Interrupted;
    fixture.send(
        &third,
        ServerFrame::Snapshot(StateSnapshot {
            agents: vec![replacement],
        }),
    );
    wait_for_revision(&handle, &agent_id, StateRevision(5)).await;
    assert_eq!(
        handle.state().agent(&agent_id).unwrap().lifecycle,
        AgentLifecycle::Interrupted
    );

    handle.shutdown().await.unwrap();
    runner_task.await.unwrap().unwrap();
}

fn lifecycle_delta(
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

async fn wait_for_credential(store: &MemoryCredentialStore, server_id: &ServerId) {
    time::timeout(Duration::from_secs(1), async {
        loop {
            if store.load(server_id).unwrap().is_some() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

async fn wait_for_connected(handle: &omp_control_client::ClientHandle) {
    time::timeout(Duration::from_secs(1), async {
        loop {
            if matches!(handle.status(), ConnectionStatus::Connected { .. }) {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

async fn wait_for_revision(
    handle: &omp_control_client::ClientHandle,
    agent_id: &AgentId,
    revision: StateRevision,
) {
    time::timeout(Duration::from_secs(1), async {
        loop {
            if handle
                .state()
                .agent(agent_id)
                .is_some_and(|agent| agent.revision == revision)
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}
