use crate::compression::maybe_decompress;
use crate::protocol::{
    check_frame_len, decode, read_framed, write_framed, ChunkDescriptor, FinalAck, ManifestAck,
    ReceiverFeedbackFrame, TransferComplete, TransferManifest, MAX_CHUNK_DESCRIPTOR_BYTES,
};
use crate::sender::hash_file_streaming;
use crate::transport::SctEndpoint;
use anyhow::Result;
use reed_solomon_erasure::galois_8::ReedSolomon;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use tokio::fs::{self, OpenOptions};
use tokio::io::SeekFrom;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio::task::JoinSet;

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
            hash_existing_chunks(&temp_path, &manifest).await
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
        // Parity: each FEC group may emit `parity_shards` extra uni streams (`fec_groups * parity`).
        //
        // `accepted_streams` counts every `accept_data_stream` iteration from connection start.
        // It must NOT shrink as `received_chunks` grows — otherwise duplicate / parity traffic
        // near the tail trips a false "incomplete transfer" while indices are still filling.
        let fec_group_count = total_chunks.div_ceil(manifest.data_shards.max(1));
        let expected_parity_streams = fec_group_count.saturating_mul(manifest.parity_shards);
        const EXTRA_STREAM_HEADROOM: usize = 256; // duplicates / speculative re-sends
        let max_streams = total_chunks
            .saturating_add(expected_parity_streams)
            .saturating_add(EXTRA_STREAM_HEADROOM);
        let mut accepted_streams = 0usize;
        let max_parallel = self.config.max_parallel_chunks.max(1);
        let mut inflight: JoinSet<Result<Vec<u8>>> = JoinSet::new();
        while received_chunks.len() < total_chunks {
            if accepted_streams >= max_streams && inflight.is_empty() {
                return Err(anyhow::anyhow!(
                    "incomplete transfer: {} of {} chunk indices after {} data streams",
                    received_chunks.len(),
                    total_chunks,
                    accepted_streams
                ));
            }
            let wire = tokio::select! {
                biased;
                Some(joined) = inflight.join_next(), if !inflight.is_empty() => {
                    joined??
                }
                d = conn.read_datagram() => {
                    d?.to_vec()
                }
                s = conn.accept_data_stream(),
                    if inflight.len() < max_parallel && accepted_streams < max_streams =>
                {
                    let mut stream = s?;
                    accepted_streams += 1;
                    inflight.spawn(async move {
                        stream
                            .read_to_end(32 * 1024 * 1024)
                            .await
                            .map_err(|e| anyhow::anyhow!("uni stream read: {e}"))
                    });
                    continue;
                }
            };
            let fec_group = ingest_wire_frame(
                &wire,
                &manifest,
                &mut out,
                &mut received_chunks,
                &mut fec_groups,
                &fec_rs,
                self.config.verify_checksums,
                &simulate_lost,
            )
            .await?;
            maybe_write_feedback(
                &mut feedback_stream,
                &conn,
                &manifest,
                &received_chunks,
                &fec_groups,
                accepted_streams,
                feedback_every,
                fec_group,
            )
            .await;
            if self.config.resume_partial {
                let n = received_chunks.len();
                if n == total_chunks || n <= 32 || n.is_multiple_of(32) {
                    let state = ResumeState {
                        received_chunks: received_chunks.iter().copied().collect(),
                    };
                    let raw = serde_json::to_vec(&state)?;
                    fs::write(&state_path, raw).await?;
                }
            }
        }

        out.flush().await?;
        drop(out);
        let full_hash = hash_file_streaming(&temp_path).await?;
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

fn parse_framed_header(wire: &[u8]) -> Result<(ChunkDescriptor, usize, usize)> {
    if wire.len() < 4 {
        return Err(anyhow::anyhow!("truncated chunk stream"));
    }
    let desc_len_bytes: [u8; 4] = wire[0..4]
        .try_into()
        .map_err(|_| anyhow::anyhow!("truncated length prefix"))?;
    let desc_len = u32::from_be_bytes(desc_len_bytes) as usize;
    check_frame_len(desc_len, MAX_CHUNK_DESCRIPTOR_BYTES)?;
    if wire.len() < 4 + desc_len {
        return Err(anyhow::anyhow!("truncated chunk descriptor"));
    }
    let desc: ChunkDescriptor = decode(&wire[4..4 + desc_len])?;
    Ok((desc, 4, desc_len))
}

#[allow(clippy::too_many_arguments)]
async fn ingest_wire_frame(
    wire: &[u8],
    manifest: &TransferManifest,
    out: &mut tokio::fs::File,
    received_chunks: &mut HashSet<u64>,
    fec_groups: &mut HashMap<u64, Vec<Option<Vec<u8>>>>,
    fec_rs: &Option<ReedSolomon>,
    verify_checksums: bool,
    simulate_lost: &HashSet<u64>,
) -> Result<u64> {
    let (desc, _prefix, _desc_len) = parse_framed_header(wire)?;
    let max_body = (manifest.chunk_size as usize)
        .saturating_mul(4)
        .max(64 * 1024);
    if desc.compressed_size as usize > max_body {
        return Err(anyhow::anyhow!(
            "payload too large at index {} (parity={})",
            desc.index,
            desc.is_parity
        ));
    }
    let payload_off = 4 + _desc_len;
    let payload_end = payload_off + desc.compressed_size as usize;
    if payload_end > wire.len() {
        return Err(anyhow::anyhow!("truncated chunk body"));
    }
    let payload = &wire[payload_off..payload_end];

    if desc.is_parity {
        let Some(rs) = fec_rs.as_ref() else {
            return Ok(desc.fec_group);
        };
        if verify_checksums {
            let got = *blake3::hash(payload).as_bytes();
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
            group[pi] = Some(payload.to_vec());
        }
        try_fec_group_recover(
            g,
            fec_groups,
            manifest,
            rs,
            out,
            received_chunks,
            verify_checksums,
        )
        .await?;
        return Ok(g);
    }

    if simulate_lost.contains(&desc.index) {
        return Ok(desc.fec_group);
    }
    persist_framed_chunk(wire, manifest, out, received_chunks, verify_checksums).await?;

    if let Some(rs) = fec_rs.as_ref() {
        let ds = manifest.data_shards.max(1);
        let g = desc.fec_group;
        let base = g.saturating_mul(ds as u64);
        let slot = desc.index.saturating_sub(base) as usize;
        if slot < manifest.data_shards {
            let group = fec_groups
                .entry(g)
                .or_insert_with(|| vec![None; manifest.data_shards + manifest.parity_shards]);
            if slot < group.len() {
                group[slot] = Some(wire.to_vec());
            }
            try_fec_group_recover(
                g,
                fec_groups,
                manifest,
                rs,
                out,
                received_chunks,
                verify_checksums,
            )
            .await?;
        }
    }
    Ok(desc.fec_group)
}

#[allow(clippy::too_many_arguments)]
async fn maybe_write_feedback(
    feedback_stream: &mut Option<quinn::SendStream>,
    conn: &crate::transport::SctConnection,
    manifest: &TransferManifest,
    received_chunks: &HashSet<u64>,
    fec_groups: &HashMap<u64, Vec<Option<Vec<u8>>>>,
    accepted_streams: usize,
    feedback_every: u64,
    fec_group: u64,
) {
    let Some(fb_send) = feedback_stream.as_mut() else {
        return;
    };
    if received_chunks.is_empty() || !(received_chunks.len() as u64).is_multiple_of(feedback_every)
    {
        return;
    }
    let mut missing: Vec<u64> = (0..manifest.num_chunks)
        .filter(|i| !received_chunks.contains(i))
        .take(64)
        .collect();
    missing.sort_unstable();
    let loss_hint = if accepted_streams == 0 {
        0.0
    } else {
        (accepted_streams.saturating_sub(received_chunks.len()) as f32 / accepted_streams as f32)
            .clamp(0.0, 1.0)
    };
    let reconstructable = fec_groups
        .get(&fec_group)
        .map(|g| fec_recovery_ready(g, manifest))
        .unwrap_or(false);
    let frame = ReceiverFeedbackFrame {
        transfer_id: manifest.transfer_id,
        decode_delay_ms: if manifest.chunk_size > (1024 * 1024) {
            20
        } else {
            8
        },
        buffer_occupancy: ((received_chunks.len() as f32) / (manifest.num_chunks.max(1) as f32))
            .clamp(0.0, 1.0),
        cpu_load: 0.45,
        loss_hint,
        rtt_ms: conn.rtt().as_millis().min(u128::from(u32::MAX)) as u32,
        completed_block_id: Some(fec_group),
        block_reconstructable: reconstructable,
        missing_chunk_indices: missing,
    };
    let _ = write_framed(fb_send, &frame).await;
}

async fn hash_existing_chunks(path: &Path, manifest: &TransferManifest) -> Vec<[u8; 32]> {
    let mut hashes = vec![[0u8; 32]; manifest.num_chunks as usize];
    let Ok(mut file) = tokio::fs::File::open(path).await else {
        return hashes;
    };
    let file_len = file.metadata().await.map(|m| m.len()).unwrap_or(0);
    for i in 0..manifest.num_chunks {
        let off = i * u64::from(manifest.chunk_size);
        let end = (off + u64::from(manifest.chunk_size))
            .min(file_len)
            .min(manifest.total_size);
        if end <= off {
            continue;
        }
        if file.seek(SeekFrom::Start(off)).await.is_err() {
            continue;
        }
        let mut buf = vec![0u8; (end - off) as usize];
        if file.read_exact(&mut buf).await.is_ok() {
            hashes[i as usize] = *blake3::hash(&buf).as_bytes();
        }
    }
    hashes
}

async fn persist_framed_chunk(
    wire: &[u8],
    manifest: &TransferManifest,
    out: &mut tokio::fs::File,
    received_chunks: &mut HashSet<u64>,
    verify_checksums: bool,
) -> Result<()> {
    let (desc, prefix, desc_len) = parse_framed_header(wire)?;
    if desc.is_parity {
        return Ok(());
    }
    let payload_off = prefix + desc_len;
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
    if received_chunks.contains(&desc.index) {
        return Ok(());
    }
    out.seek(SeekFrom::Start(desc.offset)).await?;
    if desc.was_compressed {
        let chunk = maybe_decompress(wire_payload, &manifest.compression)?;
        out.write_all(&chunk).await?;
    } else {
        out.write_all(wire_payload).await?;
    }
    received_chunks.insert(desc.index);
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
    let (desc, _, _) = parse_framed_header(wire)?;
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
