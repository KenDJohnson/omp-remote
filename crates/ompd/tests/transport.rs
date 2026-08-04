use std::{
    collections::BTreeSet,
    num::{NonZeroU32, NonZeroU64},
    os::unix::fs::PermissionsExt,
    sync::Arc,
    time::Duration,
};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use futures_util::{SinkExt, StreamExt};
use omp_control_protocol::{
    AgentId, AgentLifecycle, CborCodec, ClientAuthentication, ClientCapabilities, ClientDescriptor,
    ClientFrame, ClientHello, ClientPlatform, ConnectionPhase, ControlRequest, DeviceDescriptor,
    DeviceId, DeviceScopes, DeviceToken, EventSequence, FrameLimits, OperationId, PairingBundle,
    ProtocolVersion, RequestEnvelope, RequestId, ResponseOutcome, ResumeState, ServerFrame,
    StateRevision, SubscribeRequest, TlsIdentityHint,
};
use omp_rpc::{ServerMessage, SessionEvent};
use omp_runtime::RuntimeConfig;
use ompd::{
    DaemonServer, ServerSessionConfig, TlsMode, TransportConfig, TransportConfigError,
    load_rustls_config,
    persistence::{AgentRecord, DeviceRecord, SessionResumeRecord, Store},
    request_pairing, serve_admin_socket, unix_time_ms,
};
use parking_lot::Mutex;
use rustls::RootCertStore;
use tempfile::TempDir;
use tokio::task::JoinHandle;
use tokio_tungstenite::{
    Connector, WebSocketStream, connect_async, connect_async_tls_with_config, tungstenite::Message,
};

#[test]
fn plaintext_and_proxy_modes_are_restricted_to_loopback() {
    let loopback = "127.0.0.1:8080".parse().unwrap();
    let public = "0.0.0.0:8080".parse().unwrap();
    let development = TransportConfig {
        bind_address: loopback,
        public_endpoint: "ws://127.0.0.1:8080/control".into(),
        tls_mode: TlsMode::DevelopmentPlaintext {
            local_endpoint: loopback,
        },
    };
    assert_eq!(development.validate(), Ok(()));
    assert_eq!(
        TransportConfig {
            bind_address: public,
            public_endpoint: "ws://example.test/control".into(),
            tls_mode: TlsMode::DevelopmentPlaintext {
                local_endpoint: public,
            },
        }
        .validate(),
        Err(TransportConfigError::PlaintextMustBeLoopback)
    );
    assert_eq!(
        TransportConfig {
            bind_address: public,
            public_endpoint: "wss://example.test/control".into(),
            tls_mode: TlsMode::TrustedReverseProxy {
                local_endpoint: public,
            },
        }
        .validate(),
        Err(TransportConfigError::ProxyMustBeLocal)
    );
}

#[test]
fn certificate_modes_cover_trusted_and_pinned_deployments() {
    let directory = TempDir::new().unwrap();
    let certificates = certificates(&directory);
    load_rustls_config(
        &certificates.certificate_path,
        &certificates.private_key_path,
    )
    .unwrap();
    for endpoint in [
        "wss://host.tailnet.ts.net/control",
        "wss://control.example.com/control",
    ] {
        let transport = TransportConfig {
            bind_address: "0.0.0.0:443".parse().unwrap(),
            public_endpoint: endpoint.into(),
            tls_mode: TlsMode::CertificateFiles {
                certificate: certificates.certificate_path.clone(),
                private_key: certificates.private_key_path.clone(),
            },
        };
        assert_eq!(transport.validate(), Ok(()));
        assert_eq!(
            transport.tls_identity_hint().unwrap(),
            TlsIdentityHint::PubliclyTrusted
        );
    }
    let pinned = TransportConfig {
        bind_address: "0.0.0.0:443".parse().unwrap(),
        public_endpoint: "wss://192.0.2.1/control".into(),
        tls_mode: TlsMode::PinnedSelfSigned {
            certificate: certificates.certificate_path,
            private_key: certificates.private_key_path,
        },
    };
    let TlsIdentityHint::Sha256Fingerprint(fingerprint) = pinned.tls_identity_hint().unwrap()
    else {
        panic!("pinned TLS must advertise a fingerprint");
    };
    assert_eq!(fingerprint.len(), 64);
}

#[tokio::test]
async fn unauthenticated_and_revoked_clients_receive_no_state() {
    let fixture = PlainServer::start().await;
    let (mut socket, _) = connect_async(fixture.url()).await.unwrap();
    send_client(
        &mut socket,
        &ClientFrame::Ping(omp_control_protocol::Ping { nonce: 7 }),
        ConnectionPhase::PreAuth,
    )
    .await;
    assert!(matches!(
        socket.next().await,
        Some(Ok(Message::Close(_))) | None
    ));

    let token = DeviceToken::new("revoked-device-token");
    let device = device_record("revoked", DeviceScopes::all());
    fixture.store.lock().insert_device(&device, &token).unwrap();
    assert!(
        fixture
            .store
            .lock()
            .revoke_device(&device.device_id, unix_time_ms())
            .unwrap()
    );
    let (mut socket, _) = connect_async(fixture.url()).await.unwrap();
    send_client(
        &mut socket,
        &device_hello(&device.device_id, token),
        ConnectionPhase::PreAuth,
    )
    .await;
    assert!(matches!(
        socket.next().await,
        Some(Ok(Message::Close(_))) | None
    ));
    fixture.stop();
}

#[tokio::test]
async fn pairing_is_single_use_and_issues_a_scoped_device_credential() {
    let fixture = PlainServer::start().await;
    let now = unix_time_ms();
    let grant = fixture
        .store
        .lock()
        .create_pairing(
            "phone",
            observe_only(),
            now,
            NonZeroU64::new(60_000).unwrap(),
        )
        .unwrap();
    let hello = pair_hello(grant.pairing_id.clone(), grant.secret.clone());
    let (mut socket, _) = connect_async(fixture.url()).await.unwrap();
    send_client(&mut socket, &hello, ConnectionPhase::PreAuth).await;
    let welcome = receive_server(&mut socket).await;
    let ServerFrame::Welcome(welcome) = welcome else {
        panic!("pairing must return a welcome frame");
    };
    let credential = welcome
        .issued_credential
        .expect("pairing must issue a credential");
    assert_eq!(credential.scopes, observe_only());
    socket.close(None).await.unwrap();

    let (mut reused, _) = connect_async(fixture.url()).await.unwrap();
    send_client(&mut reused, &hello, ConnectionPhase::PreAuth).await;
    assert!(matches!(
        reused.next().await,
        Some(Ok(Message::Close(_))) | None
    ));

    let (mut authenticated, _) = connect_async(fixture.url()).await.unwrap();
    send_client(
        &mut authenticated,
        &device_hello(&credential.device_id, credential.token),
        ConnectionPhase::PreAuth,
    )
    .await;
    assert!(matches!(
        receive_server(&mut authenticated).await,
        ServerFrame::Welcome(_)
    ));
    fixture.stop();
}

#[tokio::test]
async fn expired_pairing_secret_is_rejected() {
    let fixture = PlainServer::start().await;
    let grant = fixture
        .store
        .lock()
        .create_pairing(
            "expired",
            DeviceScopes::all(),
            unix_time_ms().saturating_sub(10_000),
            NonZeroU64::new(1).unwrap(),
        )
        .unwrap();
    let (mut socket, _) = connect_async(fixture.url()).await.unwrap();
    send_client(
        &mut socket,
        &pair_hello(grant.pairing_id, grant.secret),
        ConnectionPhase::PreAuth,
    )
    .await;
    assert!(matches!(
        socket.next().await,
        Some(Ok(Message::Close(_))) | None
    ));
    fixture.stop();
}

#[tokio::test]
async fn device_scopes_are_enforced_before_mutating_dispatch() {
    let fixture = PlainServer::start().await;
    let token = DeviceToken::new("read-only-token");
    let device = device_record("reader", observe_only());
    fixture.store.lock().insert_device(&device, &token).unwrap();
    let (mut socket, _) = connect_async(fixture.url()).await.unwrap();
    send_client(
        &mut socket,
        &device_hello(&device.device_id, token),
        ConnectionPhase::PreAuth,
    )
    .await;
    assert!(matches!(
        receive_server(&mut socket).await,
        ServerFrame::Welcome(_)
    ));
    send_client(
        &mut socket,
        &ClientFrame::Request(RequestEnvelope {
            request_id: RequestId::new("launch-1").unwrap(),
            operation_id: Some(OperationId::new("operation-1").unwrap()),
            request: ControlRequest::LaunchAgent {
                agent_id: omp_control_protocol::AgentId::new("agent-1").unwrap(),
            },
        }),
        ConnectionPhase::Authenticated,
    )
    .await;
    let ServerFrame::Response(response) = receive_server(&mut socket).await else {
        panic!("request must receive a response");
    };
    let ResponseOutcome::Error(error) = response.outcome else {
        panic!("read-only device must not launch an agent");
    };
    assert_eq!(error.code, "permission_denied");
    fixture.stop();
}

#[tokio::test]
async fn tls_13_websocket_accepts_a_trusted_device() {
    let directory = TempDir::new().unwrap();
    let certificates = certificates(&directory);
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let store = Arc::new(Mutex::new(
        Store::open_at(directory.path().join("state.sqlite3"), unix_time_ms()).unwrap(),
    ));
    let token = DeviceToken::new("tls-device-token");
    let device = device_record("tls-device", DeviceScopes::all());
    store.lock().insert_device(&device, &token).unwrap();
    let server = DaemonServer::new(
        TransportConfig {
            bind_address: address,
            public_endpoint: format!("wss://localhost:{}/control", address.port()),
            tls_mode: TlsMode::CertificateFiles {
                certificate: certificates.certificate_path,
                private_key: certificates.private_key_path,
            },
        },
        test_session_config(),
        store,
        RuntimeConfig::new("unused-omp"),
    )
    .unwrap();
    let task = tokio::spawn(server.serve_with_listener(listener));
    let mut roots = RootCertStore::empty();
    roots.add(certificates.der).unwrap();
    let client_config =
        rustls::ClientConfig::builder_with_protocol_versions(&[&rustls::version::TLS13])
            .with_root_certificates(roots)
            .with_no_client_auth();
    let (mut socket, _) = connect_async_tls_with_config(
        format!("wss://localhost:{}/control", address.port()),
        None,
        false,
        Some(Connector::Rustls(Arc::new(client_config))),
    )
    .await
    .unwrap();
    send_client(
        &mut socket,
        &device_hello(&device.device_id, token),
        ConnectionPhase::PreAuth,
    )
    .await;
    assert!(matches!(
        receive_server(&mut socket).await,
        ServerFrame::Welcome(_)
    ));
    task.abort();
}

#[tokio::test]
async fn local_admin_socket_returns_fragment_links_and_owner_only_permissions() {
    let directory = TempDir::new().unwrap();
    let socket_path = directory.path().join("admin.sock");
    let store = Arc::new(Mutex::new(
        Store::open_at(directory.path().join("state.sqlite3"), unix_time_ms()).unwrap(),
    ));
    let address = "127.0.0.1:8443".parse().unwrap();
    let transport = TransportConfig {
        bind_address: address,
        public_endpoint: "wss://host.example/control".into(),
        tls_mode: TlsMode::TrustedReverseProxy {
            local_endpoint: address,
        },
    };
    let admin = tokio::spawn(serve_admin_socket(socket_path.clone(), store, transport));
    tokio::time::sleep(Duration::from_millis(10)).await;
    let links = request_pairing(
        &socket_path,
        "phone",
        NonZeroU64::new(60_000).unwrap(),
        DeviceScopes::all(),
    )
    .await
    .unwrap();
    assert!(links.native_link.starts_with("omp-remote://pair#"));
    assert!(links.browser_link.starts_with("https://host.example/pair#"));
    assert!(!links.native_link.contains('?'));
    assert_eq!(
        std::fs::metadata(&socket_path)
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    let payload = links.native_link.split_once('#').unwrap().1;
    let bytes = URL_SAFE_NO_PAD.decode(payload).unwrap();
    let bundle: PairingBundle = CborCodec::new(NonZeroU32::new(64 * 1_024).unwrap())
        .decode(&bytes)
        .unwrap();
    assert_eq!(bundle.tls_identity, TlsIdentityHint::PubliclyTrusted);
    assert!(links.human_output().contains("Native app:"));
    admin.abort();
}

#[tokio::test]
async fn heartbeat_closes_an_unresponsive_authenticated_client() {
    let fixture = PlainServer::start_with_session(ServerSessionConfig {
        authentication_timeout: Duration::from_secs(1),
        heartbeat_interval: Duration::from_millis(20),
        slow_client_timeout: Duration::from_millis(100),
        outbound_capacity: 4,
        ..ServerSessionConfig::default()
    })
    .await;
    let token = DeviceToken::new("heartbeat-token");
    let device = device_record("heartbeat", DeviceScopes::all());
    fixture.store.lock().insert_device(&device, &token).unwrap();
    let (mut socket, _) = connect_async(fixture.url()).await.unwrap();
    send_client(
        &mut socket,
        &device_hello(&device.device_id, token),
        ConnectionPhase::PreAuth,
    )
    .await;
    assert!(matches!(
        receive_server(&mut socket).await,
        ServerFrame::Welcome(_)
    ));
    tokio::time::sleep(Duration::from_millis(100)).await;
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            match socket.next().await {
                Some(Ok(Message::Ping(_))) => {}
                Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                Some(Ok(message)) => panic!("unexpected heartbeat message: {message:?}"),
            }
        }
    })
    .await
    .expect("unresponsive connection must be closed");
    fixture.stop();
}

#[tokio::test]
async fn slow_network_subscriber_never_blocks_authoritative_actor() {
    let directory = TempDir::new().unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let store = Arc::new(Mutex::new(
        Store::open_at(directory.path().join("state.sqlite3"), unix_time_ms()).unwrap(),
    ));
    let token = DeviceToken::new("slow-client-token");
    let device = device_record("slow-client", DeviceScopes::all());
    store.lock().insert_device(&device, &token).unwrap();
    let server = DaemonServer::new(
        TransportConfig {
            bind_address: address,
            public_endpoint: format!("ws://{address}/control"),
            tls_mode: TlsMode::DevelopmentPlaintext {
                local_endpoint: address,
            },
        },
        ServerSessionConfig {
            outbound_capacity: 1,
            heartbeat_interval: Duration::from_secs(60),
            ..test_session_config()
        },
        store,
        RuntimeConfig::new("unused-omp"),
    )
    .unwrap();
    let agent_id = AgentId::new("slow-agent").unwrap();
    let agent = server
        .controller()
        .registry()
        .create(agent_id.clone())
        .unwrap();
    let task = tokio::spawn(server.serve_with_listener(listener));
    tokio::task::yield_now().await;
    let (mut socket, _) = connect_async(format!("ws://{address}/control"))
        .await
        .unwrap();
    send_client(
        &mut socket,
        &device_hello(&device.device_id, token),
        ConnectionPhase::PreAuth,
    )
    .await;
    assert!(matches!(
        receive_server(&mut socket).await,
        ServerFrame::Welcome(_)
    ));
    send_client(
        &mut socket,
        &ClientFrame::Subscribe(SubscribeRequest {
            agent_id,
            cursor: None,
        }),
        ConnectionPhase::Authenticated,
    )
    .await;
    tokio::time::sleep(Duration::from_millis(10)).await;
    tokio::time::timeout(Duration::from_secs(2), async {
        for _ in 0..512 {
            agent
                .publish_event(ServerMessage::SessionEvent(SessionEvent::AgentStart))
                .await
                .unwrap();
        }
    })
    .await
    .expect("network backpressure must not reach the authoritative actor");
    task.abort();
}

#[tokio::test]
async fn daemon_restores_persisted_agent_snapshot_cursors() {
    let directory = TempDir::new().unwrap();
    let database = directory.path().join("state.sqlite3");
    let agent_id = AgentId::new("restored-agent").unwrap();
    {
        let store = Store::open_at(&database, 100).unwrap();
        store
            .upsert_agent(&AgentRecord {
                agent_id: agent_id.clone(),
                lifecycle: AgentLifecycle::Running,
                process_id: Some(42),
                active_run_id: None,
                created_at_ms: 10,
                updated_at_ms: 90,
            })
            .unwrap();
        store
            .upsert_session(&SessionResumeRecord {
                agent_id: agent_id.clone(),
                session_id: "session-1".into(),
                session_file: "/tmp/session.jsonl".into(),
                revision: StateRevision(9),
                event_sequence: EventSequence(12),
                updated_at_ms: 90,
            })
            .unwrap();
    }
    let store = Arc::new(Mutex::new(Store::open_at(&database, 200).unwrap()));
    let address = "127.0.0.1:8765".parse().unwrap();
    let server = DaemonServer::new(
        TransportConfig {
            bind_address: address,
            public_endpoint: "ws://127.0.0.1:8765/control".into(),
            tls_mode: TlsMode::DevelopmentPlaintext {
                local_endpoint: address,
            },
        },
        test_session_config(),
        store,
        RuntimeConfig::new("unused-omp"),
    )
    .unwrap();
    let snapshot = server
        .controller()
        .registry()
        .get(&agent_id)
        .unwrap()
        .snapshot()
        .await
        .unwrap();
    assert_eq!(snapshot.lifecycle, AgentLifecycle::Interrupted);
    assert_eq!(snapshot.revision, StateRevision(9));
    assert_eq!(snapshot.event_sequence, EventSequence(12));
    assert_eq!(snapshot.session.unwrap().session_id, "session-1");
}

struct PlainServer {
    address: std::net::SocketAddr,
    store: Arc<Mutex<Store>>,
    task: JoinHandle<Result<(), ompd::ServerError>>,
    _directory: TempDir,
}

impl PlainServer {
    async fn start() -> Self {
        Self::start_with_session(test_session_config()).await
    }

    async fn start_with_session(session: ServerSessionConfig) -> Self {
        let directory = TempDir::new().unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let store = Arc::new(Mutex::new(
            Store::open_at(directory.path().join("state.sqlite3"), unix_time_ms()).unwrap(),
        ));
        let server = DaemonServer::new(
            TransportConfig {
                bind_address: address,
                public_endpoint: format!("ws://{address}/control"),
                tls_mode: TlsMode::DevelopmentPlaintext {
                    local_endpoint: address,
                },
            },
            session,
            Arc::clone(&store),
            RuntimeConfig::new("unused-omp"),
        )
        .unwrap();
        let task = tokio::spawn(server.serve_with_listener(listener));
        tokio::task::yield_now().await;
        Self {
            address,
            store,
            task,
            _directory: directory,
        }
    }

    fn url(&self) -> String {
        format!("ws://{}/control", self.address)
    }

    fn stop(self) {
        self.task.abort();
    }
}

fn test_session_config() -> ServerSessionConfig {
    ServerSessionConfig {
        authentication_timeout: Duration::from_secs(1),
        heartbeat_interval: Duration::from_secs(60),
        slow_client_timeout: Duration::from_secs(1),
        ..ServerSessionConfig::default()
    }
}

fn observe_only() -> DeviceScopes {
    DeviceScopes {
        observe: true,
        prompt: false,
        mutate_session: false,
        stop_agent: false,
        answer_ui: false,
        administer_devices: false,
    }
}

fn device_record(name: &str, scopes: DeviceScopes) -> DeviceRecord {
    DeviceRecord {
        device_id: DeviceId::new(format!("{name}-id")).unwrap(),
        name: name.into(),
        platform: ClientPlatform::Desktop,
        scopes,
        created_at_ms: unix_time_ms(),
        last_seen_at_ms: None,
        revoked_at_ms: None,
    }
}

fn hello(authentication: ClientAuthentication) -> ClientFrame {
    ClientFrame::Hello(ClientHello {
        supported_versions: vec![ProtocolVersion::CURRENT],
        client: ClientDescriptor {
            name: "test-client".into(),
            version: "1".into(),
            platform: ClientPlatform::Desktop,
            capabilities: ClientCapabilities {
                requested: BTreeSet::new(),
            },
        },
        authentication,
        resume: ResumeState::default(),
    })
}

fn device_hello(device_id: &DeviceId, token: DeviceToken) -> ClientFrame {
    hello(ClientAuthentication::Device {
        device_id: device_id.clone(),
        token,
    })
}

fn pair_hello(
    pairing_id: omp_control_protocol::PairingId,
    secret: omp_control_protocol::PairingSecret,
) -> ClientFrame {
    hello(ClientAuthentication::Pair {
        pairing_id,
        secret,
        device: DeviceDescriptor {
            name: "test-phone".into(),
            platform: ClientPlatform::Mobile,
        },
    })
}

async fn send_client<S>(
    socket: &mut WebSocketStream<S>,
    frame: &ClientFrame,
    phase: ConnectionPhase,
) where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let bytes = FrameLimits::default().codec(phase).encode(frame).unwrap();
    socket.send(Message::Binary(bytes.into())).await.unwrap();
}

async fn receive_server<S>(socket: &mut WebSocketStream<S>) -> ServerFrame
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    loop {
        match socket.next().await.unwrap().unwrap() {
            Message::Binary(bytes) => {
                return FrameLimits::default()
                    .codec(ConnectionPhase::Authenticated)
                    .decode(&bytes)
                    .unwrap();
            }
            Message::Ping(payload) => socket.send(Message::Pong(payload)).await.unwrap(),
            other => panic!("unexpected WebSocket message: {other:?}"),
        }
    }
}

struct Certificates {
    certificate_path: std::path::PathBuf,
    private_key_path: std::path::PathBuf,
    der: rustls::pki_types::CertificateDer<'static>,
}

fn certificates(directory: &TempDir) -> Certificates {
    let rcgen::CertifiedKey { cert, signing_key } =
        rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    let certificate_path = directory.path().join("certificate.pem");
    let private_key_path = directory.path().join("private-key.pem");
    std::fs::write(&certificate_path, cert.pem()).unwrap();
    std::fs::write(&private_key_path, signing_key.serialize_pem()).unwrap();
    Certificates {
        certificate_path,
        private_key_path,
        der: cert.der().clone(),
    }
}
