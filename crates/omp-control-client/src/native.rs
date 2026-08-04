use std::{future::Future, net::IpAddr, pin::Pin, sync::Arc, time::Duration};

use futures_util::{SinkExt, StreamExt};
use omp_control_protocol::TlsIdentityHint;
use rustls::{
    CertificateError, ClientConfig, DigitallySignedStruct, Error as TlsError, SignatureScheme,
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    crypto::{WebPkiSupportedAlgorithms, verify_tls12_signature, verify_tls13_signature},
    pki_types::{CertificateDer, ServerName, UnixTime},
};
use sha2::{Digest, Sha256};
use tokio::net::TcpStream;
use tokio_tungstenite::{
    Connector, MaybeTlsStream, WebSocketStream, connect_async, connect_async_tls_with_config,
    tungstenite::Message,
};
use url::Url;

use crate::{BinaryWebSocket, SocketEvent, SocketTarget, TransportError, WebSocketAdapter};

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
type NativeStream = WebSocketStream<MaybeTlsStream<TcpStream>>;

#[derive(Clone, Copy, Debug, Default)]
pub struct NativeWebSocketAdapter;

pub struct NativeWebSocket {
    stream: NativeStream,
}

impl WebSocketAdapter for NativeWebSocketAdapter {
    type Socket = NativeWebSocket;
    type ConnectFuture<'a> = BoxFuture<'a, Result<Self::Socket, TransportError>>;
    type SleepFuture<'a> = tokio::time::Sleep;

    fn connect<'a>(&'a self, target: &'a SocketTarget) -> Self::ConnectFuture<'a> {
        Box::pin(async move {
            validate_target(target)?;
            let (stream, _) = match &target.tls_identity {
                TlsIdentityHint::PubliclyTrusted | TlsIdentityHint::InsecureDevelopment => {
                    connect_async(&target.endpoint)
                        .await
                        .map_err(|error| TransportError::new(error.to_string()))?
                }
                TlsIdentityHint::Sha256Fingerprint(fingerprint) => {
                    let connector = pinned_connector(fingerprint)?;
                    connect_async_tls_with_config(&target.endpoint, None, false, Some(connector))
                        .await
                        .map_err(|error| TransportError::new(error.to_string()))?
                }
            };
            Ok(NativeWebSocket { stream })
        })
    }

    fn sleep(&self, duration: Duration) -> Self::SleepFuture<'_> {
        tokio::time::sleep(duration)
    }
}

impl BinaryWebSocket for NativeWebSocket {
    type SendFuture<'a> = BoxFuture<'a, Result<(), TransportError>>;
    type ReceiveFuture<'a> = BoxFuture<'a, Result<SocketEvent, TransportError>>;
    type CloseFuture<'a> = BoxFuture<'a, Result<(), TransportError>>;

    fn send_binary(&mut self, bytes: Vec<u8>) -> Self::SendFuture<'_> {
        Box::pin(async move {
            self.stream
                .send(Message::Binary(bytes.into()))
                .await
                .map_err(|error| TransportError::new(error.to_string()))
        })
    }

    fn receive(&mut self) -> Self::ReceiveFuture<'_> {
        Box::pin(async move {
            loop {
                let Some(message) = self.stream.next().await else {
                    return Ok(SocketEvent::Closed { reason: None });
                };
                match message.map_err(|error| TransportError::new(error.to_string()))? {
                    Message::Binary(bytes) => return Ok(SocketEvent::Binary(bytes.to_vec())),
                    Message::Ping(payload) => self
                        .stream
                        .send(Message::Pong(payload))
                        .await
                        .map_err(|error| TransportError::new(error.to_string()))?,
                    Message::Pong(_) | Message::Frame(_) => {}
                    Message::Close(frame) => {
                        return Ok(SocketEvent::Closed {
                            reason: frame.map(|frame| frame.reason.to_string()),
                        });
                    }
                    Message::Text(_) => {
                        return Err(TransportError::new(
                            "server sent text on the binary control protocol",
                        ));
                    }
                }
            }
        })
    }

    fn close(&mut self) -> Self::CloseFuture<'_> {
        Box::pin(async move {
            self.stream
                .send(Message::Close(None))
                .await
                .map_err(|error| TransportError::new(error.to_string()))
        })
    }
}

fn validate_target(target: &SocketTarget) -> Result<(), TransportError> {
    let url = Url::parse(&target.endpoint)
        .map_err(|error| TransportError::new(format!("invalid WebSocket endpoint: {error}")))?;
    match target.tls_identity {
        TlsIdentityHint::PubliclyTrusted | TlsIdentityHint::Sha256Fingerprint(_) => {
            if url.scheme() != "wss" {
                return Err(TransportError::new(
                    "trusted and pinned connections require a wss:// endpoint",
                ));
            }
        }
        TlsIdentityHint::InsecureDevelopment => {
            if url.scheme() != "ws" || !is_loopback(&url) {
                return Err(TransportError::new(
                    "insecure development connections require a loopback ws:// endpoint",
                ));
            }
        }
    }
    Ok(())
}

fn is_loopback(url: &Url) -> bool {
    let Some(host) = url.host_str() else {
        return false;
    };
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

fn pinned_connector(fingerprint: &str) -> Result<Connector, TransportError> {
    let expected = decode_fingerprint(fingerprint)?;
    let provider = rustls::crypto::aws_lc_rs::default_provider();
    let verifier = Arc::new(PinnedCertificateVerifier {
        expected,
        algorithms: provider.signature_verification_algorithms,
    });
    let config = ClientConfig::builder_with_provider(Arc::new(provider))
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(|error| TransportError::new(error.to_string()))?
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth();
    Ok(Connector::Rustls(Arc::new(config)))
}

fn decode_fingerprint(value: &str) -> Result<[u8; 32], TransportError> {
    if value.len() != 64 {
        return Err(TransportError::new(
            "SHA-256 certificate fingerprint must contain 64 hexadecimal characters",
        ));
    }
    let mut output = [0_u8; 32];
    for (index, byte) in output.iter_mut().enumerate() {
        let offset = index * 2;
        *byte = u8::from_str_radix(&value[offset..offset + 2], 16).map_err(|_| {
            TransportError::new("SHA-256 certificate fingerprint contains non-hexadecimal data")
        })?;
    }
    Ok(output)
}

#[derive(Debug)]
struct PinnedCertificateVerifier {
    expected: [u8; 32],
    algorithms: WebPkiSupportedAlgorithms,
}

impl ServerCertVerifier for PinnedCertificateVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, TlsError> {
        let actual = Sha256::digest(end_entity.as_ref());
        if actual.as_slice() != self.expected {
            return Err(TlsError::InvalidCertificate(
                CertificateError::ApplicationVerificationFailure,
            ));
        }
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        signature: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        verify_tls12_signature(message, cert, signature, &self.algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        signature: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        verify_tls13_signature(message, cert, signature, &self.algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algorithms.supported_schemes()
    }
}
