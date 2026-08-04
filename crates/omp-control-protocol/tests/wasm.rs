#![cfg(target_arch = "wasm32")]

#[path = "vectors/client_ping_v1.rs"]
mod client_ping_v1;

use omp_control_protocol::{ClientFrame, ConnectionPhase, FrameLimits, Ping};
use wasm_bindgen_test::wasm_bindgen_test;

#[wasm_bindgen_test]
fn cbor_encoding_matches_the_native_golden_vector() {
    let codec = FrameLimits::default().codec(ConnectionPhase::PreAuth);
    let frame = ClientFrame::Ping(Ping { nonce: 7 });
    let expected = decode_hex(client_ping_v1::HEX);
    assert_eq!(codec.encode(&frame).unwrap(), expected);
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
