use supernova_db::protocol::{decode_request, encode_request, Request};
#[test]
fn protocol_round_trip(){let req=Request{op:"ping".into(),..Request::default()};let bytes=encode_request(&req).unwrap();assert_eq!(decode_request(&bytes).unwrap().op,"ping");}
