use supernova_db::protocol::{decode_request, encode_request, Request, MAX_FRAME_SIZE};

#[test]
fn protocol_round_trip_and_rejects_malformed_frames() {
    let request = Request { op: "ping".into(), ..Request::default() };
    let bytes = encode_request(&request).unwrap();
    assert_eq!(decode_request(&bytes).unwrap().op, "ping");

    let mut trailing = bytes.clone();
    trailing.push(0);
    assert!(decode_request(&trailing).is_err());

    let mut oversized = b"SDB3".to_vec();
    oversized.extend_from_slice(&((MAX_FRAME_SIZE as u32 + 1).to_be_bytes()));
    assert!(decode_request(&oversized).is_err());
}
