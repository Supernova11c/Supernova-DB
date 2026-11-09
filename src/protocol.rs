use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::{self, Read, Write};

pub const MAGIC: &[u8; 4] = b"SDB2";
pub const MAX_FRAME_SIZE: usize = 4 * 1024 * 1024;
const HEADER_LEN: usize = 8;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Request {
    pub op: String,
    pub table: String,
    pub key: String,
    pub value: Option<Value>,
    pub field: Option<String>,
    pub query: Option<String>,
    pub token: Option<String>,
}

impl Default for Request {
    fn default() -> Self {
        Self {
            op: String::new(),
            table: String::new(),
            key: String::new(),
            value: None,
            field: None,
            query: None,
            token: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub ok: bool,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

pub fn encode_request(request: &Request) -> Result<Vec<u8>, String> {
    encode_frame(request)
}

pub fn encode_response(response: &Response) -> Result<Vec<u8>, String> {
    encode_frame(response)
}

fn encode_frame<T: Serialize>(payload: &T) -> Result<Vec<u8>, String> {
    let payload = serde_json::to_vec(payload).map_err(|e| e.to_string())?;
    if payload.len() > MAX_FRAME_SIZE || payload.len() > u32::MAX as usize {
        return Err("payload exceeds maximum frame size".to_string());
    }
    let mut frame = Vec::with_capacity(HEADER_LEN + payload.len());
    frame.extend_from_slice(MAGIC);
    frame.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    frame.extend_from_slice(&payload);
    Ok(frame)
}

pub fn decode_request(frame: &[u8]) -> Result<Request, String> {
    let payload = decode_payload(frame)?;
    serde_json::from_slice(payload).map_err(|e| format!("invalid JSON request: {e}"))
}

pub fn decode_response(frame: &[u8]) -> Result<Response, String> {
    let payload = decode_payload(frame)?;
    serde_json::from_slice(payload).map_err(|e| format!("invalid JSON response: {e}"))
}

fn decode_payload(frame: &[u8]) -> Result<&[u8], String> {
    if frame.len() < HEADER_LEN {
        return Err("frame is too short".into());
    }
    if &frame[..4] != MAGIC {
        return Err("invalid protocol magic".into());
    }
    let length = u32::from_be_bytes(frame[4..8].try_into().unwrap()) as usize;
    if length > MAX_FRAME_SIZE {
        return Err("frame exceeds maximum size".into());
    }
    let end = HEADER_LEN.checked_add(length).ok_or("frame length overflow")?;
    if frame.len() != end {
        return Err("frame length does not match payload".into());
    }
    Ok(&frame[HEADER_LEN..end])
}

pub fn read_request<R: Read>(reader: &mut R) -> io::Result<Request> {
    let payload = read_payload(reader)?;
    serde_json::from_slice(&payload).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

pub fn write_response<W: Write>(writer: &mut W, response: &Response) -> io::Result<()> {
    let frame = encode_response(response).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    writer.write_all(&frame)
}

fn read_payload<R: Read>(reader: &mut R) -> io::Result<Vec<u8>> {
    let mut header = [0u8; HEADER_LEN];
    reader.read_exact(&mut header)?;
    if &header[..4] != MAGIC {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "invalid protocol magic"));
    }
    let length = u32::from_be_bytes(header[4..8].try_into().unwrap()) as usize;
    if length > MAX_FRAME_SIZE {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "frame exceeds maximum size"));
    }
    let mut payload = vec![0; length];
    reader.read_exact(&mut payload)?;
    Ok(payload)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_round_trips_and_rejects_trailing_bytes() {
        let request = Request { op: "ping".into(), ..Request::default() };
        let frame = encode_request(&request).unwrap();
        assert_eq!(decode_request(&frame).unwrap().op, "ping");
        let mut bad = frame;
        bad.push(0);
        assert!(decode_request(&bad).is_err());
    }
}
