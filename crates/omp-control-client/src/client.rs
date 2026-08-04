use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    num::NonZeroU32,
    sync::Arc,
    time::Duration,
};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use futures_channel::{mpsc, oneshot};
use futures_util::{StreamExt, future::Either, future::select, pin_mut};
use omp_control_protocol::{
    AgentId, CborCodec, ClientAuthentication, ClientFrame, ClientHello, ControlRequest,
    ControlResponse, DeviceCredential, DeviceId, OperationId, ProtocolError, ProtocolVersion,
    RequestEnvelope, RequestId, ResponseEnvelope, ResponseOutcome, ResumeState, ServerFrame,
    ServerWelcome, StateSnapshot, SubscribeRequest, SubscriptionCursor, UiInteractionEnvelope,
    UiResponseEnvelope, UnsubscribeRequest,
};
use omp_rpc::ExtensionUiResponseFrame;
use parking_lot::{Mutex, RwLock};

use crate::{
    AuthenticationSource, BinaryWebSocket, ClientConfig, CredentialStore, ReplicatedState,
    ReplicationEffect, ReplicationError, SocketEvent, TransportError, WebSocketAdapter,
};

#[derive(Clone)]
pub struct ClientHandle {
    commands: mpsc::UnboundedSender<ClientCommand>,
    state: Arc<RwLock<ReplicatedState>>,
    status: Arc<RwLock<ConnectionStatus>>,
    ids: Arc<Mutex<IdGenerator>>,
    subscribers: Arc<Mutex<Vec<mpsc::Sender<ClientEvent>>>>,
    event_subscriber_capacity: usize,
}

impl ClientHandle {
    #[must_use]
    pub fn state(&self) -> ReplicatedState {
        self.state.read().clone()
    }

    #[must_use]
    pub fn status(&self) -> ConnectionStatus {
        self.status.read().clone()
    }

    pub fn events(&self) -> Result<mpsc::Receiver<ClientEvent>, RequestError> {
        let (mut sender, receiver) = mpsc::channel(self.event_subscriber_capacity.max(1));
        let mut subscribers = self.subscribers.lock();
        let _ = sender.try_send(ClientEvent::ConnectionChanged(self.status()));
        subscribers.push(sender);
        Ok(receiver)
    }

    pub fn subscribe(&self, agent_id: AgentId) -> Result<(), RequestError> {
        self.commands
            .unbounded_send(ClientCommand::Subscribe(agent_id))
            .map_err(|_| RequestError::ClientStopped)
    }

    pub fn unsubscribe(&self, agent_id: AgentId) -> Result<(), RequestError> {
        self.commands
            .unbounded_send(ClientCommand::Unsubscribe(agent_id))
            .map_err(|_| RequestError::ClientStopped)
    }

    pub async fn request(&self, request: ControlRequest) -> Result<ControlResponse, RequestError> {
        let operation_id = if request.is_mutating() {
            Some(self.next_operation_id()?)
        } else {
            None
        };
        self.request_with_operation_id(request, operation_id).await
    }

    pub async fn request_with_operation_id(
        &self,
        request: ControlRequest,
        operation_id: Option<OperationId>,
    ) -> Result<ControlResponse, RequestError> {
        validate_operation_id(&request, operation_id.as_ref())?;
        let (sender, receiver) = oneshot::channel();
        self.commands
            .unbounded_send(ClientCommand::Request {
                payload: PendingPayload::Request(request),
                operation_id,
                response: sender,
            })
            .map_err(|_| RequestError::ClientStopped)?;
        receiver.await.unwrap_or(Err(RequestError::ClientStopped))
    }

    pub async fn respond_to_ui(
        &self,
        agent_id: AgentId,
        holder: omp_control_protocol::LeaseHolderId,
        response: ExtensionUiResponseFrame,
    ) -> Result<ControlResponse, RequestError> {
        let operation_id = self.next_operation_id()?;
        let (sender, receiver) = oneshot::channel();
        self.commands
            .unbounded_send(ClientCommand::Request {
                payload: PendingPayload::UiResponse {
                    agent_id,
                    holder,
                    response,
                },
                operation_id: Some(operation_id),
                response: sender,
            })
            .map_err(|_| RequestError::ClientStopped)?;
        receiver.await.unwrap_or(Err(RequestError::ClientStopped))
    }

    pub async fn shutdown(&self) -> Result<(), RequestError> {
        let (sender, receiver) = oneshot::channel();
        self.commands
            .unbounded_send(ClientCommand::Shutdown(sender))
            .map_err(|_| RequestError::ClientStopped)?;
        receiver.await.map_err(|_| RequestError::ClientStopped)
    }

    fn next_operation_id(&self) -> Result<OperationId, RequestError> {
        self.ids
            .lock()
            .next("operation")
            .and_then(|value| OperationId::new(value).map_err(|_| IdGenerationError::Exhausted))
            .map_err(|_| RequestError::IdentifierSpaceExhausted)
    }
}

pub struct ClientRunner<S> {
    config: ClientConfig,
    credentials: S,
    commands: mpsc::UnboundedReceiver<ClientCommand>,
    state: Arc<RwLock<ReplicatedState>>,
    status: Arc<RwLock<ConnectionStatus>>,
    ids: Arc<Mutex<IdGenerator>>,
    subscribers: Arc<Mutex<Vec<mpsc::Sender<ClientEvent>>>>,
    desired_subscriptions: BTreeSet<AgentId>,
    pending: BTreeMap<u64, PendingRequest>,
    response_index: BTreeMap<RequestId, u64>,
    next_pending: u64,
    active_credential: Option<DeviceCredential>,
}

impl<S> ClientRunner<S>
where
    S: CredentialStore,
{
    pub fn new(
        config: ClientConfig,
        credentials: S,
    ) -> Result<(ClientHandle, Self), ClientBuildError> {
        let ids = Arc::new(Mutex::new(IdGenerator::new()?));
        let state = Arc::new(RwLock::new(ReplicatedState::default()));
        let status = Arc::new(RwLock::new(ConnectionStatus::Disconnected { reason: None }));
        let (command_tx, command_rx) = mpsc::unbounded();
        let subscribers = Arc::new(Mutex::new(Vec::new()));
        let handle = ClientHandle {
            commands: command_tx,
            state: Arc::clone(&state),
            status: Arc::clone(&status),
            ids: Arc::clone(&ids),
            subscribers: Arc::clone(&subscribers),
            event_subscriber_capacity: config.event_subscriber_capacity,
        };
        Ok((
            handle,
            Self {
                config,
                credentials,
                commands: command_rx,
                state,
                status,
                ids,
                subscribers,
                desired_subscriptions: BTreeSet::new(),
                pending: BTreeMap::new(),
                response_index: BTreeMap::new(),
                next_pending: 0,
                active_credential: None,
            },
        ))
    }

    pub async fn run<A>(mut self, adapter: A) -> Result<(), ClientRunError>
    where
        A: WebSocketAdapter,
    {
        let mut reconnect_delay = self.config.reconnect.initial_delay;
        loop {
            self.set_status(ConnectionStatus::Connecting);
            let authentication = match self.authentication() {
                Ok(authentication) => authentication,
                Err(error) => return self.stop_with_error(error),
            };
            let mut socket = match self.connect(&adapter).await {
                Ok(Some(socket)) => socket,
                Ok(None) => return Ok(()),
                Err(ClientRunError::Transport(error)) => {
                    self.disconnected(Some(error));
                    if !self
                        .wait_before_reconnect(&adapter, reconnect_delay)
                        .await?
                    {
                        return Ok(());
                    }
                    reconnect_delay =
                        next_delay(reconnect_delay, self.config.reconnect.maximum_delay);
                    continue;
                }
                Err(error) => return self.stop_with_error(error),
            };
            let handshake = self.handshake(&mut socket, authentication).await;
            let (welcome, codec) = match handshake {
                Ok(Some(negotiated)) => negotiated,
                Ok(None) => {
                    let _ = socket.close().await;
                    return Ok(());
                }
                Err(HandshakeError::Transport(error)) => {
                    self.disconnected(Some(error.to_string()));
                    self.mark_pending_disconnected();
                    if !self
                        .wait_before_reconnect(&adapter, reconnect_delay)
                        .await?
                    {
                        return Ok(());
                    }
                    reconnect_delay =
                        next_delay(reconnect_delay, self.config.reconnect.maximum_delay);
                    continue;
                }
                Err(HandshakeError::Fatal(error)) => {
                    let _ = socket.close().await;
                    return self.stop_with_error(error);
                }
            };

            reconnect_delay = self.config.reconnect.initial_delay;
            self.set_connected(welcome.clone());
            if let Err(error) = self.initialize_connection(&mut socket, codec).await {
                self.disconnected(Some(error.to_string()));
                self.mark_pending_disconnected();
                if !self
                    .wait_before_reconnect(&adapter, reconnect_delay)
                    .await?
                {
                    return Ok(());
                }
                continue;
            }

            match self.run_connection(&mut socket, codec).await {
                SessionEnd::Shutdown => {
                    let _ = socket.close().await;
                    self.finish_shutdown();
                    return Ok(());
                }
                SessionEnd::Reconnect { reason, after } => {
                    self.disconnected(reason);
                    self.mark_pending_disconnected();
                    let delay = after.unwrap_or(reconnect_delay);
                    if !self.wait_before_reconnect(&adapter, delay).await? {
                        let _ = socket.close().await;
                        return Ok(());
                    }
                    reconnect_delay =
                        next_delay(reconnect_delay, self.config.reconnect.maximum_delay);
                }
                SessionEnd::Fatal(error) => {
                    let _ = socket.close().await;
                    return self.stop_with_error(error);
                }
            }
        }
    }

    fn authentication(&mut self) -> Result<ClientAuthentication, ClientRunError> {
        if let Some(credential) = &self.active_credential {
            return Ok(device_authentication(credential));
        }
        match &self.config.authentication {
            AuthenticationSource::StoredCredential => {
                let credential = self
                    .credentials
                    .load(&self.config.server_id)
                    .map_err(|error| ClientRunError::CredentialStorage(error.to_string()))?
                    .ok_or(ClientRunError::CredentialUnavailable)?;
                if credential.server_id != self.config.server_id {
                    return Err(ClientRunError::CredentialServerMismatch);
                }
                let authentication = device_authentication(&credential);
                self.active_credential = Some(credential);
                Ok(authentication)
            }
            AuthenticationSource::Pair(authentication) => Ok(authentication.clone()),
        }
    }

    async fn connect<A>(&mut self, adapter: &A) -> Result<Option<A::Socket>, ClientRunError>
    where
        A: WebSocketAdapter,
    {
        let target = self.config.target.clone();
        let connection = adapter.connect(&target);
        pin_mut!(connection);
        loop {
            let command = self.commands.next();
            pin_mut!(command);
            match select(connection.as_mut(), command).await {
                Either::Left((result, _)) => {
                    return result
                        .map(Some)
                        .map_err(|error| ClientRunError::Transport(error.to_string()));
                }
                Either::Right((command, _)) => {
                    if self.handle_offline_command(command)? {
                        self.finish_shutdown();
                        return Ok(None);
                    }
                }
            }
        }
    }

    async fn handshake<W>(
        &mut self,
        socket: &mut W,
        authentication: ClientAuthentication,
    ) -> Result<Option<(ServerWelcome, CborCodec)>, HandshakeError>
    where
        W: BinaryWebSocket,
    {
        let resume = ResumeState {
            subscriptions: self.resume_cursors(),
        };
        let hello = ClientFrame::Hello(ClientHello {
            supported_versions: vec![ProtocolVersion::CURRENT],
            client: self.config.client.clone(),
            authentication: authentication.clone(),
            resume,
        });
        let pre_auth = self
            .config
            .frame_limits
            .codec(omp_control_protocol::ConnectionPhase::PreAuth);
        send_frame(socket, pre_auth, &hello)
            .await
            .map_err(HandshakeError::Transport)?;
        let provisional = self
            .config
            .frame_limits
            .codec(omp_control_protocol::ConnectionPhase::Authenticated);

        loop {
            enum HandshakeInput {
                Command(Option<ClientCommand>),
                Socket(Result<SocketEvent, TransportError>),
            }
            let input = {
                let command = self.commands.next();
                let incoming = socket.receive();
                pin_mut!(command, incoming);
                match select(command, incoming).await {
                    Either::Left((command, _)) => HandshakeInput::Command(command),
                    Either::Right((incoming, _)) => HandshakeInput::Socket(incoming),
                }
            };
            match input {
                HandshakeInput::Command(command) => {
                    if self
                        .handle_offline_command(command)
                        .map_err(HandshakeError::Fatal)?
                    {
                        self.finish_shutdown();
                        return Ok(None);
                    }
                }
                HandshakeInput::Socket(Err(error)) => {
                    return Err(HandshakeError::Transport(error));
                }
                HandshakeInput::Socket(Ok(SocketEvent::Closed { reason })) => {
                    return Err(HandshakeError::Transport(TransportError::new(
                        reason.unwrap_or_else(|| "server closed during authentication".to_owned()),
                    )));
                }
                HandshakeInput::Socket(Ok(SocketEvent::Binary(bytes))) => {
                    let frame = provisional.decode::<ServerFrame>(&bytes).map_err(|error| {
                        HandshakeError::Fatal(ClientRunError::Protocol(error.to_string()))
                    })?;
                    let ServerFrame::Welcome(mut welcome) = frame else {
                        return Err(HandshakeError::Fatal(ClientRunError::Protocol(
                            "server did not send welcome as its first frame".to_owned(),
                        )));
                    };
                    if welcome.protocol_version != ProtocolVersion::CURRENT {
                        return Err(HandshakeError::Fatal(ClientRunError::Protocol(
                            "server selected an unsupported protocol version".to_owned(),
                        )));
                    }
                    if welcome.server_id != self.config.server_id {
                        return Err(HandshakeError::Fatal(
                            ClientRunError::ServerIdentityMismatch,
                        ));
                    }
                    self.accept_credential(&authentication, &welcome)?;
                    let maximum = welcome
                        .capabilities
                        .max_frame_bytes
                        .min(self.config.frame_limits.post_auth().get());
                    let maximum = NonZeroU32::new(maximum).ok_or_else(|| {
                        HandshakeError::Fatal(ClientRunError::Protocol(
                            "server advertised a zero-byte frame limit".to_owned(),
                        ))
                    })?;
                    welcome.issued_credential = None;
                    return Ok(Some((welcome, CborCodec::new(maximum))));
                }
            }
        }
    }

    fn accept_credential(
        &mut self,
        authentication: &ClientAuthentication,
        welcome: &ServerWelcome,
    ) -> Result<(), HandshakeError> {
        match authentication {
            ClientAuthentication::Pair { .. } => {
                let credential = welcome.issued_credential.as_ref().ok_or_else(|| {
                    HandshakeError::Fatal(ClientRunError::Protocol(
                        "pairing welcome did not include a device credential".to_owned(),
                    ))
                })?;
                if credential.server_id != self.config.server_id
                    || credential.device_id != welcome.device_id
                {
                    return Err(HandshakeError::Fatal(
                        ClientRunError::CredentialServerMismatch,
                    ));
                }
                self.credentials.save(credential).map_err(|error| {
                    HandshakeError::Fatal(ClientRunError::CredentialStorage(error.to_string()))
                })?;
                self.active_credential = Some(credential.clone());
            }
            ClientAuthentication::Device { device_id, .. } => {
                if *device_id != welcome.device_id {
                    return Err(HandshakeError::Fatal(
                        ClientRunError::CredentialServerMismatch,
                    ));
                }
            }
        }
        Ok(())
    }

    async fn initialize_connection<W>(
        &mut self,
        socket: &mut W,
        codec: CborCodec,
    ) -> Result<(), TransportError>
    where
        W: BinaryWebSocket,
    {
        let subscriptions: Vec<_> = self.desired_subscriptions.iter().cloned().collect();
        for agent_id in subscriptions {
            self.send_subscription(socket, codec, agent_id, true)
                .await?;
        }
        let pending: Vec<_> = self.pending.keys().copied().collect();
        for key in pending {
            self.send_pending(socket, codec, key).await?;
        }
        Ok(())
    }

    async fn run_connection<W>(&mut self, socket: &mut W, codec: CborCodec) -> SessionEnd
    where
        W: BinaryWebSocket,
    {
        loop {
            enum Input {
                Command(Option<ClientCommand>),
                Socket(Result<SocketEvent, TransportError>),
            }
            let input = {
                let command = self.commands.next();
                let incoming = socket.receive();
                pin_mut!(command, incoming);
                match select(command, incoming).await {
                    Either::Left((command, _)) => Input::Command(command),
                    Either::Right((incoming, _)) => Input::Socket(incoming),
                }
            };
            match input {
                Input::Command(command) => {
                    match self.handle_connected_command(command, socket, codec).await {
                        Ok(CommandAction::Continue) => {}
                        Ok(CommandAction::Shutdown) => return SessionEnd::Shutdown,
                        Err(error) => {
                            return SessionEnd::Reconnect {
                                reason: Some(error.to_string()),
                                after: None,
                            };
                        }
                    }
                }
                Input::Socket(Err(error)) => {
                    return SessionEnd::Reconnect {
                        reason: Some(error.to_string()),
                        after: None,
                    };
                }
                Input::Socket(Ok(SocketEvent::Closed { reason })) => {
                    return SessionEnd::Reconnect {
                        reason,
                        after: None,
                    };
                }
                Input::Socket(Ok(SocketEvent::Binary(bytes))) => {
                    let frame = match codec.decode::<ServerFrame>(&bytes) {
                        Ok(frame) => frame,
                        Err(error) => {
                            return SessionEnd::Fatal(ClientRunError::Protocol(error.to_string()));
                        }
                    };
                    if let Some(end) = self.handle_server_frame(frame, socket, codec).await {
                        return end;
                    }
                }
            }
        }
    }

    async fn handle_connected_command<W>(
        &mut self,
        command: Option<ClientCommand>,
        socket: &mut W,
        codec: CborCodec,
    ) -> Result<CommandAction, TransportError>
    where
        W: BinaryWebSocket,
    {
        let Some(command) = command else {
            self.fail_pending(RequestError::ClientStopped);
            return Ok(CommandAction::Shutdown);
        };
        match command {
            ClientCommand::Request {
                payload,
                operation_id,
                response,
            } => {
                let key = self.insert_pending(payload, operation_id, response);
                self.send_pending(socket, codec, key).await?;
            }
            ClientCommand::Subscribe(agent_id) => {
                if self.desired_subscriptions.insert(agent_id.clone()) {
                    self.send_subscription(socket, codec, agent_id, true)
                        .await?;
                }
            }
            ClientCommand::Unsubscribe(agent_id) => {
                if self.desired_subscriptions.remove(&agent_id) {
                    send_frame(
                        socket,
                        codec,
                        &ClientFrame::Unsubscribe(UnsubscribeRequest { agent_id }),
                    )
                    .await?;
                }
            }
            ClientCommand::Shutdown(acknowledge) => {
                let _ = acknowledge.send(());
                self.fail_pending(RequestError::ClientStopped);
                return Ok(CommandAction::Shutdown);
            }
        }
        Ok(CommandAction::Continue)
    }

    async fn handle_server_frame<W>(
        &mut self,
        frame: ServerFrame,
        socket: &mut W,
        codec: CborCodec,
    ) -> Option<SessionEnd>
    where
        W: BinaryWebSocket,
    {
        match frame {
            ServerFrame::Response(response) => self.complete_response(response),
            ServerFrame::Error(error) => self.broadcast(ClientEvent::ProtocolError(error)),
            ServerFrame::Pong(_) => {}
            ServerFrame::ServerShutdown(shutdown) => {
                return Some(SessionEnd::Reconnect {
                    reason: Some(shutdown.reason),
                    after: shutdown.reconnect_after_ms.map(Duration::from_millis),
                });
            }
            ServerFrame::Welcome(_) => {
                return Some(SessionEnd::Fatal(ClientRunError::Protocol(
                    "server sent a second welcome frame".to_owned(),
                )));
            }
            replicated => {
                let agent_id = frame_agent_id(&replicated);
                let result = self.state.write().apply(replicated);
                match result {
                    Ok(effects) => {
                        for effect in effects {
                            match effect {
                                ReplicationEffect::StateChanged(agent_id) => {
                                    self.broadcast(ClientEvent::StateChanged(agent_id));
                                }
                                ReplicationEffect::Event(event) => {
                                    self.broadcast(ClientEvent::AgentEvent(event));
                                }
                                ReplicationEffect::Interaction(interaction) => {
                                    self.broadcast(ClientEvent::InteractionRequest(interaction));
                                }
                                ReplicationEffect::ResyncRequired(agent_id) => {
                                    self.broadcast(ClientEvent::ResyncRequired(agent_id.clone()));
                                    if let Err(error) =
                                        self.send_subscription(socket, codec, agent_id, false).await
                                    {
                                        return Some(SessionEnd::Reconnect {
                                            reason: Some(error.to_string()),
                                            after: None,
                                        });
                                    }
                                }
                            }
                        }
                    }
                    Err(error) => {
                        self.broadcast(ClientEvent::ReplicationError(error));
                        if let Some(agent_id) = agent_id
                            && let Err(error) =
                                self.send_subscription(socket, codec, agent_id, false).await
                        {
                            return Some(SessionEnd::Reconnect {
                                reason: Some(error.to_string()),
                                after: None,
                            });
                        }
                    }
                }
            }
        }
        None
    }

    fn handle_offline_command(
        &mut self,
        command: Option<ClientCommand>,
    ) -> Result<bool, ClientRunError> {
        let Some(command) = command else {
            self.fail_pending(RequestError::ClientStopped);
            return Ok(true);
        };
        match command {
            ClientCommand::Request {
                payload,
                operation_id,
                response,
            } => {
                self.insert_pending(payload, operation_id, response);
            }
            ClientCommand::Subscribe(agent_id) => {
                self.desired_subscriptions.insert(agent_id);
            }
            ClientCommand::Unsubscribe(agent_id) => {
                self.desired_subscriptions.remove(&agent_id);
            }
            ClientCommand::Shutdown(acknowledge) => {
                let _ = acknowledge.send(());
                self.fail_pending(RequestError::ClientStopped);
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn insert_pending(
        &mut self,
        payload: PendingPayload,
        operation_id: Option<OperationId>,
        response: oneshot::Sender<Result<ControlResponse, RequestError>>,
    ) -> u64 {
        let key = self.next_pending;
        self.next_pending = self.next_pending.wrapping_add(1);
        self.pending.insert(
            key,
            PendingRequest {
                payload,
                operation_id,
                sent_request_id: None,
                response,
            },
        );
        key
    }

    async fn send_pending<W>(
        &mut self,
        socket: &mut W,
        codec: CborCodec,
        key: u64,
    ) -> Result<(), TransportError>
    where
        W: BinaryWebSocket,
    {
        let request_id = self
            .next_request_id()
            .map_err(|error| TransportError::new(error.to_string()))?;
        let frame = {
            let pending = self
                .pending
                .get(&key)
                .ok_or_else(|| TransportError::new("pending request disappeared before send"))?;
            pending.frame(request_id.clone())
        };
        send_frame(socket, codec, &frame).await?;
        if let Some(pending) = self.pending.get_mut(&key) {
            pending.sent_request_id = Some(request_id.clone());
            self.response_index.insert(request_id, key);
        }
        Ok(())
    }

    async fn send_subscription<W>(
        &mut self,
        socket: &mut W,
        codec: CborCodec,
        agent_id: AgentId,
        resume: bool,
    ) -> Result<(), TransportError>
    where
        W: BinaryWebSocket,
    {
        let cursor = resume
            .then(|| self.state.read().cursor(&agent_id))
            .flatten();
        send_frame(
            socket,
            codec,
            &ClientFrame::Subscribe(SubscribeRequest { agent_id, cursor }),
        )
        .await
    }

    fn complete_response(&mut self, response: ResponseEnvelope) {
        let Some(key) = self.response_index.remove(&response.request_id) else {
            self.broadcast(ClientEvent::ProtocolError(ProtocolError {
                code: "unexpected_response".to_owned(),
                message: "server response did not match an in-flight request".to_owned(),
                retryable: false,
            }));
            return;
        };
        let Some(pending) = self.pending.remove(&key) else {
            return;
        };
        let outcome = match response.outcome {
            ResponseOutcome::Success(response) => Ok(*response),
            ResponseOutcome::Error(error) => Err(RequestError::Protocol(error)),
        };
        let _ = pending.response.send(outcome);
    }

    fn resume_cursors(&self) -> Vec<SubscriptionCursor> {
        self.desired_subscriptions
            .iter()
            .filter_map(|agent_id| self.state.read().cursor(agent_id))
            .collect()
    }

    fn next_request_id(&self) -> Result<RequestId, IdGenerationError> {
        self.ids
            .lock()
            .next("request")
            .and_then(|value| RequestId::new(value).map_err(|_| IdGenerationError::Exhausted))
    }

    async fn wait_before_reconnect<A>(
        &mut self,
        adapter: &A,
        duration: Duration,
    ) -> Result<bool, ClientRunError>
    where
        A: WebSocketAdapter,
    {
        let sleep = adapter.sleep(duration);
        pin_mut!(sleep);
        loop {
            let command = self.commands.next();
            pin_mut!(command);
            match select(sleep.as_mut(), command).await {
                Either::Left(_) => return Ok(true),
                Either::Right((command, _)) => {
                    if self.handle_offline_command(command)? {
                        self.finish_shutdown();
                        return Ok(false);
                    }
                }
            }
        }
    }

    fn mark_pending_disconnected(&mut self) {
        self.response_index.clear();
        for pending in self.pending.values_mut() {
            pending.sent_request_id = None;
        }
    }

    fn set_connected(&mut self, welcome: ServerWelcome) {
        self.set_status(ConnectionStatus::Connected {
            server_id: welcome.server_id,
            device_id: welcome.device_id,
            protocol_version: welcome.protocol_version,
        });
    }

    fn disconnected(&mut self, reason: Option<String>) {
        self.set_status(ConnectionStatus::Disconnected { reason });
    }

    fn finish_shutdown(&mut self) {
        self.set_status(ConnectionStatus::Stopped { reason: None });
    }

    fn stop_with_error(&mut self, error: ClientRunError) -> Result<(), ClientRunError> {
        self.fail_pending(RequestError::ClientStopped);
        self.set_status(ConnectionStatus::Stopped {
            reason: Some(error.to_string()),
        });
        Err(error)
    }

    fn fail_pending(&mut self, error: RequestError) {
        self.response_index.clear();
        for (_, pending) in std::mem::take(&mut self.pending) {
            let _ = pending.response.send(Err(error.clone()));
        }
    }

    fn set_status(&mut self, status: ConnectionStatus) {
        *self.status.write() = status.clone();
        self.broadcast(ClientEvent::ConnectionChanged(status));
    }

    fn broadcast(&self, event: ClientEvent) {
        self.subscribers
            .lock()
            .retain_mut(|subscriber| match subscriber.try_send(event.clone()) {
                Ok(()) => true,
                Err(error) => error.is_full(),
            });
    }
}

fn device_authentication(credential: &DeviceCredential) -> ClientAuthentication {
    ClientAuthentication::Device {
        device_id: credential.device_id.clone(),
        token: credential.token.clone(),
    }
}

fn validate_operation_id(
    request: &ControlRequest,
    operation_id: Option<&OperationId>,
) -> Result<(), RequestError> {
    match (request.is_mutating(), operation_id.is_some()) {
        (true, false) => Err(RequestError::InvalidRequest(
            "mutating requests require an operation ID".to_owned(),
        )),
        (false, true) => Err(RequestError::InvalidRequest(
            "read-only requests cannot carry an operation ID".to_owned(),
        )),
        _ => Ok(()),
    }
}

fn frame_agent_id(frame: &ServerFrame) -> Option<AgentId> {
    match frame {
        ServerFrame::Delta(envelope) => Some(envelope.delta.agent_id.clone()),
        ServerFrame::Event(envelope) => Some(envelope.agent_id.clone()),
        ServerFrame::InteractionRequest(envelope) => Some(envelope.agent_id.clone()),
        ServerFrame::ReplayGap(gap) => Some(gap.agent_id.clone()),
        ServerFrame::Snapshot(StateSnapshot { agents }) if agents.len() == 1 => {
            agents.first().map(|agent| agent.agent_id.clone())
        }
        _ => None,
    }
}

async fn send_frame<W>(
    socket: &mut W,
    codec: CborCodec,
    frame: &ClientFrame,
) -> Result<(), TransportError>
where
    W: BinaryWebSocket,
{
    let bytes = codec
        .encode(frame)
        .map_err(|error| TransportError::new(error.to_string()))?;
    socket.send_binary(bytes).await
}

fn next_delay(current: Duration, maximum: Duration) -> Duration {
    current.saturating_mul(2).min(maximum)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConnectionStatus {
    Disconnected {
        reason: Option<String>,
    },
    Connecting,
    Connected {
        server_id: omp_control_protocol::ServerId,
        device_id: DeviceId,
        protocol_version: ProtocolVersion,
    },
    Stopped {
        reason: Option<String>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum ClientEvent {
    ConnectionChanged(ConnectionStatus),
    StateChanged(AgentId),
    AgentEvent(omp_control_protocol::EventEnvelope),
    InteractionRequest(UiInteractionEnvelope),
    ResyncRequired(AgentId),
    ProtocolError(ProtocolError),
    ReplicationError(ReplicationError),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RequestError {
    ClientStopped,
    IdentifierSpaceExhausted,
    InvalidRequest(String),
    Protocol(ProtocolError),
}

impl fmt::Display for RequestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ClientStopped => formatter.write_str("control client has stopped"),
            Self::IdentifierSpaceExhausted => {
                formatter.write_str("control client identifier space is exhausted")
            }
            Self::InvalidRequest(message) => formatter.write_str(message),
            Self::Protocol(error) => error.message.fmt(formatter),
        }
    }
}

impl std::error::Error for RequestError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClientRunError {
    CredentialUnavailable,
    CredentialServerMismatch,
    CredentialStorage(String),
    ServerIdentityMismatch,
    Protocol(String),
    Transport(String),
}

impl fmt::Display for ClientRunError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CredentialUnavailable => {
                formatter.write_str("no device credential is stored for this server")
            }
            Self::CredentialServerMismatch => {
                formatter.write_str("device credential does not match the connected server")
            }
            Self::CredentialStorage(error) => {
                write!(formatter, "device credential storage failed: {error}")
            }
            Self::ServerIdentityMismatch => formatter
                .write_str("connected server identity does not match the configured server"),
            Self::Protocol(error) => write!(formatter, "control protocol failed: {error}"),
            Self::Transport(error) => write!(formatter, "WebSocket transport failed: {error}"),
        }
    }
}

impl std::error::Error for ClientRunError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClientBuildError {
    Random(String),
}

impl fmt::Display for ClientBuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Random(error) => write!(formatter, "secure client ID generation failed: {error}"),
        }
    }
}

impl std::error::Error for ClientBuildError {}

enum ClientCommand {
    Request {
        payload: PendingPayload,
        operation_id: Option<OperationId>,
        response: oneshot::Sender<Result<ControlResponse, RequestError>>,
    },
    Subscribe(AgentId),
    Unsubscribe(AgentId),
    Shutdown(oneshot::Sender<()>),
}

struct PendingRequest {
    payload: PendingPayload,
    operation_id: Option<OperationId>,
    sent_request_id: Option<RequestId>,
    response: oneshot::Sender<Result<ControlResponse, RequestError>>,
}

impl PendingRequest {
    fn frame(&self, request_id: RequestId) -> ClientFrame {
        match &self.payload {
            PendingPayload::Request(request) => ClientFrame::Request(RequestEnvelope {
                request_id,
                operation_id: self.operation_id.clone(),
                request: request.clone(),
            }),
            PendingPayload::UiResponse {
                agent_id,
                holder,
                response,
            } => ClientFrame::UiResponse(UiResponseEnvelope {
                agent_id: agent_id.clone(),
                request_id,
                operation_id: self
                    .operation_id
                    .clone()
                    .expect("UI responses always have operation IDs"),
                holder: holder.clone(),
                response: response.clone(),
            }),
        }
    }
}

enum PendingPayload {
    Request(ControlRequest),
    UiResponse {
        agent_id: AgentId,
        holder: omp_control_protocol::LeaseHolderId,
        response: ExtensionUiResponseFrame,
    },
}

struct IdGenerator {
    prefix: String,
    counter: u64,
}

impl IdGenerator {
    fn new() -> Result<Self, ClientBuildError> {
        let mut random = [0_u8; 16];
        getrandom::fill(&mut random)
            .map_err(|error| ClientBuildError::Random(error.to_string()))?;
        Ok(Self {
            prefix: URL_SAFE_NO_PAD.encode(random),
            counter: 0,
        })
    }

    fn next(&mut self, kind: &str) -> Result<String, IdGenerationError> {
        let counter = self.counter;
        self.counter = self
            .counter
            .checked_add(1)
            .ok_or(IdGenerationError::Exhausted)?;
        Ok(format!("{kind}-{}-{counter}", self.prefix))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum IdGenerationError {
    Exhausted,
}

impl fmt::Display for IdGenerationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("control client identifier space is exhausted")
    }
}

impl std::error::Error for IdGenerationError {}

enum HandshakeError {
    Transport(TransportError),
    Fatal(ClientRunError),
}

enum SessionEnd {
    Shutdown,
    Reconnect {
        reason: Option<String>,
        after: Option<Duration>,
    },
    Fatal(ClientRunError),
}

enum CommandAction {
    Continue,
    Shutdown,
}
