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
use bytes::Bytes;
use std::collections::HashMap;
use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::time::{Duration, Instant};
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncSeekExt, SeekFrom};
use tokio::sync::{mpsc, Mutex, Semaphore};
use tokio::task::JoinSet;

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
        let file = File::open(path).await?;
        let meta = file.metadata().await?;
        drop(file);
        let total_size = meta.len();
        let num_chunks = total_size.div_ceil(self.config.chunk_size as u64);
        let filename = path
            .file_name()
            .and_then(|f| f.to_str())
            .unwrap_or("payload.bin")
            .to_string();

        let checksum = hash_file_streaming(path).await?;
        let mut id_hasher = blake3::Hasher::new();
        id_hasher.update(filename.as_bytes());
        id_hasher.update(&total_size.to_le_bytes());
        id_hasher.update(&checksum);
        let mut transfer_id = [0_u8; 16];
        transfer_id.copy_from_slice(&id_hasher.finalize().as_bytes()[..16]);
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

        // Delta-Skip: hash each local chunk from disk (no whole-file buffer).
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
                    let raw = self.read_chunk_range(path, idx, total_size).await?;
                    if !raw.is_empty() && *blake3::hash(&raw).as_bytes() == receiver_hash {
                        delta_skip.insert(idx);
                    }
                }
            }
            delta_skip
        } else {
            skip
        };

        let (xfer_metrics, nack_retransmits, loss_rate, fec_estimate) = self
            .send_adaptive(path, &manifest, &skip, total_size)
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
        path: &Path,
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

        let first_err: Arc<StdMutex<Option<String>>> = Arc::new(StdMutex::new(None));
        let max_par = self.config.max_parallel_chunks.max(1);
        let stream_err = first_err.clone();
        let stream_task = tokio::spawn(async move {
            pump_packets(
                &mut stream_rx,
                stream_conn,
                PathKind::Stream,
                max_par,
                stream_err,
            )
            .await;
        });
        let dgram_err = first_err.clone();
        let dgram_task = tokio::spawn(async move {
            pump_packets(
                &mut dgram_rx,
                dgram_conn,
                PathKind::Datagram,
                max_par,
                dgram_err,
            )
            .await;
        });

        // Receiver deduplicates by chunk index; caps concurrent speculative duplicates on the wire.
        // Default 4 after e2e dedup coverage (`e2e_loopback`, `transfer_smoke` duplicate_chunks).
        let duplicate_budget = std::env::var("SC_SCT_DUPLICATE_BUDGET")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(4)
            .max(1);

        let mut scheduler = MultiPathScheduler {
            paths: Vec::new(),
            speculative_ratio: 0.0, // startet bei 0; wird durch on_feedback_tick hochgeregelt
            duplicate_budget,
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
        let mut prev_rtt = rtt;
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
        runtime
            .scheduler
            .apply_network_sample(rtt, bw_estimate_bps, loss_hint);
        sync_stabilizer_rtt_variance(&mut runtime);
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
            let payload = self
                .build_chunk_payload(path, idx, total_size, manifest)
                .await?;
            let chunk_len = self.chunk_len(idx, total_size);
            sent += chunk_len as u64;
            self.emit_progress(sent, total_size, start);
            packets.push(self.make_data_packet(idx, payload, chunk_len, manifest, 100));
            if packets.len() >= batch_size {
                self.flush_batch(
                    &mut runtime,
                    &mut packets,
                    &feedback_state,
                    path,
                    manifest,
                    skip,
                    total_size,
                    parity_cap,
                    &mut nack_retransmitted,
                    &mut prev_rtt,
                )
                .await?;
                if let Some(e) = first_err.lock().ok().and_then(|g| g.clone()) {
                    return Err(anyhow::anyhow!("data stream send failed: {e}"));
                }
            }
        }
        if !packets.is_empty() {
            self.flush_batch(
                &mut runtime,
                &mut packets,
                &feedback_state,
                path,
                manifest,
                skip,
                total_size,
                parity_cap,
                &mut nack_retransmitted,
                &mut prev_rtt,
            )
            .await?;
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
        if let Some(e) = first_err.lock().ok().and_then(|g| g.clone()) {
            return Err(anyhow::anyhow!("data stream send failed: {e}"));
        }
        Ok((xfer_metrics, nack_retransmits, loss_rate, fec_estimate))
    }

    #[allow(clippy::too_many_arguments)]
    async fn flush_batch(
        &self,
        runtime: &mut AutopilotRuntime,
        packets: &mut Vec<Packet>,
        feedback_state: &Arc<Mutex<Option<ReceiverFeedbackFrame>>>,
        path: &Path,
        manifest: &TransferManifest,
        skip: &HashSet<u64>,
        total_size: u64,
        parity_cap: usize,
        nack_retransmitted: &mut HashSet<u64>,
        prev_rtt: &mut Duration,
    ) -> Result<()> {
        let rtt = self.connection.rtt();
        apply_feedback_if_present(runtime, feedback_state, rtt, manifest, parity_cap).await;
        push_nack_retransmits(
            self,
            path,
            manifest,
            skip,
            total_size,
            feedback_state,
            packets,
            nack_retransmitted,
        )
        .await;
        runtime
            .run_pipeline(std::mem::take(packets), parity_cap)
            .await;
        let sample_rtt = self.connection.rtt();
        let cwnd = self.connection.congestion_window().max(1200);
        let bw = (cwnd as f64 / sample_rtt.as_secs_f64().max(0.001)) * 8.0;
        runtime
            .cc
            .on_network_sample(bw, sample_rtt, *prev_rtt, runtime.cc.loss_rate);
        runtime
            .scheduler
            .apply_network_sample(sample_rtt, bw, runtime.cc.loss_rate);
        *prev_rtt = sample_rtt;
        sync_stabilizer_rtt_variance(runtime);
        Ok(())
    }

    fn make_data_packet(
        &self,
        idx: u64,
        payload: Vec<u8>,
        chunk_len: usize,
        manifest: &TransferManifest,
        priority: u8,
    ) -> Packet {
        let rtt = self.connection.rtt();
        Packet {
            id: PacketId(idx),
            seq: idx,
            payload,
            is_parity: false,
            meta: PacketMeta {
                id: idx,
                priority: if idx < 2 { 240 } else { priority },
                deadline: Some(Instant::now() + rtt + Duration::from_millis(200)),
                size: chunk_len,
            },
            fec_group: idx / manifest.data_shards.max(1) as u64,
            reconstructable: false,
            parity_index: 0,
        }
    }

    async fn build_chunk_payload(
        &self,
        path: &Path,
        idx: u64,
        total_size: u64,
        manifest: &TransferManifest,
    ) -> Result<Vec<u8>> {
        let raw = self.read_chunk_range(path, idx, total_size).await?;
        self.frame_chunk(idx, &raw, manifest)
    }

    fn frame_chunk(
        &self,
        idx: u64,
        chunk_raw: &[u8],
        manifest: &TransferManifest,
    ) -> Result<Vec<u8>> {
        let chunk = maybe_compress(chunk_raw, &self.config.compression)?;
        let was_compressed = chunk.len() < chunk_raw.len();
        let ds = manifest.data_shards.max(1) as u64;
        let off = idx * self.config.chunk_size as u64;
        let desc = ChunkDescriptor {
            index: idx,
            offset: off,
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

    async fn read_chunk_range(&self, path: &Path, idx: u64, total_size: u64) -> Result<Vec<u8>> {
        let off = idx * self.config.chunk_size as u64;
        if off >= total_size {
            return Ok(Vec::new());
        }
        let len = ((total_size - off) as usize).min(self.config.chunk_size);
        let mut file = File::open(path).await?;
        file.seek(SeekFrom::Start(off)).await?;
        let mut buf = vec![0_u8; len];
        file.read_exact(&mut buf).await?;
        Ok(buf)
    }

    fn chunk_len(&self, idx: u64, total_size: u64) -> usize {
        let off = idx * self.config.chunk_size as u64;
        if off >= total_size {
            return 0;
        }
        ((total_size - off) as usize).min(self.config.chunk_size)
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

pub(crate) async fn hash_file_streaming(path: &Path) -> Result<[u8; 32]> {
    let mut file = File::open(path).await?;
    let mut hasher = blake3::Hasher::new();
    let mut buf = vec![0_u8; 1024 * 1024];
    loop {
        let n = file.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(*hasher.finalize().as_bytes())
}

async fn pump_packets(
    rx: &mut mpsc::UnboundedReceiver<Packet>,
    conn: SctConnection,
    kind: PathKind,
    max_parallel: usize,
    first_err: Arc<StdMutex<Option<String>>>,
) {
    let sem = Arc::new(Semaphore::new(max_parallel.max(1)));
    let mut join = JoinSet::new();
    while let Some(pkt) = rx.recv().await {
        let permit = match sem.clone().acquire_owned().await {
            Ok(p) => p,
            Err(_) => break,
        };
        let c = conn.clone();
        let err = first_err.clone();
        join.spawn(async move {
            let _permit = permit;
            if let Err(e) = write_packet_on_path(&c, pkt.payload, kind).await {
                if let Ok(mut g) = err.lock() {
                    if g.is_none() {
                        *g = Some(e.to_string());
                    }
                }
            }
        });
    }
    while join.join_next().await.is_some() {}
}

async fn write_packet_on_path(
    connection: &SctConnection,
    payload: Vec<u8>,
    kind: PathKind,
) -> Result<()> {
    if kind == PathKind::Datagram {
        if let Some(cap) = connection.max_datagram_size() {
            if payload.len() <= cap {
                return connection.send_datagram(Bytes::from(payload));
            }
        }
    }
    let mut data = connection.open_data_stream().await?;
    data.write_all(&payload).await?;
    data.finish()?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn push_nack_retransmits(
    sender: &FileSender,
    path: &Path,
    manifest: &TransferManifest,
    skip: &HashSet<u64>,
    total_size: u64,
    feedback_state: &Arc<Mutex<Option<ReceiverFeedbackFrame>>>,
    packets: &mut Vec<Packet>,
    nack_retransmitted: &mut HashSet<u64>,
) {
    let maybe_frame = feedback_state.lock().await.clone();
    let Some(frame) = maybe_frame else {
        return;
    };
    for &missing_idx in frame.missing_chunk_indices.iter().take(8) {
        if skip.contains(&missing_idx) || nack_retransmitted.contains(&missing_idx) {
            continue;
        }
        nack_retransmitted.insert(missing_idx);
        if let Ok(payload) = sender
            .build_chunk_payload(path, missing_idx, total_size, manifest)
            .await
        {
            let chunk_len = sender.chunk_len(missing_idx, total_size);
            packets.push(sender.make_data_packet(missing_idx, payload, chunk_len, manifest, 255));
        }
    }
}

fn sync_stabilizer_rtt_variance(runtime: &mut AutopilotRuntime) {
    runtime.stabilizer.rtt_variance_trend = runtime.cc.rtt_variance_trend;
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
        runtime.strategy.update(rtt, loss, 0.2, &feedback);
        runtime
            .scheduler
            .apply_network_sample(rtt, runtime.cc.bandwidth_estimate, loss);
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
    sync_stabilizer_rtt_variance(runtime);
}
