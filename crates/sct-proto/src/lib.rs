use serde::{Deserialize, Serialize};
use std::collections::HashMap;

fn default_data_shards() -> usize {
    4
}

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
    /// FEC data shards (must match sender RS block width). Default keeps older manifests valid.
    #[serde(default = "default_data_shards")]
    pub data_shards: usize,
    /// FEC parity shards (0 = no Reed–Solomon parity on the wire).
    #[serde(default)]
    pub parity_shards: usize,
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
    /// True when the chunk bytes on the wire are ZSTD-compressed; false when passed through
    /// (e.g. already-compressed payload or expansion guard). Receiver must not call zstd decode
    /// when false. Omitted in older encodings → `false` via `serde(default)`.
    #[serde(default)]
    pub was_compressed: bool,
    #[serde(default)]
    pub is_parity: bool,
    #[serde(default)]
    pub parity_index: usize,
    #[serde(default)]
    pub fec_group: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestAck {
    pub accepted: bool,
    pub message: Option<String>,
    pub received_chunks: Vec<u64>,
    #[serde(default)]
    pub chunk_hashes: Vec<[u8; 32]>, // index i → BLAKE3-Hash von Chunk i beim Receiver; leer = kein Delta-Info
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReceiverFeedbackFrame {
    pub transfer_id: [u8; 16],
    pub decode_delay_ms: u32,
    pub buffer_occupancy: f32,
    pub cpu_load: f32,
    pub loss_hint: f32,
    pub rtt_ms: u32,
    #[serde(default)]
    pub completed_block_id: Option<u64>,
    #[serde(default)]
    pub block_reconstructable: bool,
    #[serde(default)]
    pub missing_chunk_indices: Vec<u64>,
}
