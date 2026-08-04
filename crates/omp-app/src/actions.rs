use std::{fmt, num::NonZeroU64};

use omp_control_client::{ClientHandle, RequestError};
use omp_control_protocol::{
    AgentId, AgentSnapshot, ControlRequest, ControlResponse, DeviceId, DeviceSummary,
    InteractionLease, LeaseHolderId, RunId,
};
use omp_rpc::{ExtensionUiResponseFrame, StreamingBehavior};

#[derive(Clone)]
pub struct AppActions {
    handle: ClientHandle,
    holder: LeaseHolderId,
}

impl AppActions {
    pub fn new(handle: ClientHandle, holder: LeaseHolderId) -> Self {
        Self { handle, holder }
    }

    #[must_use]
    pub fn handle(&self) -> &ClientHandle {
        &self.handle
    }

    #[must_use]
    pub fn holder(&self) -> &LeaseHolderId {
        &self.holder
    }

    pub fn subscribe(&self, agent_id: AgentId) -> Result<(), ActionError> {
        self.handle.subscribe(agent_id).map_err(ActionError::Client)
    }

    pub fn unsubscribe(&self, agent_id: AgentId) -> Result<(), ActionError> {
        self.handle
            .unsubscribe(agent_id)
            .map_err(ActionError::Client)
    }

    pub async fn list_agents(&self) -> Result<Vec<AgentSnapshot>, ActionError> {
        match self.handle.request(ControlRequest::ListAgents).await? {
            ControlResponse::Agents { agents } => Ok(agents),
            response => Err(ActionError::UnexpectedResponse(response)),
        }
    }

    pub async fn launch(&self, agent_id: AgentId) -> Result<(), ActionError> {
        expect_accepted(
            self.handle
                .request(ControlRequest::LaunchAgent {
                    agent_id: agent_id.clone(),
                })
                .await?,
        )?;
        self.subscribe(agent_id)
    }

    pub async fn stop(&self, agent_id: AgentId) -> Result<(), ActionError> {
        expect_accepted(
            self.handle
                .request(ControlRequest::StopAgent { agent_id })
                .await?,
        )
    }

    pub async fn prompt(&self, agent_id: AgentId, message: String) -> Result<RunId, ActionError> {
        match self
            .handle
            .request(ControlRequest::Prompt {
                agent_id,
                message,
                images: Vec::new(),
                streaming_behavior: Some(StreamingBehavior::FollowUp),
            })
            .await?
        {
            ControlResponse::PromptAccepted { run_id } => Ok(run_id),
            response => Err(ActionError::UnexpectedResponse(response)),
        }
    }

    pub async fn steer(&self, agent_id: AgentId, message: String) -> Result<(), ActionError> {
        expect_accepted(
            self.handle
                .request(ControlRequest::Steer {
                    agent_id,
                    message,
                    images: Vec::new(),
                })
                .await?,
        )
    }

    pub async fn follow_up(&self, agent_id: AgentId, message: String) -> Result<(), ActionError> {
        expect_accepted(
            self.handle
                .request(ControlRequest::FollowUp {
                    agent_id,
                    message,
                    images: Vec::new(),
                })
                .await?,
        )
    }

    pub async fn abort(&self, agent_id: AgentId) -> Result<(), ActionError> {
        expect_accepted(
            self.handle
                .request(ControlRequest::Abort { agent_id })
                .await?,
        )
    }

    pub async fn switch_session(
        &self,
        agent_id: AgentId,
        session_path: String,
    ) -> Result<(), ActionError> {
        expect_accepted(
            self.handle
                .request(ControlRequest::SwitchSession {
                    agent_id,
                    session_path,
                })
                .await?,
        )
    }

    pub async fn acquire_interaction_lease(
        &self,
        agent_id: AgentId,
    ) -> Result<InteractionLease, ActionError> {
        match self
            .handle
            .request(ControlRequest::AcquireInteractionLease {
                agent_id,
                holder: self.holder.clone(),
                ttl_ms: NonZeroU64::new(120_000).expect("the UI lease duration is non-zero"),
            })
            .await?
        {
            ControlResponse::InteractionLease { lease } => Ok(lease),
            response => Err(ActionError::UnexpectedResponse(response)),
        }
    }

    pub async fn respond_to_interaction(
        &self,
        agent_id: AgentId,
        response: ExtensionUiResponseFrame,
    ) -> Result<(), ActionError> {
        expect_accepted(
            self.handle
                .respond_to_ui(agent_id.clone(), self.holder.clone(), response)
                .await?,
        )?;
        match self
            .handle
            .request(ControlRequest::ReleaseInteractionLease {
                agent_id,
                holder: self.holder.clone(),
            })
            .await?
        {
            ControlResponse::InteractionReleased => Ok(()),
            response => Err(ActionError::UnexpectedResponse(response)),
        }
    }

    pub async fn list_devices(&self) -> Result<Vec<DeviceSummary>, ActionError> {
        match self.handle.request(ControlRequest::ListDevices).await? {
            ControlResponse::Devices { devices } => Ok(devices),
            response => Err(ActionError::UnexpectedResponse(response)),
        }
    }

    pub async fn revoke_device(&self, device_id: DeviceId) -> Result<DeviceId, ActionError> {
        match self
            .handle
            .request(ControlRequest::RevokeDevice { device_id })
            .await?
        {
            ControlResponse::DeviceRevoked { device_id } => Ok(device_id),
            response => Err(ActionError::UnexpectedResponse(response)),
        }
    }

    pub async fn shutdown(&self) -> Result<(), ActionError> {
        self.handle.shutdown().await.map_err(ActionError::Client)
    }
}

fn expect_accepted(response: ControlResponse) -> Result<(), ActionError> {
    match response {
        ControlResponse::Accepted => Ok(()),
        response => Err(ActionError::UnexpectedResponse(response)),
    }
}

#[derive(Debug)]
pub enum ActionError {
    Client(RequestError),
    UnexpectedResponse(ControlResponse),
}

impl fmt::Display for ActionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Client(error) => error.fmt(formatter),
            Self::UnexpectedResponse(response) => {
                write!(formatter, "unexpected control response: {response:?}")
            }
        }
    }
}

impl std::error::Error for ActionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Client(error) => Some(error),
            Self::UnexpectedResponse(_) => None,
        }
    }
}

impl From<RequestError> for ActionError {
    fn from(error: RequestError) -> Self {
        Self::Client(error)
    }
}
