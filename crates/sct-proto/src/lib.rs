use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferManifest {
    pub transfer_id: [u8; 16],
    pub filename: String,
    pub total_size: u64,
    pub chunk_size: u32,
    pub num_chunks: u64,
    pub checksum_algorithm: ChecksumAlg,
    pub file_checksum: [u8; 32],
    pub compression: CompressionType,
    pub metadata: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ChecksumAlg {
    Blake3,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CompressionType {
    None,
    Zstd { level: i32 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkDescriptor {
    pub index: u64,
    pub offset: u64,
    pub compressed_size: u32,
    pub uncompressed_size: u32,
    pub checksum: [u8; 32],
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestAck {
    pub accepted: bool,
    pub message: Option<String>,
    pub received_chunks: Vec<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FinalAck {
    pub success: bool,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferComplete {
    pub transfer_id: [u8; 16],
}
