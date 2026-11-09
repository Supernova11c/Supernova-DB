use serde::{Deserialize, Serialize};

pub const MAGIC: &[u8; 4] = b"SDB1";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    pub op: String,
    pub key: String,
    pub value: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub ok: bool,
    pub message: String,
    pub value: Option<String>,
}

pub fn encode_request(request: &Request) -> Vec<u8> {
    let payload = serde_json::to_vec(request).expect("request serialization should never fail");
    let mut frame = Vec::with_capacity(8 + payload.len());
    frame.extend_from_slice(MAGIC);
    frame.extend_from_slice(&((payload.len() as u32).to_be_bytes()));
    frame.extend_from_slice(&payload);
    frame
}

pub fn decode_request(frame: &[u8]) -> Result<Request, String> {
    if frame.len() < 8 {
        return Err("frame is too short".to_string());
    }

    if &frame[0..4] != MAGIC {
        return Err("invalid magic header".to_string());
    }

    let len = u32::from_be_bytes([frame[4], frame[5], frame[6], frame[7]]) as usize;
    let payload_start = 8;
    let payload_end = payload_start + len;

    if frame.len() < payload_end {
        return Err("frame payload is truncated".to_string());
    }

    let payload = &frame[payload_start..payload_end];
    serde_json::from_slice(payload).map_err(|err| format!("invalid JSON request: {err}"))
}

pub fn encode_response(response: &Response) -> Vec<u8> {
    let payload = serde_json::to_vec(response).expect("response serialization should never fail");
    let mut frame = Vec::with_capacity(8 + payload.len());
    frame.extend_from_slice(MAGIC);
    frame.extend_from_slice(&((payload.len() as u32).to_be_bytes()));
    frame.extend_from_slice(&payload);
    frame
}

pub fn decode_response(frame: &[u8]) -> Result<Response, String> {
    if frame.len() < 8 {
        return Err("frame is too short".to_string());
    }

    if &frame[0..4] != MAGIC {
        return Err("invalid magic header".to_string());
    }

    let len = u32::from_be_bytes([frame[4], frame[5], frame[6], frame[7]]) as usize;
    let payload_start = 8;
    let payload_end = payload_start + len;

    if frame.len() < payload_end {
        return Err("frame payload is truncated".to_string());
    }

    let payload = &frame[payload_start..payload_end];
    serde_json::from_slice(payload).map_err(|err| format!("invalid JSON response: {err}"))
}
