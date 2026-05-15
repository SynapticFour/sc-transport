use crate::adaptive::{
    compute_chunk_size, compute_fec_ratio, AutopilotRuntime, FecEncoder, MultiPathScheduler,
    OptimizationKpi, Packet, PacketId, PacketMeta, PathCorrelation, PathKind, PredictiveStabilizer,
    QuicDatagramPath, QuicStreamPath, ReceiverFeedback, StrategyEngine, TransferMetrics,
    TransferMode,
};
use crate::compression::maybe_compress;
use crate::metrics::SctMetrics;
use crate::protocol::{
    encode, read_framed, write_framed, ChunkDescriptor, CompressionType, FinalAck, ManifestAck,
    ReceiverFeedbackFrame, TransferComplete, TransferManifest,
};
use crate::transport::SctConnection;
use anyhow::Result;
use std::collections::HashMap;
use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::time::{Duration, Instant};
use tokio::fs::File;
use tokio::io::AsyncReadExt;
use tokio::sync::{mpsc, Mutex};

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
    pub prometheus: Option<Arc<SctMetrics>>,
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
            prometheus: None,
        }
    }
}

impl FileSender {
    pub fn new(connection: SctConnection, config: SenderConfig) -> Self {
        Self { connection, config }
    }

    /// RS block width for manifest + [`AutopilotRuntime::fec`]. Must stay in sync with `send_adaptive`.
    fn plan_fec(&self, rtt: Duration, loss_hint: f64) -> FecEncoder {
        let data_shards = if compute_chunk_size(rtt, loss_hint) <= 128 * 1024 {
            2
        } else {
            4
        };
        let (_d, p_base) = compute_fec_ratio(loss_hint, 0.0);
        let parity_shards = if data_shards == 0 {
            0
        } else {
            (p_base + 1).min(data_shards / 2).max(p_base).max(1)
        };
        FecEncoder {
            data_shards,
            parity_shards,
        }
    }

    pub async fn send(&self, path: &Path) -> Result<()> {
        let wall_start = Instant::now();
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
        let rtt = self.connection.rtt();
        let loss_hint = std::env::var("SC_SCT_ADAPTIVE_LOSS_HINT")
            .ok()
            .and_then(|v| v.parse::<f64>().ok())
            .map(|v| v.clamp(0.0, 1.0))
            .unwrap_or(0.01);
        let fec = self.plan_fec(rtt, loss_hint);
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
            data_shards: fec.data_shards,
            parity_shards: fec.parity_shards,
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
        let skip: HashSet<u64> = ack.received_chunks.iter().copied().collect();

        // Delta-Skip: Chunks die der Receiver bereits mit identischem Hash hat überspringen.
        let skip: HashSet<u64> = if !ack.chunk_hashes.is_empty() {
            let mut delta_skip = skip;
            for idx in 0u64..manifest.num_chunks {
                if delta_skip.contains(&idx) {
                    continue;
                }
                if let Some(&receiver_hash) = ack.chunk_hashes.get(idx as usize) {
                    if receiver_hash == [0u8; 32] {
                        continue;
                    }
                    let off = idx as usize * self.config.chunk_size;
                    let end = (off + self.config.chunk_size).min(full.len());
                    if end > off {
                        let local_hash = *blake3::hash(&full[off..end]).as_bytes();
                        if local_hash == receiver_hash {
                            delta_skip.insert(idx);
                        }
                    }
                }
            }
            delta_skip
        } else {
            skip
        };

        let (xfer_metrics, nack_retransmits, loss_rate, fec_estimate) = self
            .send_adaptive(&full, &manifest, &skip, total_size)
            .await?;
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
        if let Some(ref m) = self.config.prometheus {
            m.transfers_completed.inc();
            m.transfer_duration_seconds
                .observe(wall_start.elapsed().as_secs_f64());
            m.transfer_bytes_total.inc_by(total_size);
            m.transfer_p99_ms
                .observe(xfer_metrics.p99_completion.as_secs_f64() * 1000.0);
            m.transfer_loss_rate.observe(loss_rate);
            m.nack_retransmits_total.inc_by(nack_retransmits);
            m.fec_encoded_blocks_total.inc_by(fec_estimate);
        }
        Ok(())
    }

    async fn send_adaptive(
        &self,
        full: &[u8],
        manifest: &TransferManifest,
        skip: &HashSet<u64>,
        total_size: u64,
    ) -> Result<(TransferMetrics, u64, f64, u64)> {
        let (stream_tx, mut stream_rx) = mpsc::unbounded_channel::<Packet>();
        let (dgram_tx, mut dgram_rx) = mpsc::unbounded_channel::<Packet>();
        let stream_conn = self.connection.clone();
        let dgram_conn = self.connection.clone();
        let feedback_state = Arc::new(Mutex::new(None::<ReceiverFeedbackFrame>));
        let feedback_listener_conn = self.connection.clone();
        let feedback_state_bg = feedback_state.clone();
        let feedback_listener = tokio::spawn(async move {
            if let Ok((_send, mut recv)) = feedback_listener_conn.accept_control_stream().await {
                while let Ok(frame) = read_framed::<ReceiverFeedbackFrame, _>(&mut recv).await {
                    let mut guard = feedback_state_bg.lock().await;
                    *guard = Some(frame);
                }
            }
        });

        let stream_task = tokio::spawn(async move {
            while let Some(pkt) = stream_rx.recv().await {
                let _ = write_packet_payload(&stream_conn, pkt.payload).await;
            }
        });
        let dgram_task = tokio::spawn(async move {
            while let Some(pkt) = dgram_rx.recv().await {
                let _ = write_packet_payload(&dgram_conn, pkt.payload).await;
            }
        });

        let mut scheduler = MultiPathScheduler {
            paths: Vec::new(),
            speculative_ratio: 0.0, // startet bei 0; wird durch on_feedback_tick hochgeregelt
            // Receiver deduplicates by chunk index; budget caps concurrent speculative copies.
            duplicate_budget: 4,
            in_flight_duplicates: 0,
            known_reconstructable: Default::default(),
            // Full bucket: first distribute_and_send can happen in the same tick as construction,
            // where elapsed≈0 would otherwise starve small packets (≤2 MTU) and hang QUIC tests.
            tokens: 2.0 * 1500.0,
            last_token_refill: Instant::now() - Duration::from_millis(50),
            last_primary_utility: 0.12,
            queue_models: Vec::new(),
            path_correlation: PathCorrelation::from_path_kinds(&[
                PathKind::Stream,
                PathKind::Datagram,
            ]),
            optimization_kpi: OptimizationKpi::default(),
            exploration_seed: 0xC0FFEEu64,
        };
        // Beide Pfade sind immer aktiv. speculative_ratio bestimmt wie viele
        // Pakete dupliziert werden — startet bei 0.0 und wächst dynamisch
        // über estimate_unused_bandwidth / on_feedback_tick.
        scheduler
            .paths
            .push(Box::new(QuicStreamPath::new(stream_tx)));
        scheduler
            .paths
            .push(Box::new(QuicDatagramPath::new(dgram_tx)));

        let mut runtime = AutopilotRuntime {
            strategy: StrategyEngine::default(),
            cc: Default::default(),
            scheduler,
            fec: FecEncoder {
                data_shards: manifest.data_shards.max(1),
                parity_shards: manifest.parity_shards,
            },
            metrics: TransferMetrics::default(),
            stabilizer: PredictiveStabilizer::default(),
            completed_blocks: Arc::new(StdMutex::new(HashSet::new())),
            block_data_shards_sent: Arc::new(StdMutex::new(HashMap::new())),
            completion_first_enabled: true,
        };
        let fec_gap = (0..manifest.data_shards as u64).any(|i| skip.contains(&i));
        let parity_cap = if fec_gap { 0 } else { manifest.parity_shards };
        let rtt = self.connection.rtt();
        let prev_rtt = rtt;
        let cwnd = self.connection.congestion_window().max(1200);
        let bw_estimate_bps = (cwnd as f64 / rtt.as_secs_f64().max(0.001)) * 8.0;
        let loss_hint = std::env::var("SC_SCT_ADAPTIVE_LOSS_HINT")
            .ok()
            .and_then(|v| v.parse::<f64>().ok())
            .map(|v| v.clamp(0.0, 1.0))
            .unwrap_or(0.01);
        runtime
            .cc
            .on_network_sample(bw_estimate_bps, rtt, prev_rtt, loss_hint);
        runtime.cc.rtt_variance_trend = runtime.cc.rtt_variance;
        let recv_feedback = ReceiverFeedback {
            decode_delay: Duration::from_millis(if rtt > Duration::from_millis(80) {
                45
            } else {
                10
            }),
            buffer_occupancy: if cwnd < (1 << 20) { 0.75 } else { 0.35 },
            cpu_load: 0.5,
        };
        runtime.strategy.update(rtt, loss_hint, 0.2, &recv_feedback);
        // Keep RS dimensions aligned with the manifest the receiver already acknowledged.
        runtime.fec.data_shards = manifest.data_shards.max(1);
        runtime.fec.parity_shards = if parity_cap == 0 {
            0
        } else {
            manifest.parity_shards
        };

        let mut packets = Vec::new();
        let batch_size = std::env::var("SC_SCT_ADAPTIVE_BATCH_SIZE")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(64)
            .max(1);
        let start = Instant::now();
        let mut sent = 0_u64;
        let mut nack_retransmitted: HashSet<u64> = HashSet::new();
        for idx in 0..manifest.num_chunks {
            if skip.contains(&idx) {
                continue;
            }
            let payload = self.build_chunk_payload(full, idx, manifest)?;
            sent += self.chunk_len(full, idx) as u64;
            self.emit_progress(sent, total_size, start);
            packets.push(Packet {
                id: PacketId(idx),
                seq: idx,
                payload,
                is_parity: false,
                meta: PacketMeta {
                    id: idx,
                    priority: if idx < 2 { 240 } else { 100 },
                    deadline: Some(Instant::now() + Duration::from_millis(75 + (idx % 5) * 10)),
                    size: self.chunk_len(full, idx),
                },
                fec_group: idx / manifest.data_shards.max(1) as u64,
                reconstructable: false,
                parity_index: 0,
            });
            if packets.len() >= batch_size {
                apply_feedback_if_present(&mut runtime, &feedback_state, rtt, manifest, parity_cap)
                    .await;
                // NACK-Retransmit: fehlende Chunks die der Receiver gemeldet hat nachsenden.
                {
                    let maybe_frame = feedback_state.lock().await.clone();
                    if let Some(ref frame) = maybe_frame {
                        let data_shards = runtime.fec.data_shards.max(1) as u64;
                        for &missing_idx in frame.missing_chunk_indices.iter().take(8) {
                            if skip.contains(&missing_idx) {
                                continue;
                            }
                            if nack_retransmitted.contains(&missing_idx) {
                                continue;
                            }
                            nack_retransmitted.insert(missing_idx);
                            if let Ok(payload) =
                                self.build_chunk_payload(full, missing_idx, manifest)
                            {
                                packets.push(Packet {
                                    id: PacketId(missing_idx),
                                    seq: missing_idx,
                                    payload,
                                    is_parity: false,
                                    meta: PacketMeta {
                                        id: missing_idx,
                                        priority: 255,
                                        deadline: Some(Instant::now() + Duration::from_millis(50)),
                                        size: self.chunk_len(full, missing_idx),
                                    },
                                    fec_group: missing_idx / data_shards,
                                    reconstructable: false,
                                    parity_index: 0,
                                });
                            }
                        }
                    }
                }
                runtime
                    .run_pipeline(std::mem::take(&mut packets), parity_cap)
                    .await;
            }
        }
        if !packets.is_empty() {
            apply_feedback_if_present(&mut runtime, &feedback_state, rtt, manifest, parity_cap)
                .await;
            // NACK-Retransmit: fehlende Chunks die der Receiver gemeldet hat nachsenden.
            {
                let maybe_frame = feedback_state.lock().await.clone();
                if let Some(ref frame) = maybe_frame {
                    let data_shards = runtime.fec.data_shards.max(1) as u64;
                    for &missing_idx in frame.missing_chunk_indices.iter().take(8) {
                        if skip.contains(&missing_idx) {
                            continue;
                        }
                        if nack_retransmitted.contains(&missing_idx) {
                            continue;
                        }
                        nack_retransmitted.insert(missing_idx);
                        if let Ok(payload) = self.build_chunk_payload(full, missing_idx, manifest) {
                            packets.push(Packet {
                                id: PacketId(missing_idx),
                                seq: missing_idx,
                                payload,
                                is_parity: false,
                                meta: PacketMeta {
                                    id: missing_idx,
                                    priority: 255,
                                    deadline: Some(Instant::now() + Duration::from_millis(50)),
                                    size: self.chunk_len(full, missing_idx),
                                },
                                fec_group: missing_idx / data_shards,
                                reconstructable: false,
                                parity_index: 0,
                            });
                        }
                    }
                }
            }
            runtime.run_pipeline(packets, parity_cap).await;
        }
        let xfer_metrics = runtime.metrics.clone();
        let nack_retransmits = nack_retransmitted.len() as u64;
        let loss_rate = runtime.cc.loss_rate;
        let ds = runtime.fec.data_shards.max(1) as u64;
        let to_send = manifest.num_chunks.saturating_sub(skip.len() as u64);
        let fec_estimate = to_send
            .saturating_add(ds.saturating_sub(1))
            .saturating_div(ds.max(1));
        drop(runtime);
        let _ = stream_task.await;
        let _ = dgram_task.await;
        feedback_listener.abort();
        Ok((xfer_metrics, nack_retransmits, loss_rate, fec_estimate))
    }

    fn build_chunk_payload(
        &self,
        full: &[u8],
        idx: u64,
        manifest: &TransferManifest,
    ) -> Result<Vec<u8>> {
        let off = idx as usize * self.config.chunk_size;
        let end = usize::min(off + self.config.chunk_size, full.len());
        self.build_chunk_payload_at(full, idx, off, end, manifest)
    }

    fn build_chunk_payload_at(
        &self,
        full: &[u8],
        idx: u64,
        off: usize,
        end: usize,
        manifest: &TransferManifest,
    ) -> Result<Vec<u8>> {
        let chunk_raw = &full[off..end];
        let chunk = maybe_compress(chunk_raw, &self.config.compression)?;
        let was_compressed = chunk.len() < chunk_raw.len();
        let ds = manifest.data_shards.max(1) as u64;
        let desc = ChunkDescriptor {
            index: idx,
            offset: off as u64,
            compressed_size: chunk.len() as u32,
            uncompressed_size: chunk_raw.len() as u32,
            checksum: *blake3::hash(&chunk).as_bytes(),
            was_compressed,
            is_parity: false,
            parity_index: 0,
            fec_group: idx / ds,
        };
        let desc_bytes = encode(&desc)?;
        let mut payload = Vec::with_capacity(4 + desc_bytes.len() + chunk.len());
        payload.extend_from_slice(&(desc_bytes.len() as u32).to_be_bytes());
        payload.extend_from_slice(&desc_bytes);
        payload.extend_from_slice(&chunk);
        Ok(payload)
    }

    fn chunk_len(&self, full: &[u8], idx: u64) -> usize {
        let off = idx as usize * self.config.chunk_size;
        let end = usize::min(off + self.config.chunk_size, full.len());
        end.saturating_sub(off)
    }

    fn emit_progress(&self, sent: u64, total_size: u64, start: Instant) {
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
}

async fn write_packet_payload(connection: &SctConnection, payload: Vec<u8>) -> Result<()> {
    let mut data = connection.open_data_stream().await?;
    data.write_all(&payload).await?;
    data.finish()?;
    Ok(())
}

async fn apply_feedback_if_present(
    runtime: &mut AutopilotRuntime,
    state: &Arc<Mutex<Option<ReceiverFeedbackFrame>>>,
    default_rtt: Duration,
    manifest: &TransferManifest,
    parity_cap: usize,
) {
    let snapshot = { state.lock().await.clone() };
    if let Some(fb) = snapshot {
        let rtt = Duration::from_millis(fb.rtt_ms as u64).max(default_rtt);
        let loss = (fb.loss_hint as f64).clamp(0.0, 1.0);
        let feedback = ReceiverFeedback {
            decode_delay: Duration::from_millis(fb.decode_delay_ms as u64),
            buffer_occupancy: fb.buffer_occupancy as f64,
            cpu_load: fb.cpu_load as f64,
        };
        runtime
            .cc
            .on_network_sample(runtime.cc.bandwidth_estimate, rtt, runtime.cc.min_rtt, loss);
        runtime.cc.rtt_variance_trend = runtime.cc.rtt_variance;
        runtime.strategy.update(rtt, loss, 0.2, &feedback);
        if fb.block_reconstructable {
            if let Some(block_id) = fb.completed_block_id {
                if let Ok(mut done) = runtime.completed_blocks.lock() {
                    done.insert(block_id);
                }
                runtime.scheduler.mark_reconstructable(block_id);
            }
        }
        let (data, parity) = compute_fec_ratio(loss, runtime.cc.rtt_variance);
        let _data = data;
        runtime.fec.data_shards = manifest.data_shards.max(1);
        let mut p = match runtime.strategy.mode {
            TransferMode::Aggressive => parity.saturating_add(1),
            TransferMode::Balanced => parity,
            TransferMode::Conservative => parity.saturating_sub(1).max(1),
        };
        if manifest.parity_shards == 0 || parity_cap == 0 {
            runtime.fec.parity_shards = 0;
        } else {
            p = p.min(manifest.parity_shards).min(parity_cap).max(1);
            runtime.fec.parity_shards = p;
        }
        let headroom = runtime.cc.estimate_unused_bandwidth();
        runtime.scheduler.speculative_ratio = if headroom > runtime.cc.bandwidth_estimate * 0.25 {
            0.20
        } else if loss < 0.02 && rtt <= Duration::from_millis(20) {
            0.05
        } else if headroom < runtime.cc.bandwidth_estimate * 0.05 {
            0.08
        } else {
            0.15
        };
    }
}
