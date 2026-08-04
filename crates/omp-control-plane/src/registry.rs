use parking_lot::RwLock;
use std::{collections::BTreeMap, fmt, sync::Arc};

use crate::{AgentActorConfig, AgentError, AgentHandle, AgentId, AgentSnapshot};

#[derive(Clone, Debug)]
pub struct AgentRegistry {
    config: AgentActorConfig,
    agents: Arc<RwLock<BTreeMap<AgentId, AgentHandle>>>,
}

impl AgentRegistry {
    #[must_use]
    pub fn new(config: AgentActorConfig) -> Self {
        Self {
            config,
            agents: Arc::new(RwLock::new(BTreeMap::new())),
        }
    }

    pub fn create(&self, agent_id: AgentId) -> Result<AgentHandle, RegistryError> {
        let mut agents = self.agents.write();
        if agents.contains_key(&agent_id) {
            return Err(RegistryError::AlreadyExists(agent_id));
        }
        let handle = AgentHandle::spawn(agent_id.clone(), self.config);
        agents.insert(agent_id, handle.clone());
        Ok(handle)
    }

    pub fn restore(&self, snapshot: AgentSnapshot) -> Result<AgentHandle, RegistryError> {
        let agent_id = snapshot.agent_id.clone();
        let mut agents = self.agents.write();
        if agents.contains_key(&agent_id) {
            return Err(RegistryError::AlreadyExists(agent_id));
        }
        let handle = AgentHandle::spawn_with_snapshot(snapshot, self.config);
        agents.insert(agent_id, handle.clone());
        Ok(handle)
    }

    #[must_use]
    pub fn get(&self, agent_id: &AgentId) -> Option<AgentHandle> {
        self.agents.read().get(agent_id).cloned()
    }

    #[must_use]
    pub fn list(&self) -> Vec<AgentId> {
        self.agents.read().keys().cloned().collect()
    }

    pub async fn remove(&self, agent_id: &AgentId) -> Result<(), RegistryError> {
        let handle = self
            .agents
            .write()
            .remove(agent_id)
            .ok_or_else(|| RegistryError::NotFound(agent_id.clone()))?;
        handle.shutdown().await.map_err(RegistryError::Actor)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RegistryError {
    AlreadyExists(AgentId),
    NotFound(AgentId),
    Actor(AgentError),
}

impl fmt::Display for RegistryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyExists(agent_id) => write!(formatter, "agent {agent_id} already exists"),
            Self::NotFound(agent_id) => write!(formatter, "agent {agent_id} was not found"),
            Self::Actor(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for RegistryError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Actor(error) => Some(error),
            _ => None,
        }
    }
}
