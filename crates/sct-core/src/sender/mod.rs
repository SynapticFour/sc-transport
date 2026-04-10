use crate::compression::maybe_compress;
use crate::protocol::{
    encode, read_framed, write_framed, ChunkDescriptor, CompressionType, FinalAck, ManifestAck,
    TransferComplete, TransferManifest,
};
use crate::transport::SctConnection;
use anyhow::Result;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::fs::File;
use tokio::io::AsyncReadExt;
use tokio::sync::Semaphore;

pub struct FileSender {
    connection: SctConnection,
    config: SenderConfig,
}

pub struct SenderConfig {
    pub chunk_size: usize,
    pub max_parallel_chunks: usize,
    pub compression: CompressionType,
    pub require_final_ack: bool,
    pub progress_callback: Option<Arc<dyn Fn(TransferProgress) + Send + Sync>>,
}

pub struct TransferProgress {
    pub bytes_sent: u64,
    pub bytes_total: u64,
    pub throughput_mbps: f64,
    pub elapsed: Duration,
    pub eta: Duration,
}

impl Default for SenderConfig {
    fn default() -> Self {
        Self {
            chunk_size: 4 * 1024 * 1024,
            max_parallel_chunks: 16,
            compression: CompressionType::None,
            require_final_ack: true,
            progress_callback: None,
        }
    }
}

impl FileSender {
    pub fn new(connection: SctConnection, config: SenderConfig) -> Self {
        Self { connection, config }
    }

    pub async fn send(&self, path: &Path) -> Result<()> {
        let mut file = File::open(path).await?;
        let meta = file.metadata().await?;
        let total_size = meta.len();
        let num_chunks = total_size.div_ceil(self.config.chunk_size as u64);
        let filename = path
            .file_name()
            .and_then(|f| f.to_str())
            .unwrap_or("payload.bin")
            .to_string();

        let mut full = Vec::with_capacity(total_size as usize);
        file.read_to_end(&mut full).await?;
        let checksum = *blake3::hash(&full).as_bytes();

        let hash = blake3::hash(filename.as_bytes());
        let mut transfer_id = [0_u8; 16];
        transfer_id.copy_from_slice(&hash.as_bytes()[..16]);
        let manifest = TransferManifest {
            transfer_id,
            filename,
            total_size,
            chunk_size: self.config.chunk_size as u32,
            num_chunks,
            checksum_algorithm: sct_proto::ChecksumAlg::Blake3,
            file_checksum: checksum,
            compression: self.config.compression.clone(),
            metadata: HashMap::new(),
        };

        let (mut ctrl_send, mut ctrl_recv) = self.connection.open_control_stream().await?;
        write_framed(&mut ctrl_send, &manifest).await?;
        let ack: ManifestAck = read_framed(&mut ctrl_recv).await?;
        if !ack.accepted {
            return Err(anyhow::anyhow!(
                "receiver rejected manifest: {}",
                ack.message.unwrap_or_else(|| "no reason".to_string())
            ));
        }
        let skip: std::collections::HashSet<u64> = ack.received_chunks.iter().copied().collect();

        let semaphore = Semaphore::new(self.config.max_parallel_chunks);
        let start = Instant::now();
        let mut sent = 0_u64;
        for idx in 0..num_chunks {
            if skip.contains(&idx) {
                continue;
            }
            let _permit = semaphore.acquire().await?;
            let off = idx as usize * self.config.chunk_size;
            let end = usize::min(off + self.config.chunk_size, full.len());
            let chunk_raw = &full[off..end];
            let chunk = maybe_compress(chunk_raw, &self.config.compression)?;
            let desc = ChunkDescriptor {
                index: idx,
                offset: off as u64,
                compressed_size: chunk.len() as u32,
                uncompressed_size: chunk_raw.len() as u32,
                checksum: *blake3::hash(&chunk).as_bytes(),
            };
            let mut data = self.connection.open_data_stream().await?;
            let desc_bytes = encode(&desc)?;
            data.write_all(&(desc_bytes.len() as u32).to_be_bytes()).await?;
            data.write_all(&desc_bytes).await?;
            data.write_all(&chunk).await?;
            data.finish()?;
            sent += chunk_raw.len() as u64;
            if let Some(cb) = &self.config.progress_callback {
                let elapsed = start.elapsed();
                let throughput_mbps = if elapsed.as_secs_f64() > 0.0 {
                    (sent as f64 * 8.0 / 1_000_000.0) / elapsed.as_secs_f64()
                } else {
                    0.0
                };
                cb(TransferProgress {
                    bytes_sent: sent,
                    bytes_total: total_size,
                    throughput_mbps,
                    elapsed,
                    eta: Duration::from_secs(0),
                });
            }
        }
        write_framed(
            &mut ctrl_send,
            &TransferComplete {
                transfer_id: manifest.transfer_id,
            },
        )
        .await?;
        match read_framed::<FinalAck, _>(&mut ctrl_recv).await {
            Ok(final_ack) => {
                if !final_ack.success {
                    return Err(anyhow::anyhow!(
                        "receiver verification failed: {}",
                        final_ack.message.unwrap_or_else(|| "unknown".to_string())
                    ));
                }
            }
            Err(e) => {
                if self.config.require_final_ack {
                    return Err(anyhow::anyhow!("missing final ack: {e}"));
                }
            }
        }
        Ok(())
    }
}
