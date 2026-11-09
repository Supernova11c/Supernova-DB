use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::{self, Read, Write};

pub const MAGIC: &[u8; 4] = b"SDB3";
pub const MAX_FRAME_SIZE: usize = 1024 * 1024;
const HEADER_LEN: usize = 8;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Request {
    pub op: String,
    pub collection: String,
    pub id: String,
    pub document: Option<Value>,
    pub field: Option<String>,
    pub query: Option<String>,
    pub vector: Option<Vec<f64>>,
    pub dimensions: Option<usize>,
    pub limit: Option<usize>,
    pub token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub ok: bool,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

pub fn encode_request(request: &Request) -> Result<Vec<u8>, String> { encode(request) }
pub fn encode_response(response: &Response) -> Result<Vec<u8>, String> { encode(response) }

fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>, String> {
    let payload = serde_json::to_vec(value).map_err(|e| e.to_string())?;
    if payload.len() > MAX_FRAME_SIZE { return Err("payload exceeds 1 MiB limit".into()); }
    let mut frame = Vec::with_capacity(HEADER_LEN + payload.len());
    frame.extend_from_slice(MAGIC);
    frame.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    frame.extend_from_slice(&payload);
    Ok(frame)
}

fn read_payload<R: Read>(reader: &mut R) -> io::Result<Vec<u8>> {
    let mut header = [0u8; HEADER_LEN];
    reader.read_exact(&mut header)?;
    if &header[..4] != MAGIC { return Err(io::Error::new(io::ErrorKind::InvalidData, "invalid SDB3 magic")); }
    let len = u32::from_be_bytes(header[4..8].try_into().expect("fixed header")) as usize;
    if len > MAX_FRAME_SIZE { return Err(io::Error::new(io::ErrorKind::InvalidData, "frame exceeds 1 MiB limit")); }
    let mut payload = vec![0; len];
    reader.read_exact(&mut payload)?;
    Ok(payload)
}

pub fn read_request<R: Read>(reader: &mut R) -> io::Result<Request> {
    let payload = read_payload(reader)?;
    serde_json::from_slice(&payload).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

pub fn write_response<W: Write>(writer: &mut W, response: &Response) -> io::Result<()> {
    let frame = encode_response(response).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    writer.write_all(&frame)
}

pub fn decode_request(frame: &[u8]) -> Result<Request, String> {
    let payload = decode_frame(frame)?;
    serde_json::from_slice(payload).map_err(|e| format!("invalid request JSON: {e}"))
}

pub fn decode_response(frame: &[u8]) -> Result<Response, String> {
    let payload = decode_frame(frame)?;
    serde_json::from_slice(payload).map_err(|e| format!("invalid response JSON: {e}"))
}

fn decode_frame(frame: &[u8]) -> Result<&[u8], String> {
    if frame.len() < HEADER_LEN || &frame[..4] != MAGIC { return Err("invalid or short SDB3 frame".into()); }
    let len = u32::from_be_bytes(frame[4..8].try_into().expect("fixed header")) as usize;
    if len > MAX_FRAME_SIZE { return Err("frame exceeds 1 MiB limit".into()); }
    if frame.len() != HEADER_LEN + len { return Err("frame length mismatch".into()); }
    Ok(&frame[HEADER_LEN..])
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn frame_round_trip_and_size_limit() {
        let request = Request { op: "ping".into(), ..Request::default() };
        let bytes = encode_request(&request).unwrap();
        assert_eq!(decode_request(&bytes).unwrap().op, "ping");
        let mut trailing = bytes;
        trailing.push(0);
        assert!(decode_request(&trailing).is_err());
        assert!(encode(&"x".repeat(MAX_FRAME_SIZE + 1)).is_err());
    }
}
