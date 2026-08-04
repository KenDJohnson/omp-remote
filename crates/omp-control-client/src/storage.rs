use std::{collections::BTreeMap, fmt, sync::Arc};

#[cfg(not(target_arch = "wasm32"))]
use std::sync::LazyLock;

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

#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Debug)]
pub struct NativeCredentialStore {
    service: String,
}

#[cfg(not(target_arch = "wasm32"))]
impl NativeCredentialStore {
    #[must_use]
    pub fn new(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
        }
    }

    fn entry(&self, server_id: &ServerId) -> Result<keyring_core::Entry, CredentialStoreError> {
        initialize_native_store()?;
        keyring_core::Entry::new(&self.service, server_id.as_str())
            .map_err(|error| CredentialStoreError::new(format!("cannot open OS keyring: {error}")))
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl CredentialStore for NativeCredentialStore {
    type Error = CredentialStoreError;

    fn load(&self, server_id: &ServerId) -> Result<Option<DeviceCredential>, Self::Error> {
        let bytes = match self.entry(server_id)?.get_secret() {
            Ok(bytes) => bytes,
            Err(keyring_core::Error::NoEntry) => return Ok(None),
            Err(error) => {
                return Err(CredentialStoreError::new(format!(
                    "cannot read OS keyring credential: {error}"
                )));
            }
        };
        let credential: DeviceCredential = serde_json::from_slice(&bytes)
            .map_err(|_| CredentialStoreError::new("OS keyring credential is invalid"))?;
        if credential.server_id != *server_id {
            return Err(CredentialStoreError::new(
                "OS keyring credential belongs to another server",
            ));
        }
        Ok(Some(credential))
    }

    fn save(&self, credential: &DeviceCredential) -> Result<(), Self::Error> {
        let bytes = serde_json::to_vec(credential)
            .map_err(|_| CredentialStoreError::new("cannot encode device credential"))?;
        self.entry(&credential.server_id)?
            .set_secret(&bytes)
            .map_err(|error| {
                CredentialStoreError::new(format!("cannot write OS keyring credential: {error}"))
            })
    }

    fn remove(&self, server_id: &ServerId) -> Result<(), Self::Error> {
        match self.entry(server_id)?.delete_credential() {
            Ok(()) | Err(keyring_core::Error::NoEntry) => Ok(()),
            Err(error) => Err(CredentialStoreError::new(format!(
                "cannot remove OS keyring credential: {error}"
            ))),
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn initialize_native_store() -> Result<(), CredentialStoreError> {
    static INITIALIZED: LazyLock<Result<(), String>> =
        LazyLock::new(|| initialize_platform_store().map_err(|error| error.to_string()));
    INITIALIZED.clone().map_err(|error| {
        CredentialStoreError::new(format!("cannot initialize OS keyring: {error}"))
    })
}

#[cfg(target_os = "macos")]
fn initialize_platform_store() -> keyring_core::Result<()> {
    let store = apple_native_keyring_store::keychain::Store::new_with_configuration(
        &std::collections::HashMap::new(),
    )?;
    keyring_core::set_default_store(store);
    Ok(())
}

#[cfg(target_os = "ios")]
fn initialize_platform_store() -> keyring_core::Result<()> {
    let store = apple_native_keyring_store::protected::Store::new_with_configuration(
        &std::collections::HashMap::new(),
    )?;
    keyring_core::set_default_store(store);
    Ok(())
}

#[cfg(target_os = "android")]
fn initialize_platform_store() -> keyring_core::Result<()> {
    let store = android_native_keyring_store::Store::new_with_configuration(
        &std::collections::HashMap::new(),
    )?;
    keyring_core::set_default_store(store);
    Ok(())
}

#[cfg(target_os = "windows")]
fn initialize_platform_store() -> keyring_core::Result<()> {
    let store = windows_native_keyring_store::Store::new_with_configuration(
        &std::collections::HashMap::new(),
    )?;
    keyring_core::set_default_store(store);
    Ok(())
}

#[cfg(all(
    unix,
    not(any(target_os = "android", target_os = "ios", target_os = "macos"))
))]
fn initialize_platform_store() -> keyring_core::Result<()> {
    let store = zbus_secret_service_keyring_store::Store::new_with_configuration(
        &std::collections::HashMap::new(),
    )?;
    keyring_core::set_default_store(store);
    Ok(())
}

#[cfg(all(not(any(unix, windows)), not(target_arch = "wasm32")))]
fn initialize_platform_store() -> keyring_core::Result<()> {
    Err(keyring_core::Error::Invalid(
        "platform".to_owned(),
        "no secure credential store is configured".to_owned(),
    ))
}

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
