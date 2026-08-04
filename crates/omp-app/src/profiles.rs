use std::fmt;

use omp_control_client::{ClientConfig, SocketTarget};
use omp_control_protocol::{ClientDescriptor, PairingBundle, ServerId, TlsIdentityHint};
use serde::{Deserialize, Serialize};

#[cfg(target_arch = "wasm32")]
const PROFILE_STORAGE_KEY: &str = "omp-remote:server-profiles:v1";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedServerProfile {
    pub name: String,
    pub server_id: ServerId,
    pub endpoint: String,
    pub tls_identity: TlsIdentityHint,
}

impl SavedServerProfile {
    #[must_use]
    pub fn from_pairing(bundle: &PairingBundle, name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            server_id: bundle.server_id.clone(),
            endpoint: bundle.endpoint.clone(),
            tls_identity: bundle.tls_identity.clone(),
        }
    }

    #[must_use]
    pub fn client_config(&self, client: ClientDescriptor) -> ClientConfig {
        ClientConfig::stored(
            SocketTarget {
                endpoint: self.endpoint.clone(),
                tls_identity: self.tls_identity.clone(),
            },
            self.server_id.clone(),
            client,
        )
    }
}

pub fn load_profiles() -> Result<Vec<SavedServerProfile>, ProfileStoreError> {
    let encoded = load_encoded_profiles()?;
    let Some(encoded) = encoded else {
        return Ok(Vec::new());
    };
    serde_json::from_str(&encoded)
        .map_err(|_| ProfileStoreError::new("saved server profiles are invalid"))
}

pub fn save_profile(
    profile: SavedServerProfile,
) -> Result<Vec<SavedServerProfile>, ProfileStoreError> {
    let mut profiles = load_profiles()?;
    if let Some(existing) = profiles
        .iter_mut()
        .find(|existing| existing.server_id == profile.server_id)
    {
        *existing = profile;
    } else {
        profiles.push(profile);
    }
    profiles.sort_by(|left, right| left.name.cmp(&right.name));
    let encoded = serde_json::to_string(&profiles)
        .map_err(|_| ProfileStoreError::new("cannot encode server profiles"))?;
    save_encoded_profiles(&encoded)?;
    Ok(profiles)
}

#[cfg(target_arch = "wasm32")]
fn load_encoded_profiles() -> Result<Option<String>, ProfileStoreError> {
    browser_storage()?
        .get_item(PROFILE_STORAGE_KEY)
        .map_err(|_| ProfileStoreError::new("cannot read browser profile storage"))
}

#[cfg(target_arch = "wasm32")]
fn save_encoded_profiles(encoded: &str) -> Result<(), ProfileStoreError> {
    browser_storage()?
        .set_item(PROFILE_STORAGE_KEY, encoded)
        .map_err(|_| ProfileStoreError::new("cannot write browser profile storage"))
}

#[cfg(target_arch = "wasm32")]
fn browser_storage() -> Result<web_sys::Storage, ProfileStoreError> {
    web_sys::window()
        .ok_or_else(|| ProfileStoreError::new("browser window is unavailable"))?
        .local_storage()
        .map_err(|_| ProfileStoreError::new("cannot access browser profile storage"))?
        .ok_or_else(|| ProfileStoreError::new("browser profile storage is unavailable"))
}

#[cfg(not(target_arch = "wasm32"))]
fn load_encoded_profiles() -> Result<Option<String>, ProfileStoreError> {
    let path = profile_path()?;
    match std::fs::read_to_string(path) {
        Ok(encoded) => Ok(Some(encoded)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(ProfileStoreError::new(format!(
            "cannot read server profiles: {error}"
        ))),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn save_encoded_profiles(encoded: &str) -> Result<(), ProfileStoreError> {
    let path = profile_path()?;
    let parent = path
        .parent()
        .ok_or_else(|| ProfileStoreError::new("server profile path has no parent"))?;
    std::fs::create_dir_all(parent).map_err(|error| {
        ProfileStoreError::new(format!("cannot create server profile directory: {error}"))
    })?;
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, encoded).map_err(|error| {
        ProfileStoreError::new(format!("cannot write server profiles: {error}"))
    })?;
    std::fs::rename(&temporary, &path)
        .map_err(|error| ProfileStoreError::new(format!("cannot replace server profiles: {error}")))
}

#[cfg(not(target_arch = "wasm32"))]
fn profile_path() -> Result<std::path::PathBuf, ProfileStoreError> {
    directories::ProjectDirs::from("dev", "Oh My Pi", "OMP Remote")
        .map(|directories| directories.config_dir().join("profiles.json"))
        .ok_or_else(|| ProfileStoreError::new("platform configuration directory is unavailable"))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfileStoreError {
    message: String,
}

impl ProfileStoreError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for ProfileStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ProfileStoreError {}
