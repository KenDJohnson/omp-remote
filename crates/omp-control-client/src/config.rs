use std::{collections::BTreeSet, fmt, time::Duration};

use omp_control_protocol::{
    CAPABILITY_EVENT_REPLAY, CAPABILITY_INTERACTION_LEASES, CAPABILITY_STATE_DELTAS,
    ClientAuthentication, ClientDescriptor, DeviceDescriptor, FrameLimits, PairingBundle, ServerId,
};

use crate::SocketTarget;

pub const PAIRING_FORMAT_VERSION: u16 = 1;

#[derive(Clone, Debug)]
pub struct ClientConfig {
    pub target: SocketTarget,
    pub server_id: ServerId,
    pub client: ClientDescriptor,
    pub authentication: AuthenticationSource,
    pub frame_limits: FrameLimits,
    pub reconnect: ReconnectPolicy,
    pub event_subscriber_capacity: usize,
}

impl ClientConfig {
    #[must_use]
    pub fn stored(target: SocketTarget, server_id: ServerId, mut client: ClientDescriptor) -> Self {
        add_required_capabilities(&mut client);
        Self {
            target,
            server_id,
            client,
            authentication: AuthenticationSource::StoredCredential,
            frame_limits: FrameLimits::default(),
            reconnect: ReconnectPolicy::default(),
            event_subscriber_capacity: 256,
        }
    }

    pub fn pairing(
        bundle: PairingBundle,
        mut client: ClientDescriptor,
        device_name: impl Into<String>,
    ) -> Result<Self, ClientConfigError> {
        if bundle.format_version != PAIRING_FORMAT_VERSION {
            return Err(ClientConfigError::UnsupportedPairingFormat(
                bundle.format_version,
            ));
        }
        add_required_capabilities(&mut client);
        let authentication = AuthenticationSource::Pair(ClientAuthentication::Pair {
            pairing_id: bundle.pairing_id,
            secret: bundle.secret,
            device: DeviceDescriptor {
                name: device_name.into(),
                platform: client.platform,
            },
        });
        Ok(Self {
            target: SocketTarget {
                endpoint: bundle.endpoint,
                tls_identity: bundle.tls_identity,
            },
            server_id: bundle.server_id,
            client,
            authentication,
            frame_limits: FrameLimits::default(),
            reconnect: ReconnectPolicy::default(),
            event_subscriber_capacity: 256,
        })
    }
}

fn add_required_capabilities(client: &mut ClientDescriptor) {
    let required = BTreeSet::from([
        CAPABILITY_STATE_DELTAS.to_owned(),
        CAPABILITY_EVENT_REPLAY.to_owned(),
        CAPABILITY_INTERACTION_LEASES.to_owned(),
    ]);
    client.capabilities.requested.extend(required);
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AuthenticationSource {
    StoredCredential,
    Pair(ClientAuthentication),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReconnectPolicy {
    pub initial_delay: Duration,
    pub maximum_delay: Duration,
}

impl Default for ReconnectPolicy {
    fn default() -> Self {
        Self {
            initial_delay: Duration::from_millis(250),
            maximum_delay: Duration::from_secs(10),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClientConfigError {
    UnsupportedPairingFormat(u16),
}

impl fmt::Display for ClientConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedPairingFormat(version) => {
                write!(
                    formatter,
                    "unsupported pairing bundle format version {version}"
                )
            }
        }
    }
}

impl std::error::Error for ClientConfigError {}
