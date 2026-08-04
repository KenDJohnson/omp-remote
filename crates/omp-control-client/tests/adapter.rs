#![cfg(not(target_arch = "wasm32"))]

use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use omp_control_client::{
    BinaryWebSocket, NativeWebSocketAdapter, SocketEvent, SocketTarget, WebSocketAdapter,
};
use omp_control_protocol::TlsIdentityHint;
use rustls::{
    ServerConfig,
    pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer},
};
use sha2::{Digest, Sha256};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;
use tokio_tungstenite::accept_async;

#[tokio::test]
async fn native_adapter_connects_with_a_pinned_self_signed_certificate() {
    let rcgen::CertifiedKey { cert, signing_key } =
        rcgen::generate_simple_self_signed(vec!["localhost".to_owned()]).unwrap();
    let private_key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(signing_key.serialize_der()));
    let server_config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert.der().clone()], private_key)
        .unwrap();
    let acceptor = TlsAcceptor::from(Arc::new(server_config));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let tls = acceptor.accept(stream).await.unwrap();
        let mut socket = accept_async(tls).await.unwrap();
        let message = socket.next().await.unwrap().unwrap();
        socket.send(message).await.unwrap();
    });

    let fingerprint = Sha256::digest(cert.der().as_ref())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let mut socket = NativeWebSocketAdapter
        .connect(&SocketTarget {
            endpoint: format!("wss://localhost:{}/control", address.port()),
            tls_identity: TlsIdentityHint::Sha256Fingerprint(fingerprint),
        })
        .await
        .unwrap();
    socket.send_binary(vec![1, 2, 3]).await.unwrap();
    assert_eq!(
        socket.receive().await.unwrap(),
        SocketEvent::Binary(vec![1, 2, 3])
    );
    socket.close().await.unwrap();
    server.await.unwrap();
}

#[tokio::test]
async fn native_adapter_rejects_non_loopback_plaintext_before_connecting() {
    let result = NativeWebSocketAdapter
        .connect(&SocketTarget {
            endpoint: "ws://example.com/control".to_owned(),
            tls_identity: TlsIdentityHint::InsecureDevelopment,
        })
        .await;
    let Err(error) = result else {
        panic!("non-loopback plaintext unexpectedly connected")
    };
    assert!(error.to_string().contains("loopback ws://"));
}
