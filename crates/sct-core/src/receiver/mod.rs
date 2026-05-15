use crate::compression::maybe_decompress;
use crate::protocol::{
    decode, read_framed, write_framed, ChunkDescriptor, FinalAck, ManifestAck,
    ReceiverFeedbackFrame, TransferComplete, TransferManifest,
};
use crate::transport::SctEndpoint;
use anyhow::Result;
use reed_solomon_erasure::galois_8::ReedSolomon;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
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

/// Comma-separated chunk indices dropped before persist/FEC (`test-hooks` feature only).
#[cfg(feature = "test-hooks")]
pub const TEST_SIMULATE_LOST_CHUNK_INDICES_ENV: &str = "SC_SCT_TEST_SIMULATE_LOST_CHUNK_INDICES";

#[cfg(feature = "test-hooks")]
fn simulated_lost_chunks_from_env() -> HashSet<u64> {
    std::env::var(TEST_SIMULATE_LOST_CHUNK_INDICES_ENV)
        .ok()
        .map(|s| s.split(',').filter_map(|p| p.trim().parse().ok()).collect())
        .unwrap_or_default()
}

#[cfg(not(feature = "test-hooks"))]
#[inline]
fn simulated_lost_chunks_from_env() -> HashSet<u64> {
    HashSet::new()
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

        let fec_rs = if manifest.parity_shards > 0 && manifest.data_shards > 0 {
            Some(
                ReedSolomon::new(manifest.data_shards, manifest.parity_shards)
                    .map_err(|e| anyhow::anyhow!("invalid fec dimensions: {e:?}"))?,
            )
        } else {
            None
        };
        let mut fec_groups: HashMap<u64, Vec<Option<Vec<u8>>>> = HashMap::new();
        let simulate_lost = simulated_lost_chunks_from_env();

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

        // Delta-Hashing: wenn resume_partial aktiv und die Datei bereits existiert,
        // hashe jeden vorhandenen Chunk und sende die Hashes mit.
        let chunk_hashes: Vec<[u8; 32]> = if self.config.resume_partial
            && tokio::fs::try_exists(&temp_path).await.unwrap_or(false)
        {
            let existing = tokio::fs::read(&temp_path).await.unwrap_or_default();
            (0..manifest.num_chunks)
                .map(|i| {
                    let off = i as usize * manifest.chunk_size as usize;
                    let end = (off + manifest.chunk_size as usize).min(existing.len());
                    if end > off {
                        *blake3::hash(&existing[off..end]).as_bytes()
                    } else {
                        [0u8; 32] // Chunk existiert nicht → Null-Hash → kein Match
                    }
                })
                .collect()
        } else {
            vec![]
        };

        // Open server→client feedback control stream before ManifestAck so the client can
        // `accept_bi` it as soon as `send_adaptive` starts, before any data uni streams arrive.
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

        write_framed(
            &mut ctrl_send,
            &ManifestAck {
                accepted: true,
                message: None,
                received_chunks: received_chunks.iter().copied().collect(),
                chunk_hashes,
            },
        )
        .await?;

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

        let total_chunks = manifest.num_chunks as usize;
        // Adaptive / multipath senders may open extra data streams (e.g. duplicates). A fixed
        // `remaining` stream count would stop early when a duplicate chunk consumes a slot,
        // leaving holes and a bad final file hash — keep accepting until all indices are filled.
        // Parity-Streams: jede FEC-Gruppe sendet parity_shards zusätzliche Streams.
        let fec_group_count = total_chunks.div_ceil(manifest.data_shards.max(1));
        let expected_parity_streams = fec_group_count * manifest.parity_shards;
        let max_streams = total_chunks
            .saturating_sub(received_chunks.len())
            .saturating_add(expected_parity_streams)
            .saturating_add(64); // Puffer für Duplikate / Retransmits
        let mut accepted_streams = 0usize;
        while received_chunks.len() < total_chunks {
            if accepted_streams >= max_streams {
                return Err(anyhow::anyhow!(
                    "incomplete transfer: {} of {} chunk indices after {} data streams",
                    received_chunks.len(),
                    total_chunks,
                    accepted_streams
                ));
            }
            accepted_streams += 1;
            let mut stream = conn.accept_data_stream().await?;
            let mut len_buf = [0_u8; 4];
            stream.read_exact(&mut len_buf).await?;
            let desc_len = u32::from_be_bytes(len_buf) as usize;
            let mut desc_buf = vec![0_u8; desc_len];
            stream.read_exact(&mut desc_buf).await?;
            let desc: ChunkDescriptor = decode(&desc_buf)?;

            if desc.is_parity {
                let mut payload = vec![0_u8; desc.compressed_size as usize];
                stream.read_exact(&mut payload).await?;
                let Some(ref rs) = fec_rs else {
                    continue;
                };
                if self.config.verify_checksums {
                    let got = *blake3::hash(&payload).as_bytes();
                    if got != desc.checksum {
                        return Err(anyhow::anyhow!(
                            "parity checksum mismatch fec_group={} parity_index={}",
                            desc.fec_group,
                            desc.parity_index
                        ));
                    }
                }
                let g = desc.fec_group;
                let group = fec_groups
                    .entry(g)
                    .or_insert_with(|| vec![None; manifest.data_shards + manifest.parity_shards]);
                let pi = desc.parity_index.saturating_add(manifest.data_shards);
                if pi < group.len() {
                    // RS parity column on the sender is the raw encoded row (`row`), not
                    // `frame_fec_wire_shard(row)`; data columns use the full framed chunk payload.
                    group[pi] = Some(payload);
                }
                try_fec_group_recover(
                    g,
                    &mut fec_groups,
                    &manifest,
                    rs,
                    &mut out,
                    &mut received_chunks,
                    self.config.verify_checksums,
                )
                .await?;
                continue;
            }

            if received_chunks.contains(&desc.index) {
                // Already persisted in partial state; still consume payload bytes from stream.
                let mut discard = vec![0_u8; desc.compressed_size as usize];
                stream.read_exact(&mut discard).await?;
                continue;
            }
            let mut payload = vec![0_u8; desc.compressed_size as usize];
            stream.read_exact(&mut payload).await?;
            if simulate_lost.contains(&desc.index) {
                continue;
            }
            if self.config.verify_checksums {
                let got = *blake3::hash(&payload).as_bytes();
                if got != desc.checksum {
                    return Err(anyhow::anyhow!("chunk checksum mismatch at {}", desc.index));
                }
            }
            let chunk = if desc.was_compressed {
                maybe_decompress(&payload, &manifest.compression)?
            } else {
                payload.clone()
            };
            out.seek(SeekFrom::Start(desc.offset)).await?;
            out.write_all(&chunk).await?;
            received_chunks.insert(desc.index);

            if let Some(ref rs) = fec_rs {
                let ds = manifest.data_shards.max(1);
                let g = desc.fec_group;
                let base = g.saturating_mul(ds as u64);
                let slot = desc.index.saturating_sub(base) as usize;
                if slot < manifest.data_shards {
                    let mut framed = Vec::with_capacity(4 + desc_buf.len() + payload.len());
                    framed.extend_from_slice(&len_buf);
                    framed.extend_from_slice(&desc_buf);
                    framed.extend_from_slice(&payload);
                    let group = fec_groups.entry(g).or_insert_with(|| {
                        vec![None; manifest.data_shards + manifest.parity_shards]
                    });
                    if slot < group.len() {
                        group[slot] = Some(framed);
                    }
                    try_fec_group_recover(
                        g,
                        &mut fec_groups,
                        &manifest,
                        rs,
                        &mut out,
                        &mut received_chunks,
                        self.config.verify_checksums,
                    )
                    .await?;
                }
            }

            if let Some(ref mut fb_send) = feedback_stream {
                if !received_chunks.is_empty()
                    && (received_chunks.len() as u64).is_multiple_of(feedback_every)
                {
                    // Berechne fehlende Chunks: expected minus already received.
                    // Cap auf 64 Einträge, kleinste Indizes zuerst (Sender priorisiert
                    // frühe Chunks — sie blockieren oft spätere durch sequentiellen Schreibzwang).
                    let mut missing: Vec<u64> = (0..manifest.num_chunks)
                        .filter(|i| !received_chunks.contains(i))
                        .take(64)
                        .collect();
                    missing.sort_unstable();

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
                        missing_chunk_indices: missing,
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

async fn persist_framed_chunk(
    wire: &[u8],
    manifest: &TransferManifest,
    out: &mut tokio::fs::File,
    received_chunks: &mut HashSet<u64>,
    verify_checksums: bool,
) -> Result<()> {
    if wire.len() < 4 {
        return Err(anyhow::anyhow!("truncated chunk stream"));
    }
    let desc_len = u32::from_be_bytes(wire[0..4].try_into().unwrap()) as usize;
    if wire.len() < 4 + desc_len {
        return Err(anyhow::anyhow!("truncated chunk descriptor"));
    }
    let desc: ChunkDescriptor = decode(&wire[4..4 + desc_len])?;
    let payload_off = 4 + desc_len;
    let payload_end = payload_off + desc.compressed_size as usize;
    if payload_end > wire.len() {
        return Err(anyhow::anyhow!("truncated chunk body"));
    }
    let wire_payload = &wire[payload_off..payload_end];
    if verify_checksums {
        let got = *blake3::hash(wire_payload).as_bytes();
        if got != desc.checksum {
            return Err(anyhow::anyhow!("chunk checksum mismatch at {}", desc.index));
        }
    }
    let chunk = if desc.was_compressed {
        maybe_decompress(wire_payload, &manifest.compression)?
    } else {
        wire_payload.to_vec()
    };
    if !received_chunks.contains(&desc.index) {
        out.seek(SeekFrom::Start(desc.offset)).await?;
        out.write_all(&chunk).await?;
        received_chunks.insert(desc.index);
    }
    Ok(())
}

/// Data-slot indices in `fec_group` that are absent on the wire and not yet persisted.
fn fec_missing_data_slots(
    group: &[Option<Vec<u8>>],
    manifest: &TransferManifest,
    fec_group: u64,
    received_chunks: &HashSet<u64>,
) -> Vec<usize> {
    let ds = manifest.data_shards.max(1) as u64;
    (0..manifest.data_shards)
        .filter(|&slot| {
            let chunk_idx = fec_group.saturating_mul(ds).saturating_add(slot as u64);
            !received_chunks.contains(&chunk_idx)
                && group.get(slot).and_then(|s| s.as_ref()).is_none()
        })
        .collect()
}

/// True when RS can run: at least one data shard and `data_shards` total shards on the wire.
fn fec_recovery_ready(group: &[Option<Vec<u8>>], manifest: &TransferManifest) -> bool {
    let total = manifest.data_shards + manifest.parity_shards;
    if group.len() != total {
        return false;
    }
    let data_present = (0..manifest.data_shards)
        .filter(|&i| group.get(i).and_then(|s| s.as_ref()).is_some())
        .count();
    if data_present == 0 {
        return false;
    }
    group.iter().filter(|s| s.is_some()).count() >= manifest.data_shards
}

fn peek_framed_chunk_index(wire: &[u8]) -> Result<u64> {
    if wire.len() < 4 {
        return Err(anyhow::anyhow!("truncated chunk stream"));
    }
    let desc_len = u32::from_be_bytes(wire[0..4].try_into().unwrap()) as usize;
    if wire.len() < 4 + desc_len {
        return Err(anyhow::anyhow!("truncated chunk descriptor"));
    }
    let desc: ChunkDescriptor = decode(&wire[4..4 + desc_len])?;
    Ok(desc.index)
}

async fn try_fec_group_recover(
    fec_group: u64,
    fec_groups: &mut HashMap<u64, Vec<Option<Vec<u8>>>>,
    manifest: &TransferManifest,
    rs: &ReedSolomon,
    out: &mut tokio::fs::File,
    received_chunks: &mut HashSet<u64>,
    verify_checksums: bool,
) -> Result<()> {
    let Some(group) = fec_groups.get(&fec_group) else {
        return Ok(());
    };
    let missing = fec_missing_data_slots(group, manifest, fec_group, received_chunks);
    if missing.is_empty() || !fec_recovery_ready(group, manifest) {
        return Ok(());
    }
    let mut max_len = 0usize;
    for s in group.iter().flatten() {
        max_len = max_len.max(s.len());
    }
    if max_len == 0 {
        return Ok(());
    }
    let mut shards: Vec<Option<Vec<u8>>> = group
        .iter()
        .map(|opt| {
            opt.as_ref().map(|v| {
                let mut x = v.clone();
                x.resize(max_len, 0);
                x
            })
        })
        .collect();
    if rs.reconstruct_data(&mut shards).is_err() {
        return Ok(());
    }
    let ds = manifest.data_shards.max(1) as u64;
    let mut recovered_any = false;
    for slot in missing {
        let chunk_idx = fec_group.saturating_mul(ds).saturating_add(slot as u64);
        let Some(bytes) = shards.get(slot).and_then(|s| s.as_ref()) else {
            continue;
        };
        let Ok(recovered_index) = peek_framed_chunk_index(bytes) else {
            continue;
        };
        if recovered_index != chunk_idx {
            continue;
        }
        persist_framed_chunk(bytes, manifest, out, received_chunks, verify_checksums).await?;
        recovered_any = true;
    }
    if recovered_any {
        let still_missing = fec_missing_data_slots(group, manifest, fec_group, received_chunks);
        if still_missing.is_empty() {
            fec_groups.remove(&fec_group);
        }
    }
    Ok(())
}

fn hex_transfer_id(id: [u8; 16]) -> String {
    id.iter().map(|b| format!("{b:02x}")).collect::<String>()
}

#[cfg(all(test, feature = "test-hooks"))]
mod test_hooks_tests {
    use super::*;

    #[test]
    fn parses_simulated_lost_indices_from_env() {
        std::env::set_var(TEST_SIMULATE_LOST_CHUNK_INDICES_ENV, "1, 3,5");
        let got = simulated_lost_chunks_from_env();
        std::env::remove_var(TEST_SIMULATE_LOST_CHUNK_INDICES_ENV);
        assert_eq!(got, HashSet::from([1, 3, 5]));
    }
}

#[cfg(test)]
mod fec_recovery_plan_tests {
    use super::*;

    #[test]
    fn recovery_not_ready_without_enough_total_shards() {
        let group = vec![Some(vec![1]), None, None, None, Some(vec![2]), None];
        let manifest = TransferManifest {
            transfer_id: [0; 16],
            filename: "t".into(),
            total_size: 0,
            chunk_size: 1,
            num_chunks: 4,
            checksum_algorithm: sct_proto::ChecksumAlg::Blake3,
            file_checksum: [0; 32],
            compression: sct_proto::CompressionType::None,
            metadata: Default::default(),
            data_shards: 4,
            parity_shards: 2,
        };
        assert!(!fec_recovery_ready(&group, &manifest));
    }

    #[test]
    fn recovery_ready_with_one_data_and_one_parity() {
        let group = vec![Some(vec![1]), None, Some(vec![2]), None];
        let manifest = TransferManifest {
            transfer_id: [0; 16],
            filename: "t".into(),
            total_size: 0,
            chunk_size: 1,
            num_chunks: 2,
            checksum_algorithm: sct_proto::ChecksumAlg::Blake3,
            file_checksum: [0; 32],
            compression: sct_proto::CompressionType::None,
            metadata: Default::default(),
            data_shards: 2,
            parity_shards: 2,
        };
        assert!(fec_recovery_ready(&group, &manifest));
    }

    #[test]
    fn recovery_not_ready_with_parity_only() {
        let group = vec![None, None, Some(vec![1]), Some(vec![2])];
        let manifest = TransferManifest {
            transfer_id: [0; 16],
            filename: "t".into(),
            total_size: 0,
            chunk_size: 1,
            num_chunks: 2,
            checksum_algorithm: sct_proto::ChecksumAlg::Blake3,
            file_checksum: [0; 32],
            compression: sct_proto::CompressionType::None,
            metadata: Default::default(),
            data_shards: 2,
            parity_shards: 2,
        };
        assert!(!fec_recovery_ready(&group, &manifest));
    }

    #[test]
    fn recovery_ready_when_three_data_and_two_parity() {
        let group = vec![
            Some(vec![1]),
            None,
            Some(vec![2]),
            Some(vec![3]),
            Some(vec![4]),
            Some(vec![5]),
        ];
        let manifest = TransferManifest {
            transfer_id: [0; 16],
            filename: "t".into(),
            total_size: 0,
            chunk_size: 1,
            num_chunks: 4,
            checksum_algorithm: sct_proto::ChecksumAlg::Blake3,
            file_checksum: [0; 32],
            compression: sct_proto::CompressionType::None,
            metadata: Default::default(),
            data_shards: 4,
            parity_shards: 2,
        };
        let missing = fec_missing_data_slots(&group, &manifest, 0, &HashSet::new());
        assert_eq!(missing, vec![1]);
        assert!(fec_recovery_ready(&group, &manifest));
    }
}
