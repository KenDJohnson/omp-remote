use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use omp_control_client::{MAX_PAIRING_PAYLOAD_BYTES, PairingLinkError, decode_pairing_link};
use omp_control_protocol::{
    CborCodec, PairingBundle, PairingId, PairingSecret, ServerId, TlsIdentityHint,
};
use std::num::NonZeroU32;

#[test]
fn native_and_browser_fragment_links_decode_the_same_bundle() {
    let bundle = PairingBundle {
        format_version: 1,
        server_id: ServerId::new("server-1").unwrap(),
        endpoint: "wss://control.example.test/control".to_owned(),
        pairing_id: PairingId::new("pairing-1").unwrap(),
        secret: PairingSecret::new("secret"),
        expires_at_ms: 42,
        tls_identity: TlsIdentityHint::PubliclyTrusted,
    };
    let bytes = CborCodec::new(NonZeroU32::new(MAX_PAIRING_PAYLOAD_BYTES).unwrap())
        .encode(&bundle)
        .unwrap();
    let payload = URL_SAFE_NO_PAD.encode(bytes);

    assert_eq!(
        decode_pairing_link(&format!("omp-remote://pair#{payload}")).unwrap(),
        bundle
    );
    assert_eq!(
        decode_pairing_link(&format!("https://app.example.test/pair#{payload}")).unwrap(),
        bundle
    );
    assert_eq!(decode_pairing_link(&payload).unwrap(), bundle);
}

#[test]
fn malformed_and_oversized_pairing_payloads_fail_without_echoing_secrets() {
    let invalid = decode_pairing_link("omp-remote://pair#not%base64").unwrap_err();
    assert!(matches!(invalid, PairingLinkError::InvalidBase64(_)));
    assert!(!invalid.to_string().contains("not%base64"));

    let oversized = "a".repeat(MAX_PAIRING_PAYLOAD_BYTES as usize * 2);
    assert!(matches!(
        decode_pairing_link(&oversized),
        Err(PairingLinkError::PayloadTooLarge)
    ));
}
