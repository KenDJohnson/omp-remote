use std::{collections::BTreeMap, fmt, sync::Arc};

use omp_control_plane::{AgentHandle, AgentRegistry};
use omp_control_protocol::{
    AgentId, AgentLifecycle, AgentSnapshot, ControlRequest, ControlResponse, EventSequence,
    InteractionState, LeaseHolderId, ProtocolError, ResponseOutcome, RunId, RunLifecycle,
    RunSnapshot, SessionSummary, StateRevision,
};
use omp_rpc::{ClientMessage, CommandKind, Response, SessionEvent, SuccessResponse};
use omp_runtime::{OmpRuntime, RuntimeConfig, RuntimeEvent, RuntimeStatus};
use parking_lot::Mutex;
use tokio::sync::Mutex as AsyncMutex;
use uuid::Uuid;

use crate::persistence::{AgentRecord, Store};

#[derive(Clone, Debug)]
pub struct DaemonController {
    registry: AgentRegistry,
    runtimes: Arc<AsyncMutex<BTreeMap<AgentId, OmpRuntime>>>,
    runtime_config: RuntimeConfig,
    store: Arc<Mutex<Store>>,
}

impl DaemonController {
    pub fn new(
        registry: AgentRegistry,
        runtime_config: RuntimeConfig,
        store: Arc<Mutex<Store>>,
    ) -> Result<Self, ControllerError> {
        let recovered = {
            let store = store.lock();
            let records = store
                .agents()
                .map_err(|error| ControllerError::Persistence(error.to_string()))?;
            let mut recovered = Vec::with_capacity(records.len());
            for record in records {
                let session = store
                    .session(&record.agent_id)
                    .map_err(|error| ControllerError::Persistence(error.to_string()))?;
                let (session, revision, event_sequence) = session.map_or_else(
                    || (None, StateRevision::default(), EventSequence::default()),
                    |session| {
                        (
                            Some(SessionSummary {
                                session_id: session.session_id,
                                session_file: Some(session.session_file),
                                name: None,
                                message_count: 0,
                            }),
                            session.revision,
                            session.event_sequence,
                        )
                    },
                );
                recovered.push(AgentSnapshot {
                    agent_id: record.agent_id,
                    revision,
                    event_sequence,
                    lifecycle: record.lifecycle,
                    session,
                    active_run: None,
                    recent_runs: Vec::new(),
                    interaction: InteractionState::Unclaimed,
                    available_commands: Vec::new(),
                });
            }
            recovered
        };
        for snapshot in recovered {
            registry.restore(snapshot).map_err(registry_error)?;
        }
        Ok(Self {
            registry,
            runtimes: Arc::new(AsyncMutex::new(BTreeMap::new())),
            runtime_config,
            store,
        })
    }

    #[must_use]
    pub fn registry(&self) -> &AgentRegistry {
        &self.registry
    }

    pub async fn execute(&self, request: ControlRequest) -> ResponseOutcome {
        match self.execute_inner(request).await {
            Ok(response) => ResponseOutcome::Success(Box::new(response)),
            Err(error) => ResponseOutcome::Error(error.into_protocol_error()),
        }
    }

    pub async fn shutdown(&self) -> Result<(), ControllerError> {
        let runtimes = {
            let mut runtimes = self.runtimes.lock().await;
            std::mem::take(&mut *runtimes)
        };
        let mut first_error = None;
        for (agent_id, runtime) in runtimes {
            let Some(agent) = self.registry.get(&agent_id) else {
                continue;
            };
            let _ = agent.set_lifecycle(AgentLifecycle::Stopping).await;
            if let Err(error) = runtime.shutdown().await {
                let message = error.to_string();
                let _ = agent
                    .set_lifecycle(AgentLifecycle::Failed {
                        reason: message.clone(),
                    })
                    .await;
                if first_error.is_none() {
                    first_error = Some(ControllerError::Runtime(message));
                }
            } else {
                let _ = agent.set_lifecycle(AgentLifecycle::Stopped).await;
            }
            if let Err(error) = self.persist_agent(&agent, None, None).await
                && first_error.is_none()
            {
                first_error = Some(error);
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    pub async fn send_ui_response(
        &self,
        agent_id: &AgentId,
        holder: &LeaseHolderId,
        response: omp_rpc::ExtensionUiResponseFrame,
    ) -> Result<(), ControllerError> {
        let snapshot = self
            .agent(agent_id)?
            .snapshot()
            .await
            .map_err(actor_error)?;
        if !matches!(
            snapshot.interaction,
            InteractionState::Leased { lease }
                if lease.holder == *holder && lease.expires_at_ms > unix_time_ms()
        ) {
            return Err(ControllerError::InteractionLeaseRequired);
        }
        let runtime = self.runtime(agent_id).await?;
        runtime
            .send(ClientMessage::ExtensionUi(response))
            .await
            .map_err(|error| ControllerError::Runtime(error.to_string()))
    }

    async fn execute_inner(
        &self,
        request: ControlRequest,
    ) -> Result<ControlResponse, ControllerError> {
        match request {
            ControlRequest::ListAgents => {
                let mut agents = Vec::new();
                for agent_id in self.registry.list() {
                    if let Some(agent) = self.registry.get(&agent_id) {
                        agents.push(agent.snapshot().await.map_err(actor_error)?);
                    }
                }
                Ok(ControlResponse::Agents { agents })
            }
            ControlRequest::GetAgent { agent_id } => {
                let agent = self.agent(&agent_id)?;
                Ok(ControlResponse::Agent {
                    agent: Box::new(agent.snapshot().await.map_err(actor_error)?),
                })
            }
            ControlRequest::LaunchAgent { agent_id } => {
                self.launch(agent_id, self.runtime_config.clone()).await?;
                Ok(ControlResponse::Accepted)
            }
            ControlRequest::StopAgent { agent_id } => {
                self.stop(&agent_id).await?;
                Ok(ControlResponse::Accepted)
            }
            ControlRequest::Prompt {
                agent_id,
                message,
                images,
                streaming_behavior,
            } => {
                let runtime = self.runtime(&agent_id).await?;
                let agent = self.agent(&agent_id)?;
                let run_id =
                    RunId::new(Uuid::new_v4().to_string()).expect("UUID run IDs are non-empty");
                let now_ms = unix_time_ms();
                agent
                    .upsert_run(RunSnapshot {
                        run_id: run_id.clone(),
                        lifecycle: RunLifecycle::Running,
                        started_at_ms: now_ms,
                        ended_at_ms: None,
                    })
                    .await
                    .map_err(actor_error)?;
                agent
                    .set_lifecycle(AgentLifecycle::Running)
                    .await
                    .map_err(actor_error)?;
                self.persist_agent(&agent, runtime_process_id(&runtime), Some(run_id.clone()))
                    .await?;
                let images = (!images.is_empty()).then_some(images);
                match runtime.prompt(message, images, streaming_behavior).await {
                    Ok(response) => ensure_rpc_success(response)?,
                    Err(error) => {
                        self.fail_active_run(&agent, error.to_string()).await;
                        return Err(ControllerError::Runtime(error.to_string()));
                    }
                }
                Ok(ControlResponse::PromptAccepted { run_id })
            }
            ControlRequest::Steer {
                agent_id,
                message,
                images,
            } => {
                let response = self
                    .runtime(&agent_id)
                    .await?
                    .request(CommandKind::Steer {
                        message,
                        images: (!images.is_empty()).then_some(images),
                    })
                    .await
                    .map_err(|error| ControllerError::Runtime(error.to_string()))?;
                ensure_rpc_success(response)?;
                Ok(ControlResponse::Accepted)
            }
            ControlRequest::FollowUp {
                agent_id,
                message,
                images,
            } => {
                let response = self
                    .runtime(&agent_id)
                    .await?
                    .request(CommandKind::FollowUp {
                        message,
                        images: (!images.is_empty()).then_some(images),
                    })
                    .await
                    .map_err(|error| ControllerError::Runtime(error.to_string()))?;
                ensure_rpc_success(response)?;
                Ok(ControlResponse::Accepted)
            }
            ControlRequest::Abort { agent_id } => {
                let runtime = self.runtime(&agent_id).await?;
                let response = runtime
                    .request(CommandKind::Abort)
                    .await
                    .map_err(|error| ControllerError::Runtime(error.to_string()))?;
                ensure_rpc_success(response)?;
                if let Ok(snapshot) = self.agent(&agent_id)?.snapshot().await
                    && let Some(mut run) = snapshot.active_run
                {
                    run.lifecycle = RunLifecycle::Aborted;
                    run.ended_at_ms = Some(unix_time_ms());
                    self.agent(&agent_id)?
                        .upsert_run(run)
                        .await
                        .map_err(actor_error)?;
                }
                Ok(ControlResponse::Accepted)
            }
            ControlRequest::SwitchSession {
                agent_id,
                session_path,
            } => {
                self.stop(&agent_id).await?;
                let config = self
                    .runtime_config
                    .clone()
                    .arg("--session")
                    .arg(session_path);
                self.launch(agent_id, config).await?;
                Ok(ControlResponse::Accepted)
            }
            ControlRequest::RespondToUi {
                agent_id,
                holder,
                response,
            } => {
                self.send_ui_response(&agent_id, &holder, response).await?;
                Ok(ControlResponse::Accepted)
            }
            ControlRequest::AcquireInteractionLease {
                agent_id,
                holder,
                ttl_ms,
            } => {
                let lease = self
                    .agent(&agent_id)?
                    .acquire_interaction_lease(holder, unix_time_ms(), ttl_ms)
                    .await
                    .map_err(actor_error)?;
                Ok(ControlResponse::InteractionLease { lease })
            }
            ControlRequest::ReleaseInteractionLease { agent_id, holder } => {
                self.agent(&agent_id)?
                    .release_interaction_lease(holder)
                    .await
                    .map_err(actor_error)?;
                Ok(ControlResponse::InteractionReleased)
            }
        }
    }

    async fn launch(
        &self,
        agent_id: AgentId,
        config: RuntimeConfig,
    ) -> Result<(), ControllerError> {
        let mut runtimes = self.runtimes.lock().await;
        if runtimes.get(&agent_id).is_some_and(|runtime| {
            matches!(
                runtime.status().borrow().clone(),
                RuntimeStatus::Running { .. }
            )
        }) {
            return Err(ControllerError::AlreadyRunning(agent_id));
        }
        runtimes.remove(&agent_id);
        let runtime = OmpRuntime::spawn(config)
            .await
            .map_err(|error| ControllerError::Runtime(error.to_string()))?;
        let agent = self.registry.get(&agent_id).map_or_else(
            || {
                self.registry
                    .create(agent_id.clone())
                    .map_err(registry_error)
            },
            Ok,
        )?;
        agent
            .set_lifecycle(AgentLifecycle::Idle)
            .await
            .map_err(actor_error)?;
        if let Ok(Response::Success { result, .. }) = runtime.request(CommandKind::GetState).await
            && let SuccessResponse::GetState { data } = *result
        {
            agent
                .set_session(SessionSummary {
                    session_id: data.session_id,
                    session_file: data.session_file,
                    name: data.session_name,
                    message_count: data.message_count,
                })
                .await
                .map_err(actor_error)?;
        }
        let process_id = runtime_process_id(&runtime);
        self.persist_agent(&agent, process_id, None).await?;
        let events = runtime.events();
        runtimes.insert(agent_id, runtime);
        drop(runtimes);
        self.spawn_event_bridge(agent, events, process_id);
        Ok(())
    }

    async fn stop(&self, agent_id: &AgentId) -> Result<(), ControllerError> {
        let runtime = self
            .runtimes
            .lock()
            .await
            .remove(agent_id)
            .ok_or_else(|| ControllerError::NotRunning(agent_id.clone()))?;
        let agent = self.agent(agent_id)?;
        agent
            .set_lifecycle(AgentLifecycle::Stopping)
            .await
            .map_err(actor_error)?;
        runtime
            .shutdown()
            .await
            .map_err(|error| ControllerError::Runtime(error.to_string()))?;
        agent
            .set_lifecycle(AgentLifecycle::Stopped)
            .await
            .map_err(actor_error)?;
        self.persist_agent(&agent, None, None).await?;
        self.registry
            .remove(agent_id)
            .await
            .map_err(registry_error)?;
        Ok(())
    }

    fn spawn_event_bridge(
        &self,
        agent: AgentHandle,
        mut events: tokio::sync::broadcast::Receiver<RuntimeEvent>,
        process_id: Option<u32>,
    ) {
        let store = Arc::clone(&self.store);
        tokio::spawn(async move {
            loop {
                match events.recv().await {
                    Ok(RuntimeEvent::Frame(frame)) => {
                        match &frame {
                            omp_rpc::ServerMessage::SessionEvent(SessionEvent::AgentStart) => {
                                let _ = agent.set_lifecycle(AgentLifecycle::Running).await;
                            }
                            omp_rpc::ServerMessage::SessionEvent(SessionEvent::AgentEnd {
                                ..
                            }) => {
                                complete_active_run(&agent).await;
                                let _ = agent.set_lifecycle(AgentLifecycle::Idle).await;
                                let _ = persist_snapshot(&store, &agent, process_id, None).await;
                            }
                            _ => {}
                        }
                        let _ = agent.publish_event(frame).await;
                    }
                    Ok(RuntimeEvent::Exited(exit)) => {
                        let lifecycle = if exit.success {
                            AgentLifecycle::Stopped
                        } else {
                            AgentLifecycle::Failed {
                                reason: format!("OMP exited with status {:?}", exit.code),
                            }
                        };
                        let _ = agent.set_lifecycle(lifecycle).await;
                        let _ = persist_snapshot(&store, &agent, None, None).await;
                        break;
                    }
                    Ok(RuntimeEvent::Failed(error)) => {
                        let _ = agent
                            .set_lifecycle(AgentLifecycle::Failed {
                                reason: error.to_string(),
                            })
                            .await;
                        let _ = persist_snapshot(&store, &agent, None, None).await;
                        break;
                    }
                    Ok(RuntimeEvent::Stderr(_)) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        let _ = agent
                            .set_lifecycle(AgentLifecycle::Failed {
                                reason: "local OMP event bridge fell behind".into(),
                            })
                            .await;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });
    }

    async fn runtime(&self, agent_id: &AgentId) -> Result<OmpRuntime, ControllerError> {
        self.runtimes
            .lock()
            .await
            .get(agent_id)
            .cloned()
            .ok_or_else(|| ControllerError::NotRunning(agent_id.clone()))
    }

    fn agent(&self, agent_id: &AgentId) -> Result<AgentHandle, ControllerError> {
        self.registry
            .get(agent_id)
            .ok_or_else(|| ControllerError::NotFound(agent_id.clone()))
    }

    async fn persist_agent(
        &self,
        agent: &AgentHandle,
        process_id: Option<u32>,
        active_run_id: Option<RunId>,
    ) -> Result<(), ControllerError> {
        persist_snapshot(&self.store, agent, process_id, active_run_id).await
    }

    async fn fail_active_run(&self, agent: &AgentHandle, reason: String) {
        if let Ok(snapshot) = agent.snapshot().await
            && let Some(mut run) = snapshot.active_run
        {
            run.lifecycle = RunLifecycle::Failed { reason };
            run.ended_at_ms = Some(unix_time_ms());
            let _ = agent.upsert_run(run).await;
        }
        let _ = agent.set_lifecycle(AgentLifecycle::Idle).await;
        let _ = self.persist_agent(agent, None, None).await;
    }
}

async fn complete_active_run(agent: &AgentHandle) {
    if let Ok(snapshot) = agent.snapshot().await
        && let Some(mut run) = snapshot.active_run
    {
        run.lifecycle = RunLifecycle::Completed;
        run.ended_at_ms = Some(unix_time_ms());
        let _ = agent.upsert_run(run).await;
    }
}

async fn persist_snapshot(
    store: &Arc<Mutex<Store>>,
    agent: &AgentHandle,
    process_id: Option<u32>,
    active_run_id: Option<RunId>,
) -> Result<(), ControllerError> {
    let now_ms = unix_time_ms();
    let snapshot = agent.snapshot().await.map_err(actor_error)?;
    let store = store.lock();
    let created_at_ms = store
        .agent(agent.agent_id())
        .map_err(|error| ControllerError::Persistence(error.to_string()))?
        .map_or(now_ms, |record| record.created_at_ms);
    store
        .upsert_agent(&AgentRecord {
            agent_id: agent.agent_id().clone(),
            lifecycle: snapshot.lifecycle,
            process_id,
            active_run_id,
            created_at_ms,
            updated_at_ms: now_ms,
        })
        .map_err(|error| ControllerError::Persistence(error.to_string()))
}

fn runtime_process_id(runtime: &OmpRuntime) -> Option<u32> {
    match runtime.status().borrow().clone() {
        RuntimeStatus::Running { process_id } => Some(process_id),
        RuntimeStatus::Exited(_) | RuntimeStatus::Failed(_) => None,
    }
}

fn ensure_rpc_success(response: Response) -> Result<(), ControllerError> {
    match response {
        Response::Success { .. } => Ok(()),
        Response::Error { error, code, .. } => Err(ControllerError::Rpc {
            code: code.unwrap_or_else(|| "omp_error".into()),
            message: error,
        }),
    }
}

fn actor_error(error: omp_control_plane::AgentError) -> ControllerError {
    ControllerError::ControlPlane(error.to_string())
}

fn registry_error(error: omp_control_plane::RegistryError) -> ControllerError {
    ControllerError::ControlPlane(error.to_string())
}

pub fn unix_time_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

#[derive(Debug, PartialEq, Eq)]
pub enum ControllerError {
    NotFound(AgentId),
    NotRunning(AgentId),
    AlreadyRunning(AgentId),
    InteractionLeaseRequired,
    Runtime(String),
    Rpc { code: String, message: String },
    ControlPlane(String),
    Persistence(String),
}

impl ControllerError {
    #[must_use]
    pub fn into_protocol_error(self) -> ProtocolError {
        let (code, message, retryable) = match self {
            Self::NotFound(agent_id) => (
                "agent_not_found".into(),
                format!("agent {agent_id} was not found"),
                false,
            ),
            Self::NotRunning(agent_id) => (
                "agent_not_running".into(),
                format!("agent {agent_id} is not running"),
                false,
            ),
            Self::AlreadyRunning(agent_id) => (
                "agent_already_running".into(),
                format!("agent {agent_id} is already running"),
                false,
            ),
            Self::InteractionLeaseRequired => (
                "interaction_lease_required".into(),
                "an active interaction lease owned by this client is required".into(),
                false,
            ),
            Self::Runtime(message) => ("runtime_error".into(), message, true),
            Self::Rpc { code, message } => (code, message, false),
            Self::ControlPlane(message) => ("control_plane_error".into(), message, true),
            Self::Persistence(message) => ("persistence_error".into(), message, true),
        };
        ProtocolError {
            code,
            message,
            retryable,
        }
    }
}

impl fmt::Display for ControllerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound(agent_id) => write!(formatter, "agent {agent_id} was not found"),
            Self::NotRunning(agent_id) => write!(formatter, "agent {agent_id} is not running"),
            Self::AlreadyRunning(agent_id) => {
                write!(formatter, "agent {agent_id} is already running")
            }
            Self::InteractionLeaseRequired => {
                formatter.write_str("an active interaction lease owned by this client is required")
            }
            Self::Runtime(message)
            | Self::Rpc { message, .. }
            | Self::ControlPlane(message)
            | Self::Persistence(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for ControllerError {}
