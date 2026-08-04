use std::{
    fmt::{self, Write as _},
    fs::File,
    io::BufReader,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::Arc,
};

use axum_server::tls_rustls::RustlsConfig;
use omp_control_protocol::TlsIdentityHint;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TlsMode {
    CertificateFiles {
        certificate: PathBuf,
        private_key: PathBuf,
    },
    TrustedReverseProxy {
        local_endpoint: SocketAddr,
    },
    PinnedSelfSigned {
        certificate: PathBuf,
        private_key: PathBuf,
    },
    DevelopmentPlaintext {
        local_endpoint: SocketAddr,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransportConfig {
    pub bind_address: SocketAddr,
    pub public_endpoint: String,
    pub tls_mode: TlsMode,
}

impl TransportConfig {
    pub fn validate(&self) -> Result<(), TransportConfigError> {
        match &self.tls_mode {
            TlsMode::CertificateFiles { .. } | TlsMode::PinnedSelfSigned { .. } => {
                if !self.public_endpoint.starts_with("wss://") {
                    return Err(TransportConfigError::SecureEndpointRequired);
                }
            }
            TlsMode::TrustedReverseProxy { local_endpoint } => {
                if !local_endpoint.ip().is_loopback() || *local_endpoint != self.bind_address {
                    return Err(TransportConfigError::ProxyMustBeLocal);
                }
                if !self.public_endpoint.starts_with("wss://") {
                    return Err(TransportConfigError::SecureEndpointRequired);
                }
            }
            TlsMode::DevelopmentPlaintext { local_endpoint } => {
                if !local_endpoint.ip().is_loopback() || *local_endpoint != self.bind_address {
                    return Err(TransportConfigError::PlaintextMustBeLoopback);
                }
                if !self.public_endpoint.starts_with("ws://") {
                    return Err(TransportConfigError::DevelopmentEndpointMustBePlaintext);
                }
            }
        }
        if !self.public_endpoint.ends_with("/control") {
            return Err(TransportConfigError::ControlPathRequired);
        }
        Ok(())
    }

    pub fn tls_identity_hint(&self) -> Result<TlsIdentityHint, TransportConfigError> {
        match &self.tls_mode {
            TlsMode::CertificateFiles { .. } | TlsMode::TrustedReverseProxy { .. } => {
                Ok(TlsIdentityHint::PubliclyTrusted)
            }
            TlsMode::PinnedSelfSigned { certificate, .. } => Ok(
                TlsIdentityHint::Sha256Fingerprint(certificate_fingerprint(certificate)?),
            ),
            TlsMode::DevelopmentPlaintext { .. } => Ok(TlsIdentityHint::InsecureDevelopment),
        }
    }

    #[must_use]
    pub fn uses_direct_tls(&self) -> bool {
        matches!(
            self.tls_mode,
            TlsMode::CertificateFiles { .. } | TlsMode::PinnedSelfSigned { .. }
        )
    }
}

pub fn load_rustls_config(
    certificate_path: &Path,
    private_key_path: &Path,
) -> Result<RustlsConfig, TransportConfigError> {
    let certificates = read_certificates(certificate_path)?;
    let private_key = read_private_key(private_key_path)?;
    let mut config =
        rustls::ServerConfig::builder_with_protocol_versions(&[&rustls::version::TLS13])
            .with_no_client_auth()
            .with_single_cert(certificates, private_key)
            .map_err(|error| TransportConfigError::Tls(error.to_string()))?;
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(RustlsConfig::from_config(Arc::new(config)))
}

fn read_certificates(path: &Path) -> Result<Vec<CertificateDer<'static>>, TransportConfigError> {
    let file = File::open(path).map_err(|error| TransportConfigError::File {
        path: path.to_owned(),
        error: error.to_string(),
    })?;
    let certificates: Vec<_> = rustls_pemfile::certs(&mut BufReader::new(file))
        .collect::<Result<_, _>>()
        .map_err(|error| TransportConfigError::File {
            path: path.to_owned(),
            error: error.to_string(),
        })?;
    if certificates.is_empty() {
        return Err(TransportConfigError::NoCertificates(path.to_owned()));
    }
    Ok(certificates)
}

fn read_private_key(path: &Path) -> Result<PrivateKeyDer<'static>, TransportConfigError> {
    let file = File::open(path).map_err(|error| TransportConfigError::File {
        path: path.to_owned(),
        error: error.to_string(),
    })?;
    rustls_pemfile::private_key(&mut BufReader::new(file))
        .map_err(|error| TransportConfigError::File {
            path: path.to_owned(),
            error: error.to_string(),
        })?
        .ok_or_else(|| TransportConfigError::NoPrivateKey(path.to_owned()))
}

fn certificate_fingerprint(path: &Path) -> Result<String, TransportConfigError> {
    let certificate = read_certificates(path)?
        .into_iter()
        .next()
        .expect("certificate collection was validated as non-empty");
    let hash = Sha256::digest(certificate.as_ref());
    let mut fingerprint = String::with_capacity(hash.len() * 2);
    for byte in hash {
        write!(&mut fingerprint, "{byte:02x}")
            .expect("writing a certificate fingerprint to a string cannot fail");
    }
    Ok(fingerprint)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TransportConfigError {
    SecureEndpointRequired,
    DevelopmentEndpointMustBePlaintext,
    ControlPathRequired,
    ProxyMustBeLocal,
    PlaintextMustBeLoopback,
    File { path: PathBuf, error: String },
    NoCertificates(PathBuf),
    NoPrivateKey(PathBuf),
    Tls(String),
}

impl fmt::Display for TransportConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SecureEndpointRequired => {
                formatter.write_str("non-development public endpoint must use wss://")
            }
            Self::DevelopmentEndpointMustBePlaintext => {
                formatter.write_str("development plaintext endpoint must use ws://")
            }
            Self::ControlPathRequired => {
                formatter.write_str("public endpoint must end with /control")
            }
            Self::ProxyMustBeLocal => formatter
                .write_str("trusted reverse proxy listener must bind its loopback endpoint"),
            Self::PlaintextMustBeLoopback => {
                formatter.write_str("development plaintext listener must bind loopback")
            }
            Self::File { path, error } => {
                write!(formatter, "failed to read {}: {error}", path.display())
            }
            Self::NoCertificates(path) => {
                write!(formatter, "{} contains no certificates", path.display())
            }
            Self::NoPrivateKey(path) => {
                write!(formatter, "{} contains no private key", path.display())
            }
            Self::Tls(error) => write!(formatter, "invalid TLS configuration: {error}"),
        }
    }
}

impl std::error::Error for TransportConfigError {}
