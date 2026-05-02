pub mod adaptive;
pub mod compression;
pub mod congestion;
pub mod metrics;
pub mod protocol;
pub mod receiver;
pub mod sender;
pub mod transport;

pub const SPARQ_VERSION: &str = env!("CARGO_PKG_VERSION");
