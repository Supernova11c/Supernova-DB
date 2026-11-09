use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::{self, Read, Write};

pub const MAGIC: &[u8; 4] = b"SDB3";
pub const MAX_FRAME_SIZE: usize = 1024 * 1024;
const HEADER_SIZE: usize = 8;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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

fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>, String> {
    let payload = serde_json::to_vec(value).map_err(|e| e.to_string())?;
    if payload.len() > MAX_FRAME_SIZE { return Err("payload exceeds 1 MiB".into()); }
    let mut frame = Vec::with_capacity(HEADER_SIZE + payload.len());
    frame.extend_from_slice(MAGIC);
    frame.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    frame.extend_from_slice(&payload);
    Ok(frame)
}

pub fn encode_request(request: &Request) -> Result<Vec<u8>, String> { encode(request) }
pub fn encode_response(response: &Response) -> Result<Vec<u8>, String> { encode(response) }

fn decode<T: for<'de> Deserialize<'de>>(frame: &[u8]) -> Result<T, String> {
    if frame.len() < HEADER_SIZE || &frame[..4] != MAGIC { return Err("invalid or incomplete SDB3 frame".into()); }
    let size = u32::from_be_bytes(frame[4..8].try_into().map_err(|_| "invalid frame header")?) as usize;
    if size > MAX_FRAME_SIZE { return Err("frame exceeds 1 MiB".into()); }
    if frame.len() != HEADER_SIZE + size { return Err("frame length mismatch".into()); }
    serde_json::from_slice(&frame[HEADER_SIZE..]).map_err(|e| e.to_string())
}

pub fn decode_request(frame: &[u8]) -> Result<Request, String> { decode(frame) }
pub fn decode_response(frame: &[u8]) -> Result<Response, String> { decode(frame) }

pub fn read_request<R: Read>(reader: &mut R) -> io::Result<Request> {
    let mut header = [0; HEADER_SIZE];
    reader.read_exact(&mut header)?;
    if &header[..4] != MAGIC { return Err(io::Error::new(io::ErrorKind::InvalidData, "invalid SDB3 magic")); }
    let size = u32::from_be_bytes(header[4..8].try_into().expect("fixed header")) as usize;
    if size > MAX_FRAME_SIZE { return Err(io::Error::new(io::ErrorKind::InvalidData, "frame exceeds 1 MiB")); }
    let mut frame = Vec::with_capacity(HEADER_SIZE + size);
    frame.extend_from_slice(&header);
    frame.resize(HEADER_SIZE + size, 0);
    reader.read_exact(&mut frame[HEADER_SIZE..])?;
    decode_request(&frame).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

pub fn write_response<W: Write>(writer: &mut W, response: &Response) -> io::Result<()> {
    let frame = encode_response(response).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    writer.write_all(&frame)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn frame_round_trip_and_rejects_trailing_data() {
        let request = Request { op: "ping".into(), ..Request::default() };
        let frame = encode_request(&request).unwrap();
        assert_eq!(decode_request(&frame).unwrap().op, "ping");
        let mut bad = frame;
        bad.push(0);
        assert!(decode_request(&bad).is_err());
    }
}
