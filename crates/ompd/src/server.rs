use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    sync::Arc,
    time::{Duration, Instant},
};

use axum::{
    Router,
    extract::{
        State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    response::Response,
    routing::get,
};
use futures_util::{SinkExt, StreamExt, stream::SplitSink};
use omp_control_plane::{AgentActorConfig, AgentRegistry, SubscriptionError};
use omp_control_protocol::{
    AgentUpdate, CAPABILITY_EVENT_REPLAY, CAPABILITY_INTERACTION_LEASES, CAPABILITY_STATE_DELTAS,
    ClientAuthentication, ClientFrame, ConnectionId, ConnectionPhase, ControlRequest,
    DeviceCredential, DeviceScopes, EventEnvelope, FrameLimits, Ping, Pong, ProtocolError,
    ReplayGap, ReplayGapReason, RequestEnvelope, ResponseEnvelope, ResponseOutcome,
    ServerCapabilities, ServerFrame, ServerWelcome, StateSnapshot, SubscribeRequest,
    SubscriptionStart, UiInteractionEnvelope, UiResponseEnvelope, negotiate_client_hello,
};
use omp_rpc::{ServerMessage, SessionEvent};
use omp_runtime::RuntimeConfig;
use parking_lot::Mutex;
use tokio::{
    sync::{mpsc, watch},
    task::JoinHandle,
    time,
};
use uuid::Uuid;

use crate::{
    controller::{ControllerError, DaemonController, unix_time_ms},
    persistence::{DeviceRecord, OperationClaim, OperationKey, Store},
    transport::{TlsMode, TransportConfig, TransportConfigError, load_rustls_config},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ServerSessionConfig {
    pub frame_limits: FrameLimits,
    pub authentication_timeout: Duration,
    pub heartbeat_interval: Duration,
    pub slow_client_timeout: Duration,
    pub outbound_capacity: usize,
}

impl Default for ServerSessionConfig {
    fn default() -> Self {
        Self {
            frame_limits: FrameLimits::default(),
            authentication_timeout: Duration::from_secs(5),
            heartbeat_interval: Duration::from_secs(15),
            slow_client_timeout: Duration::from_secs(10),
            outbound_capacity: 64,
        }
    }
}

#[derive(Clone, Debug)]
pub struct DaemonServer {
    transport: TransportConfig,
    state: AppState,
}

impl DaemonServer {
    pub fn new(
        transport: TransportConfig,
        session: ServerSessionConfig,
        store: Arc<Mutex<Store>>,
        runtime_config: RuntimeConfig,
    ) -> Result<Self, ServerError> {
        let registry = AgentRegistry::new(AgentActorConfig::default());
        let controller = DaemonController::new(registry, runtime_config, Arc::clone(&store))?;
        Ok(Self {
            transport,
            state: AppState {
                store,
                controller,
                session,
            },
        })
    }

    #[must_use]
    pub fn controller(&self) -> &DaemonController {
        &self.state.controller
    }

    pub fn router(&self) -> Router {
        Router::new()
            .route("/control", get(control_socket))
            .with_state(self.state.clone())
    }

    pub async fn serve(self) -> Result<(), ServerError> {
        self.transport.validate()?;
        let listener = std::net::TcpListener::bind(self.transport.bind_address)?;
        self.serve_with_listener(listener).await
    }

    pub async fn serve_with_listener(
        self,
        listener: std::net::TcpListener,
    ) -> Result<(), ServerError> {
        self.transport.validate()?;
        let app = self.router();
        match &self.transport.tls_mode {
            TlsMode::CertificateFiles {
                certificate,
                private_key,
            }
            | TlsMode::PinnedSelfSigned {
                certificate,
                private_key,
            } => {
                let tls = load_rustls_config(certificate, private_key)?;
                axum_server::from_tcp_rustls(listener, tls)
                    .serve(app.into_make_service())
                    .await
                    .map_err(ServerError::Io)
            }
            TlsMode::TrustedReverseProxy { .. } | TlsMode::DevelopmentPlaintext { .. } => {
                listener.set_nonblocking(true)?;
                let listener = tokio::net::TcpListener::from_std(listener)?;
                axum::serve(listener, app.into_make_service())
                    .await
                    .map_err(ServerError::Io)
            }
        }
    }
}

#[derive(Clone, Debug)]
struct AppState {
    store: Arc<Mutex<Store>>,
    controller: DaemonController,
    session: ServerSessionConfig,
}

async fn control_socket(State(state): State<AppState>, upgrade: WebSocketUpgrade) -> Response {
    let max_frame = state.session.frame_limits.post_auth().get() as usize;
    upgrade
        .max_message_size(max_frame)
        .max_frame_size(max_frame)
        .on_upgrade(move |socket| run_connection(socket, state))
}

async fn run_connection(mut socket: WebSocket, state: AppState) {
    let Some((hello, version, authenticated)) = authenticate_first_frame(&mut socket, &state).await
    else {
        let _ = socket.send(Message::Close(None)).await;
        return;
    };
    let server_id = { state.store.lock().server_id().clone() };
    let capabilities = ServerCapabilities::negotiate(
        &available_capabilities(),
        &hello.client.capabilities,
        state.session.frame_limits.post_auth().get(),
    );
    let welcome = ServerFrame::Welcome(ServerWelcome {
        protocol_version: version,
        server_id,
        connection_id: ConnectionId::new(Uuid::new_v4().to_string())
            .expect("UUID connection IDs are non-empty"),
        device_id: authenticated.device.device_id.clone(),
        capabilities,
        heartbeat_interval_ms: state
            .session
            .heartbeat_interval
            .as_millis()
            .try_into()
            .unwrap_or(u64::MAX),
        issued_credential: authenticated.issued_credential,
    });
    let codec = state
        .session
        .frame_limits
        .codec(ConnectionPhase::Authenticated);
    let Ok(welcome) = codec.encode(&welcome) else {
        let _ = socket.send(Message::Close(None)).await;
        return;
    };
    if socket.send(Message::Binary(welcome.into())).await.is_err() {
        return;
    }

    let (writer, mut reader) = socket.split();
    let (outbound, queues) = Outbound::new(state.session.outbound_capacity);
    let (activity_tx, activity_rx) = watch::channel(Instant::now());
    let mut writer_task = tokio::spawn(run_writer(
        writer,
        queues,
        codec,
        state.session,
        activity_rx,
    ));
    let mut subscriptions: BTreeMap<_, JoinHandle<()>> = BTreeMap::new();

    loop {
        tokio::select! {
            writer_result = &mut writer_task => {
                let _ = writer_result;
                break;
            }
            incoming = reader.next() => {
                let Some(incoming) = incoming else { break };
                let Ok(message) = incoming else { break };
                let _ = activity_tx.send(Instant::now());
                match message {
                    Message::Binary(bytes) => {
                        let Ok(frame) = codec.decode::<ClientFrame>(&bytes) else {
                            let _ = outbound.control(ServerFrame::Error(protocol_error(
                                "invalid_frame",
                                "client frame is not valid authenticated CBOR",
                                false,
                            ))).await;
                            break;
                        };
                        if !handle_client_frame(
                            frame,
                            &state,
                            &authenticated.device,
                            &outbound,
                            &mut subscriptions,
                        ).await {
                            break;
                        }
                    }
                    Message::Ping(payload) => {
                        if outbound.websocket(Message::Pong(payload)).await.is_err() {
                            break;
                        }
                    }
                    Message::Pong(_) => {}
                    Message::Close(_) => break,
                    Message::Text(_) => {
                        let _ = outbound.control(ServerFrame::Error(protocol_error(
                            "binary_frames_required",
                            "control protocol accepts binary CBOR frames only",
                            false,
                        ))).await;
                        break;
                    }
                }
            }
        }
    }

    for (_, task) in subscriptions {
        task.abort();
    }
    writer_task.abort();
}

async fn authenticate_first_frame(
    socket: &mut WebSocket,
    state: &AppState,
) -> Option<(
    omp_control_protocol::ClientHello,
    omp_control_protocol::ProtocolVersion,
    Authenticated,
)> {
    let incoming = time::timeout(state.session.authentication_timeout, socket.recv())
        .await
        .ok()?;
    let incoming = incoming?;
    let message = incoming.ok()?;
    let Message::Binary(bytes) = message else {
        return None;
    };
    if bytes.len() > state.session.frame_limits.pre_auth().get() as usize {
        return None;
    }
    let codec = state.session.frame_limits.codec(ConnectionPhase::PreAuth);
    let frame = codec.decode::<ClientFrame>(&bytes).ok()?;
    let (hello, version) = negotiate_client_hello(&frame).ok()?;
    let hello = hello.clone();
    let authenticated = match &hello.authentication {
        ClientAuthentication::Device { device_id, token } => {
            let device = state
                .store
                .lock()
                .authenticate_device(device_id, token, unix_time_ms())
                .ok()?;
            Authenticated {
                device,
                issued_credential: None,
            }
        }
        ClientAuthentication::Pair {
            pairing_id,
            secret,
            device,
        } => {
            let store = state.store.lock();
            let credential = store
                .exchange_pairing(pairing_id, secret, device, unix_time_ms())
                .ok()?;
            let record = store.device(&credential.device_id).ok()??;
            Authenticated {
                device: record,
                issued_credential: Some(credential),
            }
        }
    };
    Some((hello, version, authenticated))
}

#[derive(Clone, Debug)]
struct Authenticated {
    device: DeviceRecord,
    issued_credential: Option<DeviceCredential>,
}

async fn handle_client_frame(
    frame: ClientFrame,
    state: &AppState,
    device: &DeviceRecord,
    outbound: &Outbound,
    subscriptions: &mut BTreeMap<omp_control_protocol::AgentId, JoinHandle<()>>,
) -> bool {
    match frame {
        ClientFrame::Request(request) => {
            let response = execute_request(state, device, request).await;
            outbound
                .response(ServerFrame::Response(response))
                .await
                .is_ok()
        }
        ClientFrame::Subscribe(request) => {
            if !device.scopes.observe {
                return outbound
                    .control(ServerFrame::Error(permission_error()))
                    .await
                    .is_ok();
            }
            subscribe(request, state, outbound, subscriptions).await;
            true
        }
        ClientFrame::Unsubscribe(request) => {
            if !device.scopes.observe {
                return outbound
                    .control(ServerFrame::Error(permission_error()))
                    .await
                    .is_ok();
            }
            if let Some(task) = subscriptions.remove(&request.agent_id) {
                task.abort();
            }
            true
        }
        ClientFrame::UiResponse(UiResponseEnvelope {
            agent_id,
            request_id,
            operation_id,
            holder,
            response,
        }) => {
            let response = execute_request(
                state,
                device,
                RequestEnvelope {
                    request_id,
                    operation_id: Some(operation_id),
                    request: ControlRequest::RespondToUi {
                        agent_id,
                        holder,
                        response,
                    },
                },
            )
            .await;
            outbound
                .response(ServerFrame::Response(response))
                .await
                .is_ok()
        }
        ClientFrame::Ping(Ping { nonce }) => outbound
            .control(ServerFrame::Pong(Pong { nonce }))
            .await
            .is_ok(),
        ClientFrame::Hello(_) => {
            let _ = outbound
                .control(ServerFrame::Error(protocol_error(
                    "unexpected_hello",
                    "hello is valid only as the first frame",
                    false,
                )))
                .await;
            false
        }
    }
}

async fn execute_request(
    state: &AppState,
    device: &DeviceRecord,
    envelope: RequestEnvelope,
) -> ResponseEnvelope {
    let request_id = envelope.request_id.clone();
    let outcome = if let Err(error) = envelope.validate() {
        ResponseOutcome::Error(protocol_error("invalid_request", &error.to_string(), false))
    } else if !authorized(&device.scopes, &envelope.request) {
        ResponseOutcome::Error(permission_error())
    } else if let Some(operation_id) = envelope.operation_id {
        let key = OperationKey {
            device_id: device.device_id.clone(),
            operation_id,
        };
        let claim = state
            .store
            .lock()
            .claim_operation(&key, &envelope.request, unix_time_ms());
        match claim {
            Ok(OperationClaim::Execute) => {
                let outcome = state.controller.execute(envelope.request).await;
                let completion =
                    state
                        .store
                        .lock()
                        .complete_operation(&key, &outcome, unix_time_ms());
                if let Err(error) = completion {
                    ResponseOutcome::Error(protocol_error(
                        "operation_outcome_not_persisted",
                        &error.to_string(),
                        false,
                    ))
                } else {
                    outcome
                }
            }
            Ok(OperationClaim::Completed(outcome)) => outcome,
            Ok(OperationClaim::InProgress) => ResponseOutcome::Error(protocol_error(
                "operation_in_progress",
                "the operation is still in progress",
                true,
            )),
            Ok(OperationClaim::Indeterminate) => ResponseOutcome::Error(protocol_error(
                "operation_indeterminate",
                "the daemon restarted before recording the operation outcome",
                false,
            )),
            Err(error) => ResponseOutcome::Error(protocol_error(
                "operation_conflict",
                &error.to_string(),
                false,
            )),
        }
    } else {
        state.controller.execute(envelope.request).await
    };
    ResponseEnvelope {
        request_id,
        outcome,
    }
}

fn authorized(scopes: &DeviceScopes, request: &ControlRequest) -> bool {
    match request {
        ControlRequest::ListAgents | ControlRequest::GetAgent { .. } => scopes.observe,
        ControlRequest::LaunchAgent { .. } | ControlRequest::SwitchSession { .. } => {
            scopes.mutate_session
        }
        ControlRequest::StopAgent { .. } | ControlRequest::Abort { .. } => scopes.stop_agent,
        ControlRequest::Prompt { .. }
        | ControlRequest::Steer { .. }
        | ControlRequest::FollowUp { .. } => scopes.prompt,
        ControlRequest::RespondToUi { .. }
        | ControlRequest::AcquireInteractionLease { .. }
        | ControlRequest::ReleaseInteractionLease { .. } => scopes.answer_ui,
    }
}

async fn subscribe(
    request: SubscribeRequest,
    state: &AppState,
    outbound: &Outbound,
    subscriptions: &mut BTreeMap<omp_control_protocol::AgentId, JoinHandle<()>>,
) {
    let Some(agent) = state.controller.registry().get(&request.agent_id) else {
        let _ = outbound
            .control(ServerFrame::Error(protocol_error(
                "agent_not_found",
                "subscription agent was not found",
                false,
            )))
            .await;
        return;
    };
    let Ok(mut subscription) = agent.subscribe(request.cursor).await else {
        let _ = outbound
            .control(ServerFrame::Error(protocol_error(
                "subscription_failed",
                "could not create agent subscription",
                true,
            )))
            .await;
        return;
    };
    if let Some(previous) = subscriptions.remove(&request.agent_id) {
        previous.abort();
    }
    emit_subscription_start(subscription.start().clone(), outbound).await;
    let outbound = outbound.clone();
    let task = tokio::spawn(async move {
        loop {
            match subscription.recv().await {
                Ok(update) => {
                    if !emit_update(update, &outbound) {
                        send_slow_consumer_gap(&agent, &outbound).await;
                        break;
                    }
                }
                Err(SubscriptionError::ResyncRequired) => {
                    send_slow_consumer_gap(&agent, &outbound).await;
                    break;
                }
                Err(SubscriptionError::Closed) => break,
            }
        }
    });
    subscriptions.insert(request.agent_id, task);
}

async fn emit_subscription_start(start: SubscriptionStart, outbound: &Outbound) {
    match start {
        SubscriptionStart::Snapshot(snapshot) => {
            let _ = outbound.state(ServerFrame::Snapshot(StateSnapshot {
                agents: vec![snapshot],
            }));
        }
        SubscriptionStart::Replay(updates) => {
            for update in updates {
                if !emit_update(update, outbound) {
                    break;
                }
            }
        }
        SubscriptionStart::ResyncRequired(snapshot) => {
            let _ = outbound
                .control(ServerFrame::ReplayGap(ReplayGap {
                    agent_id: snapshot.agent_id.clone(),
                    current_revision: snapshot.revision,
                    current_event_sequence: snapshot.event_sequence,
                    reason: ReplayGapReason::NonContiguousCursor,
                }))
                .await;
            let _ = outbound.state(ServerFrame::Snapshot(StateSnapshot {
                agents: vec![snapshot],
            }));
        }
    }
}

fn emit_update(update: AgentUpdate, outbound: &Outbound) -> bool {
    match update {
        AgentUpdate::Delta {
            event_sequence,
            delta,
        } => outbound.state(ServerFrame::Delta(omp_control_protocol::DeltaEnvelope {
            event_sequence,
            delta,
        })),
        AgentUpdate::Event {
            agent_id,
            event_sequence,
            event,
        } => match event {
            ServerMessage::ExtensionUi(request) => {
                outbound.interaction(ServerFrame::InteractionRequest(UiInteractionEnvelope {
                    agent_id,
                    event_sequence,
                    request,
                }))
            }
            event => {
                let is_streaming = matches!(
                    event,
                    ServerMessage::SessionEvent(
                        SessionEvent::MessageUpdate { .. }
                            | SessionEvent::ToolExecutionUpdate { .. }
                    )
                );
                let frame = ServerFrame::Event(EventEnvelope {
                    agent_id,
                    event_sequence,
                    event,
                });
                if is_streaming {
                    outbound.streaming(frame)
                } else {
                    outbound.event(frame)
                }
            }
        },
    }
}

async fn send_slow_consumer_gap(agent: &omp_control_plane::AgentHandle, outbound: &Outbound) {
    if let Ok(snapshot) = agent.snapshot().await {
        let _ = outbound
            .control(ServerFrame::ReplayGap(ReplayGap {
                agent_id: snapshot.agent_id,
                current_revision: snapshot.revision,
                current_event_sequence: snapshot.event_sequence,
                reason: ReplayGapReason::SlowConsumer,
            }))
            .await;
    }
}

#[derive(Clone, Debug)]
struct Outbound {
    control: mpsc::Sender<OutboundPayload>,
    response: mpsc::Sender<OutboundPayload>,
    state: mpsc::Sender<OutboundPayload>,
    interaction: mpsc::Sender<OutboundPayload>,
    event: mpsc::Sender<OutboundPayload>,
    streaming: mpsc::Sender<OutboundPayload>,
}

impl Outbound {
    fn new(capacity: usize) -> (Self, OutboundQueues) {
        let capacity = capacity.max(1);
        let (control, control_rx) = mpsc::channel(capacity);
        let (response, response_rx) = mpsc::channel(capacity);
        let (state, state_rx) = mpsc::channel(capacity);
        let (interaction, interaction_rx) = mpsc::channel(capacity);
        let (event, event_rx) = mpsc::channel(capacity);
        let (streaming, streaming_rx) = mpsc::channel(capacity);
        (
            Self {
                control,
                response,
                state,
                interaction,
                event,
                streaming,
            },
            OutboundQueues {
                control: control_rx,
                response: response_rx,
                state: state_rx,
                interaction: interaction_rx,
                event: event_rx,
                streaming: streaming_rx,
            },
        )
    }

    async fn control(&self, frame: ServerFrame) -> Result<(), ()> {
        self.control
            .send(OutboundPayload::Protocol(frame))
            .await
            .map_err(|_| ())
    }

    async fn response(&self, frame: ServerFrame) -> Result<(), ()> {
        self.response
            .send(OutboundPayload::Protocol(frame))
            .await
            .map_err(|_| ())
    }

    async fn websocket(&self, message: Message) -> Result<(), ()> {
        self.control
            .send(OutboundPayload::WebSocket(message))
            .await
            .map_err(|_| ())
    }

    fn state(&self, frame: ServerFrame) -> bool {
        self.state
            .try_send(OutboundPayload::Protocol(frame))
            .is_ok()
    }

    fn interaction(&self, frame: ServerFrame) -> bool {
        self.interaction
            .try_send(OutboundPayload::Protocol(frame))
            .is_ok()
    }

    fn event(&self, frame: ServerFrame) -> bool {
        self.event
            .try_send(OutboundPayload::Protocol(frame))
            .is_ok()
    }

    fn streaming(&self, frame: ServerFrame) -> bool {
        self.streaming
            .try_send(OutboundPayload::Protocol(frame))
            .is_ok()
    }
}

#[derive(Debug)]
enum OutboundPayload {
    Protocol(ServerFrame),
    WebSocket(Message),
}

#[derive(Debug)]
struct OutboundQueues {
    control: mpsc::Receiver<OutboundPayload>,
    response: mpsc::Receiver<OutboundPayload>,
    state: mpsc::Receiver<OutboundPayload>,
    interaction: mpsc::Receiver<OutboundPayload>,
    event: mpsc::Receiver<OutboundPayload>,
    streaming: mpsc::Receiver<OutboundPayload>,
}

async fn run_writer(
    mut writer: SplitSink<WebSocket, Message>,
    mut queues: OutboundQueues,
    codec: omp_control_protocol::CborCodec,
    config: ServerSessionConfig,
    activity: watch::Receiver<Instant>,
) {
    let mut heartbeat = time::interval(config.heartbeat_interval);
    heartbeat.set_missed_tick_behavior(time::MissedTickBehavior::Delay);
    heartbeat.tick().await;
    loop {
        let payload = tokio::select! {
            biased;
            Some(payload) = queues.control.recv() => payload,
            Some(payload) = queues.response.recv() => payload,
            Some(payload) = queues.state.recv() => payload,
            Some(payload) = queues.interaction.recv() => payload,
            Some(payload) = queues.event.recv() => payload,
            Some(payload) = queues.streaming.recv() => payload,
            _ = heartbeat.tick() => {
                if Instant::now().saturating_duration_since(*activity.borrow())
                    > config.heartbeat_interval.saturating_mul(2)
                {
                    break;
                }
                OutboundPayload::WebSocket(Message::Ping(
                    unix_time_ms().to_be_bytes().to_vec().into(),
                ))
            }
        };
        let message = match payload {
            OutboundPayload::Protocol(frame) => match codec.encode(&frame) {
                Ok(bytes) => Message::Binary(bytes.into()),
                Err(_) => break,
            },
            OutboundPayload::WebSocket(message) => message,
        };
        match time::timeout(config.slow_client_timeout, writer.send(message)).await {
            Ok(Ok(())) => {}
            Ok(Err(_)) | Err(_) => break,
        }
    }
    let _ = time::timeout(
        config.slow_client_timeout,
        writer.send(Message::Close(None)),
    )
    .await;
}

fn available_capabilities() -> BTreeSet<String> {
    BTreeSet::from([
        CAPABILITY_STATE_DELTAS.to_owned(),
        CAPABILITY_EVENT_REPLAY.to_owned(),
        CAPABILITY_INTERACTION_LEASES.to_owned(),
    ])
}

fn permission_error() -> ProtocolError {
    protocol_error(
        "permission_denied",
        "device credential does not grant this operation",
        false,
    )
}

fn protocol_error(code: &str, message: &str, retryable: bool) -> ProtocolError {
    ProtocolError {
        code: code.to_owned(),
        message: message.to_owned(),
        retryable,
    }
}

#[derive(Debug)]
pub enum ServerError {
    Transport(TransportConfigError),
    Controller(ControllerError),
    Io(std::io::Error),
}

impl fmt::Display for ServerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Transport(error) => error.fmt(formatter),
            Self::Controller(error) => error.fmt(formatter),
            Self::Io(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for ServerError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Transport(error) => Some(error),
            Self::Controller(error) => Some(error),
            Self::Io(error) => Some(error),
        }
    }
}

impl From<TransportConfigError> for ServerError {
    fn from(error: TransportConfigError) -> Self {
        Self::Transport(error)
    }
}

impl From<ControllerError> for ServerError {
    fn from(error: ControllerError) -> Self {
        Self::Controller(error)
    }
}

impl From<std::io::Error> for ServerError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}
