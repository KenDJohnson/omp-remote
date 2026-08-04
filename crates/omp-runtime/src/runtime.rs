use std::{
    collections::HashMap,
    process::{ExitStatus, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use omp_rpc::{
    ClientMessage, Command, CommandKind, ImageContent, PromptAcknowledgement, RequestId, Response,
    RpcFrameDecoder, ServerMessage, SideChannelFrame, StreamingBehavior, SuccessResponse,
    TransportFrame,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout, Command as ProcessCommand},
    sync::{broadcast, mpsc, oneshot, watch},
    time,
};

use crate::{
    PromptCompletion, PromptPhase, PromptStatus, RequestError, RuntimeConfig, RuntimeEvent,
    RuntimeExit, RuntimeSpawnError, RuntimeStatus,
};

const ACTOR_CHANNEL_CAPACITY: usize = 64;
const STDERR_CHUNK_BYTES: usize = 8 * 1024;
const NEGOTIATION_REQUEST_ID: &str = "omp-runtime-negotiate-v2";

#[derive(Clone, Debug)]
pub struct OmpRuntime {
    inner: Arc<RuntimeInner>,
}

#[derive(Debug)]
struct RuntimeInner {
    actor_tx: mpsc::Sender<ActorMessage>,
    events_tx: broadcast::Sender<RuntimeEvent>,
    first_events: Mutex<Option<broadcast::Receiver<RuntimeEvent>>>,
    status_tx: watch::Sender<RuntimeStatus>,
    prompt_tx: watch::Sender<Option<PromptStatus>>,
    next_request_id: AtomicU64,
    request_timeout: Duration,
}

impl OmpRuntime {
    pub async fn spawn(config: RuntimeConfig) -> Result<Self, RuntimeSpawnError> {
        let mut command = ProcessCommand::new(&config.program);
        command
            .args(&config.args)
            .arg("--mode")
            .arg("rpc")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if let Some(current_dir) = &config.current_dir {
            command.current_dir(current_dir);
        }

        let mut child = command.spawn().map_err(|source| RuntimeSpawnError::Spawn {
            program: config.program.clone(),
            source,
        })?;
        let process_id = child.id().ok_or(RuntimeSpawnError::StartupProtocol(
            "spawned process has no ID".into(),
        ))?;
        let mut stdin = child
            .stdin
            .take()
            .ok_or(RuntimeSpawnError::MissingPipe("stdin"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or(RuntimeSpawnError::MissingPipe("stdout"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or(RuntimeSpawnError::MissingPipe("stderr"))?;

        let (events_tx, first_events) = broadcast::channel(config.event_capacity.get());
        tokio::spawn(read_stderr(stderr, events_tx.clone()));

        let startup = time::timeout(
            config.startup_timeout,
            initialize_rpc(BufReader::new(stdout), &mut stdin),
        )
        .await
        .map_err(|_| RuntimeSpawnError::StartupTimedOut)??;

        let (actor_tx, actor_rx) = mpsc::channel(ACTOR_CHANNEL_CAPACITY);
        let (reader_tx, reader_rx) = mpsc::channel(ACTOR_CHANNEL_CAPACITY);
        let (status_tx, _) = watch::channel(RuntimeStatus::Running { process_id });
        let (prompt_tx, _) = watch::channel(None);

        let inner = Arc::new(RuntimeInner {
            actor_tx,
            events_tx: events_tx.clone(),
            first_events: Mutex::new(Some(first_events)),
            status_tx: status_tx.clone(),
            prompt_tx: prompt_tx.clone(),
            next_request_id: AtomicU64::new(1),
            request_timeout: config.request_timeout,
        });

        for event in startup.initial_events {
            let _ = events_tx.send(RuntimeEvent::Frame(event));
        }
        tokio::spawn(read_stdout(
            startup.stdout,
            startup.decoder,
            startup.max_frame_bytes,
            reader_tx,
        ));
        tokio::spawn(run_actor(
            child,
            stdin,
            actor_rx,
            reader_rx,
            events_tx,
            status_tx,
            prompt_tx,
            startup.max_frame_bytes,
            config.shutdown_timeout,
        ));

        Ok(Self { inner })
    }

    pub async fn request(&self, kind: CommandKind) -> Result<Response, RequestError> {
        let sequence = self.inner.next_request_id.fetch_add(1, Ordering::Relaxed);
        let request_id = RequestId::new(format!("omp-runtime-{sequence}"));
        let command = Command::with_id(request_id.clone(), kind);
        let (response_tx, response_rx) = oneshot::channel();
        self.inner
            .actor_tx
            .send(ActorMessage::Request {
                command,
                response_tx,
            })
            .await
            .map_err(|_| RequestError::RuntimeStopped)?;

        match time::timeout(self.inner.request_timeout, response_rx).await {
            Ok(Ok(response)) => response,
            Ok(Err(_)) => Err(RequestError::RuntimeStopped),
            Err(_) => {
                let _ = self
                    .inner
                    .actor_tx
                    .send(ActorMessage::Cancel {
                        request_id: request_id.clone(),
                    })
                    .await;
                Err(RequestError::TimedOut { request_id })
            }
        }
    }

    pub async fn prompt(
        &self,
        message: impl Into<String>,
        images: Option<Vec<ImageContent>>,
        streaming_behavior: Option<StreamingBehavior>,
    ) -> Result<Response, RequestError> {
        self.request(CommandKind::Prompt {
            message: message.into(),
            images,
            streaming_behavior,
        })
        .await
    }

    pub async fn send(&self, message: ClientMessage) -> Result<(), RequestError> {
        if matches!(message, ClientMessage::Command(_)) {
            return Err(RequestError::Transport(
                "commands must use request correlation".into(),
            ));
        }
        let (response_tx, response_rx) = oneshot::channel();
        self.inner
            .actor_tx
            .send(ActorMessage::Send {
                message,
                response_tx,
            })
            .await
            .map_err(|_| RequestError::RuntimeStopped)?;
        response_rx
            .await
            .unwrap_or(Err(RequestError::RuntimeStopped))
    }

    #[must_use]
    pub fn events(&self) -> broadcast::Receiver<RuntimeEvent> {
        self.inner
            .first_events
            .lock()
            .expect("runtime event receiver mutex was poisoned")
            .take()
            .unwrap_or_else(|| self.inner.events_tx.subscribe())
    }

    #[must_use]
    pub fn status(&self) -> watch::Receiver<RuntimeStatus> {
        self.inner.status_tx.subscribe()
    }

    #[must_use]
    pub fn prompt_status(&self) -> watch::Receiver<Option<PromptStatus>> {
        self.inner.prompt_tx.subscribe()
    }

    pub async fn wait(&self) -> RuntimeStatus {
        let mut status = self.status();
        loop {
            let current = status.borrow().clone();
            if !matches!(current, RuntimeStatus::Running { .. }) {
                return current;
            }
            if status.changed().await.is_err() {
                return status.borrow().clone();
            }
        }
    }

    pub async fn shutdown(&self) -> Result<RuntimeExit, RequestError> {
        match self.inner.status_tx.borrow().clone() {
            RuntimeStatus::Exited(exit) => return Ok(exit),
            RuntimeStatus::Failed(_) => return Err(RequestError::RuntimeStopped),
            RuntimeStatus::Running { .. } => {}
        }

        let (response_tx, response_rx) = oneshot::channel();
        if self
            .inner
            .actor_tx
            .send(ActorMessage::Shutdown { response_tx })
            .await
            .is_err()
        {
            return match self.inner.status_tx.borrow().clone() {
                RuntimeStatus::Exited(exit) => Ok(exit),
                _ => Err(RequestError::RuntimeStopped),
            };
        }
        response_rx
            .await
            .unwrap_or(Err(RequestError::RuntimeStopped))
    }
}

#[derive(Debug)]
struct Startup {
    stdout: BufReader<ChildStdout>,
    decoder: RpcFrameDecoder,
    max_frame_bytes: usize,
    initial_events: Vec<ServerMessage>,
}

async fn initialize_rpc(
    mut stdout: BufReader<ChildStdout>,
    stdin: &mut ChildStdin,
) -> Result<Startup, RuntimeSpawnError> {
    let default_limit = usize::try_from(omp_rpc::ReadyFrame::DEFAULT_MAX_FRAME_BYTES)
        .expect("the default physical frame limit fits supported targets");
    let line = read_limited_line(&mut stdout, default_limit)
        .await
        .map_err(line_startup_error)?
        .ok_or(RuntimeSpawnError::StartupEof)?;
    let mut decoder = RpcFrameDecoder::default();
    let message = decoder
        .push_json_line(line)
        .map_err(|error| RuntimeSpawnError::StartupProtocol(error.to_string().into()))?
        .ok_or_else(|| RuntimeSpawnError::StartupProtocol("ready frame was chunked".into()))?;
    let ServerMessage::Transport(TransportFrame::Ready { ready }) = message else {
        return Err(RuntimeSpawnError::ExpectedReadyFrame);
    };

    let max_frame_bytes = usize::try_from(ready.max_frame_bytes().get()).map_err(|_| {
        RuntimeSpawnError::StartupProtocol("physical frame limit is unsupported".into())
    })?;
    let mut initial_events = Vec::new();
    if ready.advertises_capabilities() {
        let request_id = RequestId::new(NEGOTIATION_REQUEST_ID);
        let command = ClientMessage::Command(Command::negotiate_protocol(request_id.clone()));
        write_message(stdin, &command, max_frame_bytes)
            .await
            .map_err(request_startup_error)?;

        loop {
            let line = read_limited_line(&mut stdout, max_frame_bytes)
                .await
                .map_err(line_startup_error)?
                .ok_or(RuntimeSpawnError::StartupEof)?;
            let Some(message) = decoder
                .push_json_line(line)
                .map_err(|error| RuntimeSpawnError::StartupProtocol(error.to_string().into()))?
            else {
                continue;
            };
            match message {
                ServerMessage::Response(Response::Success {
                    id: Some(id),
                    result,
                }) if id == request_id => {
                    if matches!(*result, SuccessResponse::NegotiateProtocol { .. }) {
                        break;
                    }
                    return Err(RuntimeSpawnError::ProtocolNegotiation(
                        "server returned the wrong success response".into(),
                    ));
                }
                ServerMessage::Response(Response::Error {
                    id: Some(id),
                    error,
                    ..
                }) if id == request_id => {
                    return Err(RuntimeSpawnError::ProtocolNegotiation(error.into()));
                }
                message => initial_events.push(message),
            }
        }
    }

    Ok(Startup {
        stdout,
        decoder,
        max_frame_bytes,
        initial_events,
    })
}

fn line_startup_error(error: LineReadError) -> RuntimeSpawnError {
    match error {
        LineReadError::Io(error) => RuntimeSpawnError::StartupIo(error),
        other => RuntimeSpawnError::StartupProtocol(other.to_string().into()),
    }
}

fn request_startup_error(error: RequestError) -> RuntimeSpawnError {
    RuntimeSpawnError::ProtocolNegotiation(error.to_string().into())
}

#[derive(Debug)]
enum ActorMessage {
    Request {
        command: Command,
        response_tx: oneshot::Sender<Result<Response, RequestError>>,
    },
    Cancel {
        request_id: RequestId,
    },
    Send {
        message: ClientMessage,
        response_tx: oneshot::Sender<Result<(), RequestError>>,
    },
    Shutdown {
        response_tx: oneshot::Sender<Result<RuntimeExit, RequestError>>,
    },
}

#[derive(Debug)]
enum ReaderItem {
    Frame(ServerMessage),
    Error(Arc<str>),
    Eof,
}

#[derive(Debug)]
enum StopReason {
    Shutdown(Option<oneshot::Sender<Result<RuntimeExit, RequestError>>>),
    ChildExited(Result<ExitStatus, std::io::Error>),
    TransportFailed(Arc<str>),
}

#[allow(clippy::too_many_arguments)]
async fn run_actor(
    mut child: Child,
    mut stdin: ChildStdin,
    mut actor_rx: mpsc::Receiver<ActorMessage>,
    mut reader_rx: mpsc::Receiver<ReaderItem>,
    events_tx: broadcast::Sender<RuntimeEvent>,
    status_tx: watch::Sender<RuntimeStatus>,
    prompt_tx: watch::Sender<Option<PromptStatus>>,
    max_frame_bytes: usize,
    shutdown_timeout: Duration,
) {
    let mut pending = HashMap::new();
    let mut reader_open = true;
    let reason = loop {
        tokio::select! {
            status = child.wait() => break StopReason::ChildExited(status),
            actor_message = actor_rx.recv() => {
                let Some(actor_message) = actor_message else {
                    break StopReason::Shutdown(None);
                };
                match actor_message {
                    ActorMessage::Request { command, response_tx } => {
                        let request_id = command.id.clone().expect("runtime commands always have IDs");
                        if is_prompt_command(&command.kind) {
                            prompt_tx.send_replace(Some(PromptStatus {
                                request_id: request_id.clone(),
                                phase: PromptPhase::Submitted,
                            }));
                        }
                        let message = ClientMessage::Command(command);
                        match write_message(&mut stdin, &message, max_frame_bytes).await {
                            Ok(()) => {
                                pending.insert(request_id, response_tx);
                            }
                            Err(error @ RequestError::FrameTooLarge { .. }) => {
                                fail_prompt(&prompt_tx, &request_id, error.to_string());
                                let _ = response_tx.send(Err(error));
                            }
                            Err(error) => {
                                let message: Arc<str> = error.to_string().into();
                                let _ = response_tx.send(Err(error));
                                break StopReason::TransportFailed(message);
                            }
                        }
                    }
                    ActorMessage::Cancel { request_id } => {
                        pending.remove(&request_id);
                    }
                    ActorMessage::Send { message, response_tx } => {
                        match write_message(&mut stdin, &message, max_frame_bytes).await {
                            Ok(()) => {
                                let _ = response_tx.send(Ok(()));
                            }
                            Err(error @ RequestError::FrameTooLarge { .. }) => {
                                let _ = response_tx.send(Err(error));
                            }
                            Err(error) => {
                                let message: Arc<str> = error.to_string().into();
                                let _ = response_tx.send(Err(error));
                                break StopReason::TransportFailed(message);
                            }
                        }
                    }
                    ActorMessage::Shutdown { response_tx } => {
                        break StopReason::Shutdown(Some(response_tx));
                    }
                }
            }
            reader_item = reader_rx.recv(), if reader_open => {
                match reader_item {
                    Some(ReaderItem::Frame(frame)) => {
                        handle_frame(frame, &mut pending, &events_tx, &prompt_tx);
                    }
                    Some(ReaderItem::Error(error)) => break StopReason::TransportFailed(error),
                    Some(ReaderItem::Eof) | None => reader_open = false,
                }
            }
        }
    };

    match reason {
        StopReason::Shutdown(response_tx) => {
            drop(stdin);
            let exit = shutdown_child(
                &mut child,
                &mut reader_rx,
                &mut pending,
                &events_tx,
                &prompt_tx,
                shutdown_timeout,
                reader_open,
            )
            .await;
            match exit {
                Ok(exit) => {
                    publish_exit(&status_tx, &events_tx, exit.clone());
                    if let Some(response_tx) = response_tx {
                        let _ = response_tx.send(Ok(exit));
                    }
                }
                Err(message) => {
                    publish_failure(&status_tx, &events_tx, message.clone());
                    if let Some(response_tx) = response_tx {
                        let _ = response_tx.send(Err(RequestError::Transport(message)));
                    }
                }
            }
        }
        StopReason::ChildExited(status) => match status {
            Ok(status) => {
                drain_reader(&mut reader_rx, &mut pending, &events_tx, &prompt_tx).await;
                publish_exit(&status_tx, &events_tx, exit_from_status(status, false));
            }
            Err(error) => {
                publish_failure(&status_tx, &events_tx, error.to_string().into());
            }
        },
        StopReason::TransportFailed(message) => {
            let _ = child.start_kill();
            let _ = child.wait().await;
            publish_failure(&status_tx, &events_tx, message);
        }
    }
    fail_pending(&mut pending, RequestError::RuntimeStopped);
}

async fn shutdown_child(
    child: &mut Child,
    reader_rx: &mut mpsc::Receiver<ReaderItem>,
    pending: &mut HashMap<RequestId, oneshot::Sender<Result<Response, RequestError>>>,
    events_tx: &broadcast::Sender<RuntimeEvent>,
    prompt_tx: &watch::Sender<Option<PromptStatus>>,
    timeout: Duration,
    reader_open: bool,
) -> Result<RuntimeExit, Arc<str>> {
    let graceful = time::timeout(timeout, async {
        let mut reader_open = reader_open;
        loop {
            if !reader_open {
                return child
                    .wait()
                    .await
                    .map_err(|error| Arc::<str>::from(error.to_string()));
            }
            tokio::select! {
                status = child.wait() => {
                    return status.map_err(|error| Arc::<str>::from(error.to_string()));
                }
                reader_item = reader_rx.recv() => {
                    match reader_item {
                        Some(ReaderItem::Frame(frame)) => {
                            handle_frame(frame, pending, events_tx, prompt_tx);
                        }
                        Some(ReaderItem::Error(error)) => return Err(error),
                        Some(ReaderItem::Eof) | None => reader_open = false,
                    }
                }
            }
        }
    })
    .await;

    match graceful {
        Ok(Ok(status)) => {
            drain_reader(reader_rx, pending, events_tx, prompt_tx).await;
            Ok(exit_from_status(status, false))
        }
        Ok(Err(error)) => Err(error),
        Err(_) => {
            child
                .start_kill()
                .map_err(|error| Arc::<str>::from(error.to_string()))?;
            let status = child
                .wait()
                .await
                .map_err(|error| Arc::<str>::from(error.to_string()))?;
            drain_reader(reader_rx, pending, events_tx, prompt_tx).await;
            Ok(exit_from_status(status, true))
        }
    }
}

fn handle_reader_item(
    item: Option<ReaderItem>,
    pending: &mut HashMap<RequestId, oneshot::Sender<Result<Response, RequestError>>>,
    events_tx: &broadcast::Sender<RuntimeEvent>,
    prompt_tx: &watch::Sender<Option<PromptStatus>>,
) -> bool {
    match item {
        Some(ReaderItem::Frame(frame)) => {
            handle_frame(frame, pending, events_tx, prompt_tx);
            false
        }
        Some(ReaderItem::Error(error)) => {
            let _ = events_tx.send(RuntimeEvent::Failed(error));
            true
        }
        Some(ReaderItem::Eof) | None => true,
    }
}

async fn drain_reader(
    reader_rx: &mut mpsc::Receiver<ReaderItem>,
    pending: &mut HashMap<RequestId, oneshot::Sender<Result<Response, RequestError>>>,
    events_tx: &broadcast::Sender<RuntimeEvent>,
    prompt_tx: &watch::Sender<Option<PromptStatus>>,
) {
    while let Some(item) = reader_rx.recv().await {
        if handle_reader_item(Some(item), pending, events_tx, prompt_tx) {
            break;
        }
    }
}

fn handle_frame(
    frame: ServerMessage,
    pending: &mut HashMap<RequestId, oneshot::Sender<Result<Response, RequestError>>>,
    events_tx: &broadcast::Sender<RuntimeEvent>,
    prompt_tx: &watch::Sender<Option<PromptStatus>>,
) {
    update_prompt_status(&frame, prompt_tx);
    if let ServerMessage::Response(response) = frame {
        if let Some(request_id) = response.id()
            && let Some(response_tx) = pending.remove(request_id)
        {
            let _ = response_tx.send(Ok(response));
            return;
        }
        let _ = events_tx.send(RuntimeEvent::Frame(ServerMessage::Response(response)));
        return;
    }
    let _ = events_tx.send(RuntimeEvent::Frame(frame));
}

fn update_prompt_status(frame: &ServerMessage, prompt_tx: &watch::Sender<Option<PromptStatus>>) {
    let Some(current) = prompt_tx.borrow().clone() else {
        return;
    };
    let next_phase = match frame {
        ServerMessage::Response(Response::Error {
            id: Some(id),
            error,
            ..
        }) if *id == current.request_id => Some(PromptPhase::Failed(error.clone().into())),
        ServerMessage::Response(Response::Success {
            id: Some(id),
            result,
            ..
        }) if *id == current.request_id => match result.as_ref() {
            SuccessResponse::Prompt {
                data:
                    Some(PromptAcknowledgement {
                        agent_invoked: false,
                    }),
            } => Some(PromptPhase::Completed(PromptCompletion::Local)),
            SuccessResponse::Prompt {
                data:
                    Some(PromptAcknowledgement {
                        agent_invoked: true,
                    }),
            } if matches!(current.phase, PromptPhase::Submitted) => Some(PromptPhase::Running),
            _ => None,
        },
        ServerMessage::SideChannel(SideChannelFrame::PromptResult {
            id: Some(id),
            agent_invoked,
        }) if *id == current.request_id => Some(if *agent_invoked {
            PromptPhase::Running
        } else {
            PromptPhase::Completed(PromptCompletion::Local)
        }),
        ServerMessage::SessionEvent(omp_rpc::SessionEvent::AgentStart)
            if matches!(current.phase, PromptPhase::Submitted) =>
        {
            Some(PromptPhase::Running)
        }
        ServerMessage::SessionEvent(omp_rpc::SessionEvent::AgentEnd { .. })
            if matches!(current.phase, PromptPhase::Submitted | PromptPhase::Running) =>
        {
            Some(PromptPhase::Completed(PromptCompletion::Agent))
        }
        _ => None,
    };
    if let Some(phase) = next_phase {
        prompt_tx.send_replace(Some(PromptStatus {
            request_id: current.request_id,
            phase,
        }));
    }
}

fn is_prompt_command(kind: &CommandKind) -> bool {
    matches!(
        kind,
        CommandKind::Prompt { .. } | CommandKind::AbortAndPrompt { .. }
    )
}

fn fail_prompt(
    prompt_tx: &watch::Sender<Option<PromptStatus>>,
    request_id: &RequestId,
    error: impl Into<Arc<str>>,
) {
    let Some(current) = prompt_tx.borrow().clone() else {
        return;
    };
    if current.request_id == *request_id {
        prompt_tx.send_replace(Some(PromptStatus {
            request_id: current.request_id,
            phase: PromptPhase::Failed(error.into()),
        }));
    }
}

fn fail_pending(
    pending: &mut HashMap<RequestId, oneshot::Sender<Result<Response, RequestError>>>,
    error: RequestError,
) {
    for (_, response_tx) in pending.drain() {
        let _ = response_tx.send(Err(error.clone()));
    }
}

fn publish_exit(
    status_tx: &watch::Sender<RuntimeStatus>,
    events_tx: &broadcast::Sender<RuntimeEvent>,
    exit: RuntimeExit,
) {
    status_tx.send_replace(RuntimeStatus::Exited(exit.clone()));
    let _ = events_tx.send(RuntimeEvent::Exited(exit));
}

fn publish_failure(
    status_tx: &watch::Sender<RuntimeStatus>,
    events_tx: &broadcast::Sender<RuntimeEvent>,
    message: Arc<str>,
) {
    status_tx.send_replace(RuntimeStatus::Failed(message.clone()));
    let _ = events_tx.send(RuntimeEvent::Failed(message));
}

fn exit_from_status(status: ExitStatus, forced: bool) -> RuntimeExit {
    RuntimeExit {
        code: status.code(),
        success: status.success(),
        forced,
    }
}

async fn write_message(
    stdin: &mut ChildStdin,
    message: &ClientMessage,
    max_frame_bytes: usize,
) -> Result<(), RequestError> {
    let encoded = message
        .to_json_line()
        .map_err(|error| RequestError::Transport(error.to_string().into()))?;
    if encoded.len() > max_frame_bytes {
        return Err(RequestError::FrameTooLarge {
            bytes: encoded.len(),
            limit: max_frame_bytes,
        });
    }
    stdin
        .write_all(&encoded)
        .await
        .map_err(|error| RequestError::Transport(error.to_string().into()))?;
    stdin
        .flush()
        .await
        .map_err(|error| RequestError::Transport(error.to_string().into()))
}

async fn read_stdout(
    mut stdout: BufReader<ChildStdout>,
    mut decoder: RpcFrameDecoder,
    max_frame_bytes: usize,
    reader_tx: mpsc::Sender<ReaderItem>,
) {
    loop {
        match read_limited_line(&mut stdout, max_frame_bytes).await {
            Ok(Some(line)) => match decoder.push_json_line(line) {
                Ok(Some(frame)) => {
                    if reader_tx.send(ReaderItem::Frame(frame)).await.is_err() {
                        return;
                    }
                }
                Ok(None) => {}
                Err(error) => {
                    let _ = reader_tx
                        .send(ReaderItem::Error(error.to_string().into()))
                        .await;
                    return;
                }
            },
            Ok(None) => {
                let item = if decoder.is_reassembling() {
                    ReaderItem::Error("OMP stdout ended during an RPC chunk sequence".into())
                } else {
                    ReaderItem::Eof
                };
                let _ = reader_tx.send(item).await;
                return;
            }
            Err(error) => {
                let _ = reader_tx
                    .send(ReaderItem::Error(error.to_string().into()))
                    .await;
                return;
            }
        }
    }
}

async fn read_stderr<R>(mut stderr: R, events_tx: broadcast::Sender<RuntimeEvent>)
where
    R: AsyncRead + Unpin,
{
    let mut buffer = [0_u8; STDERR_CHUNK_BYTES];
    loop {
        match stderr.read(&mut buffer).await {
            Ok(0) => return,
            Ok(bytes) => {
                let text: Arc<str> = String::from_utf8_lossy(&buffer[..bytes])
                    .into_owned()
                    .into();
                let _ = events_tx.send(RuntimeEvent::Stderr(text));
            }
            Err(error) => {
                let _ = events_tx.send(RuntimeEvent::Stderr(
                    format!("failed to read OMP stderr: {error}").into(),
                ));
                return;
            }
        }
    }
}

#[derive(Debug)]
enum LineReadError {
    Io(std::io::Error),
    TooLarge { limit: usize },
    UnexpectedEof,
}

impl std::fmt::Display for LineReadError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => error.fmt(formatter),
            Self::TooLarge { limit } => write!(formatter, "RPC frame exceeds {limit} bytes"),
            Self::UnexpectedEof => formatter.write_str("OMP stdout ended in the middle of a frame"),
        }
    }
}

async fn read_limited_line<R>(
    reader: &mut BufReader<R>,
    limit: usize,
) -> Result<Option<Vec<u8>>, LineReadError>
where
    R: AsyncRead + Unpin,
{
    let mut line = Vec::new();
    loop {
        let available = reader.fill_buf().await.map_err(LineReadError::Io)?;
        if available.is_empty() {
            return if line.is_empty() {
                Ok(None)
            } else {
                Err(LineReadError::UnexpectedEof)
            };
        }
        let consumed = available
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(available.len(), |position| position + 1);
        if line.len().saturating_add(consumed) > limit {
            return Err(LineReadError::TooLarge { limit });
        }
        line.extend_from_slice(&available[..consumed]);
        let complete = available[consumed - 1] == b'\n';
        reader.consume(consumed);
        if complete {
            return Ok(Some(line));
        }
    }
}
