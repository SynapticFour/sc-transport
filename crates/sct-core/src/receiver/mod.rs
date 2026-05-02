use crate::compression::maybe_decompress;
use crate::protocol::{
    decode, read_framed, write_framed, ChunkDescriptor, FinalAck, ManifestAck,
    ReceiverFeedbackFrame, TransferComplete, TransferManifest,
};
use crate::transport::SctEndpoint;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::PathBuf;
use tokio::fs::{self, OpenOptions};
use tokio::io::SeekFrom;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

pub struct FileReceiver {
    endpoint: SctEndpoint,
    output_dir: PathBuf,
    config: ReceiverConfig,
}

pub struct ReceiverConfig {
    pub max_parallel_chunks: usize,
    pub verify_checksums: bool,
    pub resume_partial: bool,
    pub temp_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct ResumeState {
    received_chunks: Vec<u64>,
}

impl Default for ReceiverConfig {
    fn default() -> Self {
        Self {
            max_parallel_chunks: 16,
            verify_checksums: true,
            resume_partial: false,
            temp_dir: None,
        }
    }
}

impl FileReceiver {
    pub fn new(endpoint: SctEndpoint, output_dir: PathBuf, config: ReceiverConfig) -> Self {
        Self {
            endpoint,
            output_dir,
            config,
        }
    }

    pub async fn accept_transfer(&self) -> Result<PathBuf> {
        let conn = self
            .endpoint
            .accept()
            .await
            .ok_or_else(|| anyhow::anyhow!("no incoming connection"))??;
        let (mut ctrl_send, mut ctrl_recv) = conn.accept_control_stream().await?;
        let manifest: TransferManifest = read_framed(&mut ctrl_recv).await?;

        fs::create_dir_all(&self.output_dir).await?;
        let final_path = self.output_dir.join(&manifest.filename);
        let temp_base_dir = self
            .config
            .temp_dir
            .clone()
            .unwrap_or_else(|| self.output_dir.clone());
        fs::create_dir_all(&temp_base_dir).await?;
        let temp_path = temp_base_dir.join(format!(
            "{}.{}.part",
            manifest.filename,
            hex_transfer_id(manifest.transfer_id)
        ));
        let state_path = temp_base_dir.join(format!(
            "{}.{}.state.json",
            manifest.filename,
            hex_transfer_id(manifest.transfer_id)
        ));

        let mut received_chunks: HashSet<u64> = HashSet::new();
        if self.config.resume_partial && state_path.exists() && temp_path.exists() {
            if let Ok(raw) = fs::read(&state_path).await {
                if let Ok(state) = serde_json::from_slice::<ResumeState>(&raw) {
                    received_chunks.extend(state.received_chunks);
                }
            }
        }

        write_framed(
            &mut ctrl_send,
            &ManifestAck {
                accepted: true,
                message: None,
                received_chunks: received_chunks.iter().copied().collect(),
            },
        )
        .await?;

        let feedback_enabled = true;
        let feedback_every = std::env::var("SC_SCT_FEEDBACK_EVERY_CHUNKS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(8)
            .max(1);
        let mut feedback_stream = if feedback_enabled {
            match conn.open_control_stream().await {
                Ok((send, _recv)) => Some(send),
                Err(_) => None,
            }
        } else {
            None
        };

        let mut out = OpenOptions::new()
            .create(true)
            .truncate(!self.config.resume_partial)
            .write(true)
            .read(true)
            .open(&temp_path)
            .await?;
        if !self.config.resume_partial || out.metadata().await?.len() != manifest.total_size {
            out.set_len(manifest.total_size).await?;
        }

        let remaining = manifest
            .num_chunks
            .saturating_sub(received_chunks.len() as u64);
        for _ in 0..remaining {
            let mut stream = conn.accept_data_stream().await?;
            let mut len_buf = [0_u8; 4];
            stream.read_exact(&mut len_buf).await?;
            let desc_len = u32::from_be_bytes(len_buf) as usize;
            let mut desc_buf = vec![0_u8; desc_len];
            stream.read_exact(&mut desc_buf).await?;
            let desc: ChunkDescriptor = decode(&desc_buf)?;
            if received_chunks.contains(&desc.index) {
                // Already persisted in partial state; still consume payload bytes from stream.
                let mut discard = vec![0_u8; desc.compressed_size as usize];
                stream.read_exact(&mut discard).await?;
                continue;
            }
            let mut payload = vec![0_u8; desc.compressed_size as usize];
            stream.read_exact(&mut payload).await?;
            if self.config.verify_checksums {
                let got = *blake3::hash(&payload).as_bytes();
                if got != desc.checksum {
                    return Err(anyhow::anyhow!("chunk checksum mismatch at {}", desc.index));
                }
            }
            let chunk = maybe_decompress(&payload, &manifest.compression)?;
            out.seek(SeekFrom::Start(desc.offset)).await?;
            out.write_all(&chunk).await?;
            received_chunks.insert(desc.index);
            if let Some(ref mut fb_send) = feedback_stream {
                if (received_chunks.len() as u64).is_multiple_of(feedback_every) {
                    let frame = ReceiverFeedbackFrame {
                        transfer_id: manifest.transfer_id,
                        decode_delay_ms: if manifest.chunk_size > (1024 * 1024) {
                            20
                        } else {
                            8
                        },
                        buffer_occupancy: ((received_chunks.len() as f32)
                            / (manifest.num_chunks.max(1) as f32))
                            .clamp(0.0, 1.0),
                        cpu_load: 0.45,
                        loss_hint: 0.02,
                        rtt_ms: 25,
                        completed_block_id: Some(desc.index / 4),
                        block_reconstructable: true,
                    };
                    let _ = write_framed(fb_send, &frame).await;
                }
            }
            if self.config.resume_partial {
                let state = ResumeState {
                    received_chunks: received_chunks.iter().copied().collect(),
                };
                let raw = serde_json::to_vec(&state)?;
                fs::write(&state_path, raw).await?;
            }
        }

        out.flush().await?;
        out.seek(SeekFrom::Start(0)).await?;
        let mut full = Vec::with_capacity(manifest.total_size as usize);
        out.read_to_end(&mut full).await?;
        let full_hash = *blake3::hash(&full).as_bytes();
        let complete: TransferComplete = read_framed(&mut ctrl_recv).await?;
        if complete.transfer_id != manifest.transfer_id {
            write_framed(
                &mut ctrl_send,
                &FinalAck {
                    success: false,
                    message: Some("transfer_id mismatch".to_string()),
                },
            )
            .await?;
            return Err(anyhow::anyhow!("transfer id mismatch"));
        }
        if self.config.verify_checksums && full_hash != manifest.file_checksum {
            write_framed(
                &mut ctrl_send,
                &FinalAck {
                    success: false,
                    message: Some("file checksum mismatch".to_string()),
                },
            )
            .await?;
            return Err(anyhow::anyhow!("file checksum mismatch"));
        }
        write_framed(
            &mut ctrl_send,
            &FinalAck {
                success: true,
                message: None,
            },
        )
        .await?;
        fs::rename(&temp_path, &final_path).await?;
        if state_path.exists() {
            let _ = fs::remove_file(&state_path).await;
        }
        Ok(final_path)
    }
}

fn hex_transfer_id(id: [u8; 16]) -> String {
    id.iter().map(|b| format!("{b:02x}")).collect::<String>()
}
