pub mod protocol;
pub mod storage;

pub use storage::{SupernovaDb, Transaction};

pub const APP_NAME: &str = "Supernova DB";
pub const PROTOCOL_MAGIC: &[u8; 4] = b"SDB2";

pub fn app_version() -> &'static str { env!("CARGO_PKG_VERSION") }
