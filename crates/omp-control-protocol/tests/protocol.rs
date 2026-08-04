#[path = "vectors/client_ping_v1.rs"]
mod client_ping_v1;

use std::{collections::BTreeSet, num::NonZeroU32};

use omp_control_protocol::*;
use serde::Serialize;

fn hello(versions: Vec<ProtocolVersion>) -> ClientFrame {
    ClientFrame::Hello(ClientHello {
        supported_versions: versions,
        client: ClientDescriptor {
            name: "test-client".into(),
            version: "1.2.3".into(),
            platform: ClientPlatform::Cli,
            capabilities: ClientCapabilities {
                requested: BTreeSet::from([
                    CAPABILITY_EVENT_REPLAY.to_owned(),
                    CAPABILITY_STATE_DELTAS.to_owned(),
                ]),
            },
        },
        authentication: ClientAuthentication::Device {
            device_id: DeviceId::new("device-1").unwrap(),
            token: DeviceToken::new("secret-token"),
        },
        resume: ResumeState::default(),
    })
}

#[test]
fn client_and_server_frames_round_trip_as_one_cbor_message() {
    let codec = FrameLimits::default().codec(ConnectionPhase::PreAuth);
    let client = hello(vec![ProtocolVersion::V1]);
    let encoded = codec.encode(&client).unwrap();
    assert_eq!(codec.decode::<ClientFrame>(&encoded).unwrap(), client);

    let welcome = ServerFrame::Welcome(ServerWelcome {
        protocol_version: ProtocolVersion::V1,
        server_id: ServerId::new("server-1").unwrap(),
        connection_id: ConnectionId::new("connection-1").unwrap(),
        device_id: DeviceId::new("device-1").unwrap(),
        capabilities: ServerCapabilities {
            enabled: BTreeSet::from([CAPABILITY_STATE_DELTAS.to_owned()]),
            max_frame_bytes: DEFAULT_POST_AUTH_FRAME_BYTES,
        },
        heartbeat_interval_ms: 15_000,
    });
    let encoded = FrameLimits::default()
        .codec(ConnectionPhase::Authenticated)
        .encode(&welcome)
        .unwrap();
    assert_eq!(
        FrameLimits::default()
            .codec(ConnectionPhase::Authenticated)
            .decode::<ServerFrame>(&encoded)
            .unwrap(),
        welcome
    );
}

#[test]
fn unsupported_versions_and_non_hello_frames_fail_negotiation() {
    assert_eq!(
        negotiate_client_hello(&hello(vec![ProtocolVersion(99)])),
        Err(ProtocolNegotiationError::UnsupportedVersions)
    );
    assert_eq!(
        negotiate_client_hello(&ClientFrame::Ping(Ping { nonce: 1 })),
        Err(ProtocolNegotiationError::ExpectedHello)
    );

    let frame = hello(vec![ProtocolVersion(99), ProtocolVersion::V1]);
    let (_, version) = negotiate_client_hello(&frame).unwrap();
    assert_eq!(version, ProtocolVersion::V1);
}

#[test]
fn capability_negotiation_enables_only_requested_supported_features() {
    let available = BTreeSet::from([
        CAPABILITY_INTERACTION_LEASES.to_owned(),
        CAPABILITY_STATE_DELTAS.to_owned(),
    ]);
    let requested = ClientCapabilities {
        requested: BTreeSet::from([
            CAPABILITY_EVENT_REPLAY.to_owned(),
            CAPABILITY_STATE_DELTAS.to_owned(),
        ]),
    };
    let negotiated = ServerCapabilities::negotiate(&available, &requested, 64 * 1_024);
    assert_eq!(
        negotiated.enabled,
        BTreeSet::from([CAPABILITY_STATE_DELTAS.to_owned()])
    );
}

#[test]
fn codec_enforces_limits_and_exactly_one_frame() {
    let codec = CborCodec::new(NonZeroU32::new(32).unwrap());
    assert!(matches!(
        codec.encode(&"x".repeat(128)),
        Err(CborCodecError::FrameTooLarge { limit: 32 })
    ));
    assert!(matches!(
        codec.decode::<ClientFrame>(&[0_u8; 33]),
        Err(CborCodecError::FrameTooLarge { limit: 32 })
    ));

    let codec = FrameLimits::default().codec(ConnectionPhase::PreAuth);
    let encoded = codec.encode(&ClientFrame::Ping(Ping { nonce: 7 })).unwrap();
    let mut two_frames = encoded.clone();
    two_frames.extend_from_slice(&encoded);
    assert_eq!(
        codec.decode::<ClientFrame>(&two_frames),
        Err(CborCodecError::TrailingData)
    );
}

#[derive(Serialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
enum FutureClientFrame {
    Ping(FuturePing),
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FuturePing {
    nonce: u64,
    optional_addition: Option<String>,
}

#[test]
fn optional_field_additions_remain_decodable() {
    let codec = FrameLimits::default().codec(ConnectionPhase::PreAuth);
    let future = FutureClientFrame::Ping(FuturePing {
        nonce: 42,
        optional_addition: Some("new capability".into()),
    });
    let encoded = codec.encode(&future).unwrap();
    assert_eq!(
        codec.decode::<ClientFrame>(&encoded).unwrap(),
        ClientFrame::Ping(Ping { nonce: 42 })
    );
}

#[test]
fn mutating_requests_require_stable_operation_ids() {
    let read = RequestEnvelope {
        request_id: RequestId::new("request-1").unwrap(),
        operation_id: None,
        request: ControlRequest::ListAgents,
    };
    assert_eq!(read.validate(), Ok(()));

    let mutation = RequestEnvelope {
        request_id: RequestId::new("request-2").unwrap(),
        operation_id: None,
        request: ControlRequest::StopAgent {
            agent_id: AgentId::new("agent-1").unwrap(),
        },
    };
    assert_eq!(
        mutation.validate(),
        Err(RequestValidationError::MissingOperationId)
    );
}

#[test]
fn golden_ping_vector_is_stable() {
    let codec = FrameLimits::default().codec(ConnectionPhase::PreAuth);
    let frame = ClientFrame::Ping(Ping { nonce: 7 });
    let encoded = codec.encode(&frame).unwrap();
    let expected = decode_hex(client_ping_v1::HEX);
    assert_eq!(encoded, expected);
    assert_eq!(codec.decode::<ClientFrame>(&expected).unwrap(), frame);
}

fn decode_hex(value: &str) -> Vec<u8> {
    let compact: String = value
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect();
    assert_eq!(compact.len() % 2, 0);
    (0..compact.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&compact[index..index + 2], 16).unwrap())
        .collect()
}
