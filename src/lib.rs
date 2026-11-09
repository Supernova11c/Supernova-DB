pub mod protocol;
pub mod storage;

pub use storage::{CollectionInfo, SupernovaDb};

pub const APP_NAME: &str = "Supernova DB";
pub fn app_version() -> &'static str { env!("CARGO_PKG_VERSION") }
