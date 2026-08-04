use std::{collections::BTreeMap, fmt, sync::Arc};

use omp_control_protocol::{DeviceCredential, ServerId};
use parking_lot::RwLock;

pub trait CredentialStore {
    type Error: std::error::Error + 'static;

    fn load(&self, server_id: &ServerId) -> Result<Option<DeviceCredential>, Self::Error>;
    fn save(&self, credential: &DeviceCredential) -> Result<(), Self::Error>;
    fn remove(&self, server_id: &ServerId) -> Result<(), Self::Error>;
}

#[derive(Clone, Debug, Default)]
pub struct MemoryCredentialStore {
    credentials: Arc<RwLock<BTreeMap<ServerId, DeviceCredential>>>,
}

impl CredentialStore for MemoryCredentialStore {
    type Error = CredentialStoreError;

    fn load(&self, server_id: &ServerId) -> Result<Option<DeviceCredential>, Self::Error> {
        Ok(self.credentials.read().get(server_id).cloned())
    }

    fn save(&self, credential: &DeviceCredential) -> Result<(), Self::Error> {
        self.credentials
            .write()
            .insert(credential.server_id.clone(), credential.clone());
        Ok(())
    }

    fn remove(&self, server_id: &ServerId) -> Result<(), Self::Error> {
        self.credentials.write().remove(server_id);
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CredentialStoreError {
    message: String,
}

impl CredentialStoreError {
    #[must_use]
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for CredentialStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for CredentialStoreError {}

#[cfg(target_arch = "wasm32")]
#[derive(Clone, Debug)]
pub struct BrowserCredentialStore {
    namespace: String,
}

#[cfg(target_arch = "wasm32")]
impl BrowserCredentialStore {
    #[must_use]
    pub fn new(namespace: impl Into<String>) -> Self {
        Self {
            namespace: namespace.into(),
        }
    }

    fn key(&self, server_id: &ServerId) -> String {
        format!("{}:{}", self.namespace, server_id.as_str())
    }

    fn storage(&self) -> Result<web_sys::Storage, CredentialStoreError> {
        web_sys::window()
            .ok_or_else(|| CredentialStoreError::new("browser window is unavailable"))?
            .local_storage()
            .map_err(|_| CredentialStoreError::new("cannot access browser local storage"))?
            .ok_or_else(|| CredentialStoreError::new("browser local storage is unavailable"))
    }
}

#[cfg(target_arch = "wasm32")]
impl CredentialStore for BrowserCredentialStore {
    type Error = CredentialStoreError;

    fn load(&self, server_id: &ServerId) -> Result<Option<DeviceCredential>, Self::Error> {
        let Some(encoded) = self
            .storage()?
            .get_item(&self.key(server_id))
            .map_err(|_| CredentialStoreError::new("cannot read browser credential storage"))?
        else {
            return Ok(None);
        };
        let credential: DeviceCredential = serde_json::from_str(&encoded)
            .map_err(|_| CredentialStoreError::new("stored device credential is invalid"))?;
        if credential.server_id != *server_id {
            return Err(CredentialStoreError::new(
                "stored device credential belongs to another server",
            ));
        }
        Ok(Some(credential))
    }

    fn save(&self, credential: &DeviceCredential) -> Result<(), Self::Error> {
        let encoded = serde_json::to_string(credential)
            .map_err(|_| CredentialStoreError::new("cannot encode device credential"))?;
        self.storage()?
            .set_item(&self.key(&credential.server_id), &encoded)
            .map_err(|_| CredentialStoreError::new("cannot write browser credential storage"))
    }

    fn remove(&self, server_id: &ServerId) -> Result<(), Self::Error> {
        self.storage()?
            .remove_item(&self.key(server_id))
            .map_err(|_| CredentialStoreError::new("cannot remove browser device credential"))
    }
}
