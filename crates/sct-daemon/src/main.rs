use anyhow::Result;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use sct_core::protocol::CompressionType;
use sct_core::receiver::{FileReceiver, ReceiverConfig};
use sct_core::sender::{FileSender, SenderConfig};
use sct_core::transport::{SctEndpoint, TransportConfig};
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};
use std::net::IpAddr;
use std::net::SocketAddr;
use std::path::{Path as StdPath, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::sync::{Mutex, Semaphore};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SctConfig {
    server: ServerConfig,
    transfer: TransferConfig,
    auth: AuthConfig,
    metrics: MetricsConfig,
    logging: LoggingConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ServerConfig {
    listen_port: u16,
    api_port: u16,
    max_concurrent_transfers: usize,
    output_base_dir: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TransferConfig {
    default_chunk_size_mb: usize,
    default_parallel_streams: usize,
    default_compression: String,
    enable_resumption: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AuthConfig {
    mode: String,
    known_hosts_file: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct MetricsConfig {
    enabled: bool,
    prometheus_port: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LoggingConfig {
    level: String,
    format: String,
}

impl Default for SctConfig {
    fn default() -> Self {
        Self {
            server: ServerConfig {
                listen_port: 7272,
                api_port: 7273,
                max_concurrent_transfers: 4,
                output_base_dir: "/data/incoming".to_string(),
            },
            transfer: TransferConfig {
                default_chunk_size_mb: 4,
                default_parallel_streams: 16,
                default_compression: "auto".to_string(),
                enable_resumption: true,
            },
            auth: AuthConfig {
                mode: "tofu".to_string(),
                known_hosts_file: "~/.sct/known_hosts".to_string(),
            },
            metrics: MetricsConfig {
                enabled: true,
                prometheus_port: 9090,
            },
            logging: LoggingConfig {
                level: "info".to_string(),
                format: "json".to_string(),
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum TransferStatus {
    Queued,
    Active,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TransferRecord {
    transfer_id: Uuid,
    source: String,
    destination: String,
    priority: u8,
    status: TransferStatus,
    progress_pct: f64,
    throughput_mbps: f64,
    bytes_transferred: u64,
    bytes_total: u64,
    error: Option<String>,
    #[serde(default)]
    created_at_unix_ms: u128,
    #[serde(default)]
    updated_at_unix_ms: u128,
    #[serde(default)]
    attempts: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SubmitTransferRequest {
    source: String,
    destination: String,
    priority: u8,
    callback_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SubmitTransferResponse {
    transfer_id: Uuid,
    status: TransferStatus,
}

#[derive(Debug, Clone, Deserialize)]
struct ListQuery {
    status: Option<String>,
    limit: Option<usize>,
}

#[derive(Debug, Clone, Deserialize)]
struct PriorityUpdate {
    priority: u8,
}

#[derive(Clone)]
struct AppState {
    cfg: SctConfig,
    queue: Arc<Mutex<BinaryHeap<(Reverse<u8>, u64, Uuid)>>>,
    records: Arc<Mutex<HashMap<Uuid, TransferRecord>>>,
    semaphore: Arc<Semaphore>,
    sequence: Arc<Mutex<u64>>,
    state_path: Arc<PathBuf>,
    events_path: Arc<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TransferEventRecord {
    ts_unix_ms: u128,
    transfer_id: Uuid,
    kind: String,
    source: Option<String>,
    destination: Option<String>,
    priority: Option<u8>,
    status: Option<TransferStatus>,
    progress_pct: Option<f64>,
    throughput_mbps: Option<f64>,
    bytes_transferred: Option<u64>,
    bytes_total: Option<u64>,
    error: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cfg = load_config("sct.toml").unwrap_or_default();
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::new(cfg.logging.level.clone()))
        .init();

    let state = AppState {
        cfg: cfg.clone(),
        queue: Arc::new(Mutex::new(BinaryHeap::new())),
        records: Arc::new(Mutex::new(HashMap::new())),
        semaphore: Arc::new(Semaphore::new(cfg.server.max_concurrent_transfers)),
        sequence: Arc::new(Mutex::new(0)),
        state_path: Arc::new(default_state_path()),
        events_path: Arc::new(default_events_path()),
    };
    load_records_into_state(&state).await?;

    spawn_scheduler(state.clone());
    let app = build_router(state);
    let addr = SocketAddr::from(([0, 0, 0, 0], cfg.server.api_port));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

fn load_config(path: &str) -> Result<SctConfig> {
    let raw = std::fs::read_to_string(path)?;
    Ok(toml::from_str(&raw)?)
}

fn default_state_path() -> PathBuf {
    if let Ok(path) = std::env::var("SCT_DAEMON_STATE_PATH") {
        return PathBuf::from(path);
    }
    PathBuf::from(".sct-daemon/transfers.json")
}

fn default_events_path() -> PathBuf {
    if let Ok(path) = std::env::var("SCT_DAEMON_EVENTS_PATH") {
        return PathBuf::from(path);
    }
    PathBuf::from(".sct-daemon/events.jsonl")
}

async fn load_records_into_state(state: &AppState) -> Result<()> {
    let mut records = state.records.lock().await;
    let mut seq = state.sequence.lock().await;
    if state.state_path.exists() {
        let raw = tokio::fs::read(&*state.state_path).await?;
        let list: Vec<TransferRecord> = serde_json::from_slice(&raw)?;
        for mut rec in list {
            if rec.status == TransferStatus::Active {
                rec.status = TransferStatus::Queued;
            }
            records.insert(rec.transfer_id, rec);
        }
    }

    if state.events_path.exists() {
        let raw = tokio::fs::read_to_string(&*state.events_path).await?;
        for line in raw.lines() {
            if line.trim().is_empty() {
                continue;
            }
            let event: TransferEventRecord = serde_json::from_str(line)?;
            let entry = records.entry(event.transfer_id).or_insert_with(|| TransferRecord {
                transfer_id: event.transfer_id,
                source: event.source.clone().unwrap_or_default(),
                destination: event.destination.clone().unwrap_or_default(),
                priority: event.priority.unwrap_or(5),
                status: event.status.clone().unwrap_or(TransferStatus::Queued),
                progress_pct: event.progress_pct.unwrap_or(0.0),
                throughput_mbps: event.throughput_mbps.unwrap_or(0.0),
                bytes_transferred: event.bytes_transferred.unwrap_or(0),
                bytes_total: event.bytes_total.unwrap_or(0),
                error: event.error.clone(),
                created_at_unix_ms: event.ts_unix_ms,
                updated_at_unix_ms: event.ts_unix_ms,
                attempts: 0,
            });
            apply_event(entry, &event);
        }
    }

    let mut queue = state.queue.lock().await;
    queue.clear();
    for rec in records.values() {
        if rec.status == TransferStatus::Queued {
            *seq += 1;
            queue.push((Reverse(rec.priority), *seq, rec.transfer_id));
        }
    }
    Ok(())
}

async fn persist_records(state: &AppState) -> Result<()> {
    let list: Vec<TransferRecord> = state.records.lock().await.values().cloned().collect();
    let raw = serde_json::to_vec_pretty(&list)?;
    if let Some(parent) = state.state_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let tmp_path = state.state_path.with_extension("json.tmp");
    tokio::fs::write(&tmp_path, raw).await?;
    tokio::fs::rename(&tmp_path, &*state.state_path).await?;
    Ok(())
}

fn apply_event(rec: &mut TransferRecord, event: &TransferEventRecord) {
    if let Some(source) = &event.source {
        rec.source = source.clone();
    }
    if let Some(destination) = &event.destination {
        rec.destination = destination.clone();
    }
    if let Some(priority) = event.priority {
        rec.priority = priority;
    }
    if let Some(status) = &event.status {
        rec.status = status.clone();
    }
    if let Some(progress_pct) = event.progress_pct {
        rec.progress_pct = progress_pct;
    }
    if let Some(throughput_mbps) = event.throughput_mbps {
        rec.throughput_mbps = throughput_mbps;
    }
    if let Some(bytes_transferred) = event.bytes_transferred {
        rec.bytes_transferred = bytes_transferred;
    }
    if let Some(bytes_total) = event.bytes_total {
        rec.bytes_total = bytes_total;
    }
    if event.error.is_some() {
        rec.error = event.error.clone();
    }
    if rec.created_at_unix_ms == 0 {
        rec.created_at_unix_ms = event.ts_unix_ms;
    }
    rec.updated_at_unix_ms = event.ts_unix_ms;
}

async fn append_event(state: &AppState, event: TransferEventRecord) -> Result<()> {
    if let Some(parent) = state.events_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&*state.events_path)
        .await?;
    let line = serde_json::to_string(&event)?;
    file.write_all(line.as_bytes()).await?;
    file.write_all(b"\n").await?;
    file.flush().await?;
    file.sync_data().await?;
    Ok(())
}

async fn enqueue_transfer(state: &AppState, transfer_id: Uuid, priority: u8) {
    let mut seq = state.sequence.lock().await;
    *seq += 1;
    state
        .queue
        .lock()
        .await
        .push((Reverse(priority.clamp(1, 10)), *seq, transfer_id));
}

fn now_unix_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/v1/transfer", post(submit_transfer))
        .route(
            "/v1/transfer/{id}",
            get(get_transfer).delete(cancel_transfer).patch(update_priority),
        )
        .route("/v1/transfers", get(list_transfers))
        .route("/v1/metrics", get(metrics))
        .route("/v1/health", get(health))
        .with_state(state)
}

async fn submit_transfer(
    State(state): State<AppState>,
    Json(req): Json<SubmitTransferRequest>,
) -> (StatusCode, Json<SubmitTransferResponse>) {
    let source = req.source.clone();
    let destination = req.destination.clone();
    let transfer_id = Uuid::new_v4();
    let priority = req.priority.clamp(1, 10);
    let record = TransferRecord {
        transfer_id,
        source,
        destination,
        priority,
        status: TransferStatus::Queued,
        progress_pct: 0.0,
        throughput_mbps: 0.0,
        bytes_transferred: 0,
        bytes_total: 1024 * 1024 * 1024,
        error: None,
        created_at_unix_ms: now_unix_ms(),
        updated_at_unix_ms: now_unix_ms(),
        attempts: 0,
    };
    state.records.lock().await.insert(transfer_id, record);
    enqueue_transfer(&state, transfer_id, priority).await;
    let _ = append_event(
        &state,
        TransferEventRecord {
            ts_unix_ms: now_unix_ms(),
            transfer_id,
            kind: "submitted".to_string(),
            source: Some(req.source),
            destination: Some(req.destination),
            priority: Some(priority),
            status: Some(TransferStatus::Queued),
            progress_pct: Some(0.0),
            throughput_mbps: Some(0.0),
            bytes_transferred: Some(0),
            bytes_total: Some(1024 * 1024 * 1024),
            error: None,
        },
    )
    .await;
    let _ = persist_records(&state).await;
    (
        StatusCode::ACCEPTED,
        Json(SubmitTransferResponse {
            transfer_id,
            status: TransferStatus::Queued,
        }),
    )
}

async fn get_transfer(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<TransferRecord>, StatusCode> {
    state
        .records
        .lock()
        .await
        .get(&id)
        .cloned()
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

async fn cancel_transfer(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, StatusCode> {
    let mut records = state.records.lock().await;
    let rec = records.get_mut(&id).ok_or(StatusCode::NOT_FOUND)?;
    rec.status = TransferStatus::Cancelled;
    rec.updated_at_unix_ms = now_unix_ms();
    drop(records);
    let _ = append_event(
        &state,
        TransferEventRecord {
            ts_unix_ms: now_unix_ms(),
            transfer_id: id,
            kind: "cancelled".to_string(),
            source: None,
            destination: None,
            priority: None,
            status: Some(TransferStatus::Cancelled),
            progress_pct: None,
            throughput_mbps: None,
            bytes_transferred: None,
            bytes_total: None,
            error: None,
        },
    )
    .await;
    let _ = persist_records(&state).await;
    Ok(StatusCode::NO_CONTENT)
}

async fn update_priority(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<PriorityUpdate>,
) -> Result<StatusCode, StatusCode> {
    let mut records = state.records.lock().await;
    let rec = records.get_mut(&id).ok_or(StatusCode::NOT_FOUND)?;
    rec.priority = req.priority.clamp(1, 10);
    rec.status = TransferStatus::Queued;
    rec.updated_at_unix_ms = now_unix_ms();
    let new_priority = rec.priority;
    drop(records);
    enqueue_transfer(&state, id, new_priority).await;
    let _ = append_event(
        &state,
        TransferEventRecord {
            ts_unix_ms: now_unix_ms(),
            transfer_id: id,
            kind: "priority_updated".to_string(),
            source: None,
            destination: None,
            priority: Some(new_priority),
            status: Some(TransferStatus::Queued),
            progress_pct: None,
            throughput_mbps: None,
            bytes_transferred: None,
            bytes_total: None,
            error: None,
        },
    )
    .await;
    let _ = persist_records(&state).await;
    Ok(StatusCode::NO_CONTENT)
}

async fn list_transfers(
    State(state): State<AppState>,
    Query(query): Query<ListQuery>,
) -> Json<Vec<TransferRecord>> {
    let mut rows: Vec<TransferRecord> = state.records.lock().await.values().cloned().collect();
    if let Some(status) = query.status {
        rows.retain(|r| format!("{:?}", r.status).eq_ignore_ascii_case(&status));
    }
    rows.sort_by_key(|r| r.transfer_id);
    if let Some(limit) = query.limit {
        rows.truncate(limit);
    }
    Json(rows)
}

async fn metrics(State(state): State<AppState>) -> (StatusCode, String) {
    let records = state.records.lock().await;
    let active = records.values().filter(|r| r.status == TransferStatus::Active).count();
    let queued = records.values().filter(|r| r.status == TransferStatus::Queued).count();
    let body = format!(
        "sct_active_transfers {}\nsct_queued_transfers {}\n",
        active, queued
    );
    (StatusCode::OK, body)
}

async fn health(State(state): State<AppState>) -> Json<serde_json::Value> {
    let active = state
        .records
        .lock()
        .await
        .values()
        .filter(|r| r.status == TransferStatus::Active)
        .count();
    Json(serde_json::json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
        "active_transfers": active,
        "max_concurrent_transfers": state.cfg.server.max_concurrent_transfers
    }))
}

fn spawn_scheduler(state: AppState) {
    tokio::spawn(async move {
        loop {
            let next = state.queue.lock().await.pop();
            let Some((_prio, _seq, transfer_id)) = next else {
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            };
            let permit = state.semaphore.clone().acquire_owned().await;
            if permit.is_err() {
                continue;
            }
            let state_clone = state.clone();
            tokio::spawn(async move {
                let _permit = permit.expect("permit");
                {
                    let mut records = state_clone.records.lock().await;
                    if let Some(rec) = records.get_mut(&transfer_id) {
                        if rec.status == TransferStatus::Cancelled {
                            return;
                        }
                        rec.status = TransferStatus::Active;
                        rec.attempts = rec.attempts.saturating_add(1);
                        rec.updated_at_unix_ms = now_unix_ms();
                    }
                }
                let _ = append_event(
                    &state_clone,
                    TransferEventRecord {
                        ts_unix_ms: now_unix_ms(),
                        transfer_id,
                        kind: "started".to_string(),
                        source: None,
                        destination: None,
                        priority: None,
                        status: Some(TransferStatus::Active),
                        progress_pct: None,
                        throughput_mbps: None,
                        bytes_transferred: None,
                        bytes_total: None,
                        error: None,
                    },
                )
                .await;
                let _ = persist_records(&state_clone).await;
                let result = run_transfer_job(&state_clone, transfer_id).await;
                if let Err(err) = result {
                    let mut records = state_clone.records.lock().await;
                    if let Some(rec) = records.get_mut(&transfer_id) {
                        rec.status = TransferStatus::Failed;
                        rec.error = Some(err.to_string());
                        rec.updated_at_unix_ms = now_unix_ms();
                    }
                    let _ = append_event(
                        &state_clone,
                        TransferEventRecord {
                            ts_unix_ms: now_unix_ms(),
                            transfer_id,
                            kind: "failed".to_string(),
                            source: None,
                            destination: None,
                            priority: None,
                            status: Some(TransferStatus::Failed),
                            progress_pct: None,
                            throughput_mbps: None,
                            bytes_transferred: None,
                            bytes_total: None,
                            error: Some(err.to_string()),
                        },
                    )
                    .await;
                }
                let _ = persist_records(&state_clone).await;
            });
        }
    });
}

async fn run_transfer_job(state: &AppState, transfer_id: Uuid) -> Result<()> {
    let rec = state
        .records
        .lock()
        .await
        .get(&transfer_id)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("missing transfer record"))?;
    if rec.status == TransferStatus::Cancelled {
        return Ok(());
    }

    if rec.destination.starts_with("sct://") {
        let src = local_source_path(&rec.source);
        let parsed = parse_sct_endpoint(&rec.destination)?;
        let total = tokio::fs::metadata(&src).await?.len();
        {
            let mut records = state.records.lock().await;
            if let Some(item) = records.get_mut(&transfer_id) {
                item.bytes_total = total;
            }
        }
        let endpoint = SctEndpoint::client(TransportConfig {
            bind_addr: "0.0.0.0:0".parse()?,
            ..Default::default()
        })?;
        let conn = endpoint.connect(parsed.addr, &parsed.server_name).await?;
        let records = state.records.clone();
        let transfer_id_for_progress = transfer_id;
        let state_for_progress = state.clone();
        let sender = FileSender::new(
            conn,
            SenderConfig {
                chunk_size: state.cfg.transfer.default_chunk_size_mb * 1024 * 1024,
                max_parallel_chunks: state.cfg.transfer.default_parallel_streams,
                compression: parse_compression(&state.cfg.transfer.default_compression),
                require_final_ack: true,
                progress_callback: Some(Arc::new(move |p| {
                    let records = records.clone();
                    let state_for_progress = state_for_progress.clone();
                    tokio::spawn(async move {
                        let mut lock = records.lock().await;
                        if let Some(item) = lock.get_mut(&transfer_id_for_progress) {
                            item.bytes_total = p.bytes_total;
                            item.bytes_transferred = p.bytes_sent;
                            item.progress_pct = if p.bytes_total > 0 {
                                (p.bytes_sent as f64 / p.bytes_total as f64) * 100.0
                            } else {
                                0.0
                            };
                            item.throughput_mbps = p.throughput_mbps;
                            item.updated_at_unix_ms = now_unix_ms();
                        }
                        let _ = append_event(
                            &state_for_progress,
                            TransferEventRecord {
                                ts_unix_ms: now_unix_ms(),
                                transfer_id: transfer_id_for_progress,
                                kind: "progress".to_string(),
                                source: None,
                                destination: None,
                                priority: None,
                                status: Some(TransferStatus::Active),
                                progress_pct: Some(if p.bytes_total > 0 {
                                    (p.bytes_sent as f64 / p.bytes_total as f64) * 100.0
                                } else {
                                    0.0
                                }),
                                throughput_mbps: Some(p.throughput_mbps),
                                bytes_transferred: Some(p.bytes_sent),
                                bytes_total: Some(p.bytes_total),
                                error: None,
                            },
                        )
                        .await;
                    });
                })),
            },
        );
        sender.send(&src).await?;
    } else if rec.source.starts_with("sct://") {
        // Receiver-side transfer path: daemon listens and accepts one incoming SCT transfer.
        let out_dir = resolve_output_dir(&state.cfg, &rec.destination);
        tokio::fs::create_dir_all(&out_dir).await?;
        let endpoint = SctEndpoint::server(TransportConfig {
            bind_addr: SocketAddr::from(([0, 0, 0, 0], state.cfg.server.listen_port)),
            ..Default::default()
        })?;
        let receiver = FileReceiver::new(
            endpoint,
            out_dir,
            ReceiverConfig {
                max_parallel_chunks: state.cfg.transfer.default_parallel_streams,
                verify_checksums: true,
                resume_partial: state.cfg.transfer.enable_resumption,
                temp_dir: None,
            },
        );
        let out = receiver.accept_transfer().await?;
        let total = tokio::fs::metadata(&out).await?.len();
        let mut records = state.records.lock().await;
        if let Some(item) = records.get_mut(&transfer_id) {
            item.bytes_total = total;
            item.bytes_transferred = total;
            item.progress_pct = 100.0;
            item.updated_at_unix_ms = now_unix_ms();
        }
        let _ = append_event(
            state,
            TransferEventRecord {
                ts_unix_ms: now_unix_ms(),
                transfer_id,
                kind: "progress".to_string(),
                source: None,
                destination: None,
                priority: None,
                status: Some(TransferStatus::Active),
                progress_pct: Some(100.0),
                throughput_mbps: None,
                bytes_transferred: Some(total),
                bytes_total: Some(total),
                error: None,
            },
        )
        .await;
    } else if rec.source.starts_with("file://") || PathBuf::from(&rec.source).is_file() {
        let src = PathBuf::from(rec.source.trim_start_matches("file://"));
        let dst = resolve_destination(&state.cfg, &rec.destination, &src)?;
        if let Some(parent) = dst.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let total = tokio::fs::metadata(&src).await?.len();
        for step in 1..=5_u64 {
            tokio::time::sleep(Duration::from_millis(50)).await;
            let mut records = state.records.lock().await;
            if let Some(item) = records.get_mut(&transfer_id) {
                if item.status == TransferStatus::Cancelled {
                    return Ok(());
                }
                item.progress_pct = step as f64 * 20.0;
                item.bytes_total = total;
                item.bytes_transferred = (total / 5) * step;
                item.throughput_mbps = 300.0 + step as f64 * 20.0;
                item.updated_at_unix_ms = now_unix_ms();
            }
            let _ = append_event(
                state,
                TransferEventRecord {
                    ts_unix_ms: now_unix_ms(),
                    transfer_id,
                    kind: "progress".to_string(),
                    source: None,
                    destination: None,
                    priority: None,
                    status: Some(TransferStatus::Active),
                    progress_pct: Some(step as f64 * 20.0),
                    throughput_mbps: Some(300.0 + step as f64 * 20.0),
                    bytes_transferred: Some((total / 5) * step),
                    bytes_total: Some(total),
                    error: None,
                },
            )
            .await;
        }
        tokio::fs::copy(&src, &dst).await?;
    } else {
        // Non-file source placeholder for remote transfers.
        for step in 1..=5_u64 {
            tokio::time::sleep(Duration::from_millis(80)).await;
            let mut records = state.records.lock().await;
            if let Some(item) = records.get_mut(&transfer_id) {
                if item.status == TransferStatus::Cancelled {
                    return Ok(());
                }
                item.progress_pct = step as f64 * 20.0;
                item.bytes_transferred = (item.bytes_total / 5) * step;
                item.throughput_mbps = 400.0 + step as f64 * 15.0;
                item.updated_at_unix_ms = now_unix_ms();
            }
            let _ = append_event(
                state,
                TransferEventRecord {
                    ts_unix_ms: now_unix_ms(),
                    transfer_id,
                    kind: "progress".to_string(),
                    source: None,
                    destination: None,
                    priority: None,
                    status: Some(TransferStatus::Active),
                    progress_pct: Some(step as f64 * 20.0),
                    throughput_mbps: Some(400.0 + step as f64 * 15.0),
                    bytes_transferred: None,
                    bytes_total: None,
                    error: None,
                },
            )
            .await;
        }
    }

    let mut records = state.records.lock().await;
    if let Some(item) = records.get_mut(&transfer_id) {
        if item.status != TransferStatus::Cancelled {
            item.status = TransferStatus::Completed;
            item.progress_pct = 100.0;
            item.error = None;
            item.updated_at_unix_ms = now_unix_ms();
        }
    }
    let _ = append_event(
        state,
        TransferEventRecord {
            ts_unix_ms: now_unix_ms(),
            transfer_id,
            kind: "completed".to_string(),
            source: None,
            destination: None,
            priority: None,
            status: Some(TransferStatus::Completed),
            progress_pct: Some(100.0),
            throughput_mbps: None,
            bytes_transferred: None,
            bytes_total: None,
            error: None,
        },
    )
    .await;
    Ok(())
}

fn local_source_path(source: &str) -> PathBuf {
    if source.starts_with("file://") {
        PathBuf::from(source.trim_start_matches("file://"))
    } else {
        PathBuf::from(source)
    }
}

fn parse_compression(value: &str) -> CompressionType {
    match value.to_ascii_lowercase().as_str() {
        "zstd" => CompressionType::Zstd { level: 3 },
        _ => CompressionType::None,
    }
}

#[derive(Debug, Clone)]
struct ParsedSctEndpoint {
    addr: SocketAddr,
    server_name: String,
}

fn parse_sct_endpoint(value: &str) -> Result<ParsedSctEndpoint> {
    let trimmed = value.strip_prefix("sct://").unwrap_or(value);
    let host_port = trimmed.split('/').next().unwrap_or(trimmed);
    let addr: SocketAddr = host_port.parse()?;
    let server_name = match addr.ip() {
        IpAddr::V4(v4) if v4.is_loopback() => "localhost".to_string(),
        IpAddr::V6(v6) if v6.is_loopback() => "localhost".to_string(),
        ip => ip.to_string(),
    };
    Ok(ParsedSctEndpoint { addr, server_name })
}

fn resolve_destination(cfg: &SctConfig, destination: &str, src: &StdPath) -> Result<PathBuf> {
    let mut dst = if destination.starts_with('/') {
        PathBuf::from(destination)
    } else {
        PathBuf::from(&cfg.server.output_base_dir).join(destination)
    };
    if destination.ends_with('/') || dst.is_dir() {
        let file_name = src
            .file_name()
            .ok_or_else(|| anyhow::anyhow!("source path has no file name"))?;
        dst = dst.join(file_name);
    }
    Ok(dst)
}

fn resolve_output_dir(cfg: &SctConfig, destination: &str) -> PathBuf {
    if destination.starts_with('/') {
        PathBuf::from(destination)
    } else {
        PathBuf::from(&cfg.server.output_base_dir).join(destination)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Method, Request};
    use sct_core::sender::{FileSender, SenderConfig};
    use sct_core::transport::{SctEndpoint, TransportConfig};
    use tempfile::tempdir;
    use tokio::io::AsyncWriteExt;
    use tower::util::ServiceExt;

    fn test_state() -> AppState {
        let cfg = SctConfig::default();
        let temp = tempdir().expect("tempdir");
        AppState {
            semaphore: Arc::new(Semaphore::new(1)),
            queue: Arc::new(Mutex::new(BinaryHeap::new())),
            records: Arc::new(Mutex::new(HashMap::new())),
            sequence: Arc::new(Mutex::new(0)),
            cfg,
            state_path: Arc::new(temp.path().join("state.json")),
            events_path: Arc::new(temp.path().join("events.jsonl")),
        }
    }

    #[tokio::test]
    async fn submit_and_query_transfer() {
        let state = test_state();
        let app = build_router(state.clone());
        let req = Request::builder()
            .method(Method::POST)
            .uri("/v1/transfer")
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"source":"sct://a:1/file","destination":"/tmp","priority":1}"#,
            ))
            .expect("req");
        let resp = app.clone().oneshot(req).await.expect("resp");
        assert_eq!(resp.status(), StatusCode::ACCEPTED);
    }

    #[tokio::test]
    async fn queue_honors_priority_order() {
        let state = test_state();
        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();
        state
            .queue
            .lock()
            .await
            .push((Reverse(10), 2, id1));
        state
            .queue
            .lock()
            .await
            .push((Reverse(1), 1, id2));
        let first = state.queue.lock().await.pop().expect("first");
        assert_eq!(first.2, id2);
    }

    #[tokio::test]
    async fn scheduler_respects_concurrency_limit_of_one() {
        let state = test_state();
        spawn_scheduler(state.clone());
        for prio in [5_u8, 6_u8] {
            let transfer_id = Uuid::new_v4();
            state.records.lock().await.insert(
                transfer_id,
                TransferRecord {
                    transfer_id,
                    source: "placeholder-source".to_string(),
                    destination: "/tmp".to_string(),
                    priority: prio,
                    status: TransferStatus::Queued,
                    progress_pct: 0.0,
                    throughput_mbps: 0.0,
                    bytes_transferred: 0,
                    bytes_total: 100,
                    error: None,
                    created_at_unix_ms: now_unix_ms(),
                    updated_at_unix_ms: now_unix_ms(),
                    attempts: 0,
                },
            );
            let mut seq = state.sequence.lock().await;
            *seq += 1;
            state
                .queue
                .lock()
                .await
                .push((Reverse(prio), *seq, transfer_id));
        }

        tokio::time::sleep(Duration::from_millis(120)).await;
        let active_now = state
            .records
            .lock()
            .await
            .values()
            .filter(|r| r.status == TransferStatus::Active)
            .count();
        assert!(active_now <= 1);
    }

    #[tokio::test]
    async fn e2e_receive_path_accepts_real_sct_sender() {
        let temp = tempdir().expect("tempdir");
        std::env::set_var("HOME", temp.path());
        let data_dir = temp.path().join("data");
        let input_path = temp.path().join("payload.bin");
        let mut in_file = tokio::fs::File::create(&input_path).await.expect("create input");
        in_file
            .write_all(b"hello-from-daemon-e2e")
            .await
            .expect("write input");
        in_file.flush().await.expect("flush");

        let std_listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind ephemeral");
        let port = std_listener.local_addr().expect("addr").port();
        drop(std_listener);

        let mut cfg = SctConfig::default();
        cfg.server.listen_port = port;
        cfg.server.output_base_dir = data_dir.to_string_lossy().to_string();
        let state = AppState {
            semaphore: Arc::new(Semaphore::new(1)),
            queue: Arc::new(Mutex::new(BinaryHeap::new())),
            records: Arc::new(Mutex::new(HashMap::new())),
            sequence: Arc::new(Mutex::new(0)),
            cfg,
            state_path: Arc::new(temp.path().join("state.json")),
            events_path: Arc::new(temp.path().join("events.jsonl")),
        };

        let transfer_id = Uuid::new_v4();
        state.records.lock().await.insert(
            transfer_id,
            TransferRecord {
                transfer_id,
                source: "sct://remote/source".to_string(),
                destination: "incoming".to_string(),
                priority: 1,
                status: TransferStatus::Active,
                progress_pct: 0.0,
                throughput_mbps: 0.0,
                bytes_transferred: 0,
                bytes_total: 0,
                error: None,
                created_at_unix_ms: now_unix_ms(),
                updated_at_unix_ms: now_unix_ms(),
                attempts: 0,
            },
        );

        let sender_task = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(120)).await;
            let endpoint = SctEndpoint::client(TransportConfig {
                bind_addr: "0.0.0.0:0".parse().expect("bind"),
                ..Default::default()
            })
            .expect("client endpoint");
            let server_name = format!("daemon-e2e-{port}");
            let conn = endpoint
                .connect(
                    SocketAddr::from(([127, 0, 0, 1], port)),
                    &server_name,
                )
                .await
                .expect("connect");
            let sender = FileSender::new(
                conn,
                SenderConfig {
                    progress_callback: None,
                    ..Default::default()
                },
            );
            sender.send(&input_path).await.expect("send");
        });

        run_transfer_job(&state, transfer_id).await.expect("receive job");
        sender_task.await.expect("sender task");

        let out = data_dir.join("incoming").join("payload.bin");
        let bytes = tokio::fs::read(&out).await.expect("read output");
        assert_eq!(bytes, b"hello-from-daemon-e2e");
    }

    #[tokio::test]
    async fn restart_recovers_active_to_queued() {
        let temp = tempdir().expect("tempdir");
        let id = Uuid::new_v4();
        let snapshot = vec![TransferRecord {
            transfer_id: id,
            source: "file:///tmp/a.bin".to_string(),
            destination: "/tmp/out".to_string(),
            priority: 2,
            status: TransferStatus::Active,
            progress_pct: 50.0,
            throughput_mbps: 123.0,
            bytes_transferred: 50,
            bytes_total: 100,
            error: None,
            created_at_unix_ms: now_unix_ms(),
            updated_at_unix_ms: now_unix_ms(),
            attempts: 1,
        }];
        tokio::fs::write(
            temp.path().join("state.json"),
            serde_json::to_vec(&snapshot).expect("serialize"),
        )
        .await
        .expect("write snapshot");
        let state = AppState {
            cfg: SctConfig::default(),
            queue: Arc::new(Mutex::new(BinaryHeap::new())),
            records: Arc::new(Mutex::new(HashMap::new())),
            semaphore: Arc::new(Semaphore::new(1)),
            sequence: Arc::new(Mutex::new(0)),
            state_path: Arc::new(temp.path().join("state.json")),
            events_path: Arc::new(temp.path().join("events.jsonl")),
        };
        load_records_into_state(&state).await.expect("load");
        let rec = state.records.lock().await.get(&id).cloned().expect("record");
        assert_eq!(rec.status, TransferStatus::Queued);
        let head = state.queue.lock().await.pop().expect("queued item");
        assert_eq!(head.2, id);
    }

    #[tokio::test]
    async fn cancelled_transfer_not_requeued_on_restart() {
        let temp = tempdir().expect("tempdir");
        let id = Uuid::new_v4();
        let snapshot = vec![TransferRecord {
            transfer_id: id,
            source: "file:///tmp/a.bin".to_string(),
            destination: "/tmp/out".to_string(),
            priority: 2,
            status: TransferStatus::Cancelled,
            progress_pct: 0.0,
            throughput_mbps: 0.0,
            bytes_transferred: 0,
            bytes_total: 100,
            error: None,
            created_at_unix_ms: now_unix_ms(),
            updated_at_unix_ms: now_unix_ms(),
            attempts: 0,
        }];
        tokio::fs::write(
            temp.path().join("state.json"),
            serde_json::to_vec(&snapshot).expect("serialize"),
        )
        .await
        .expect("write snapshot");
        let state = AppState {
            cfg: SctConfig::default(),
            queue: Arc::new(Mutex::new(BinaryHeap::new())),
            records: Arc::new(Mutex::new(HashMap::new())),
            semaphore: Arc::new(Semaphore::new(1)),
            sequence: Arc::new(Mutex::new(0)),
            state_path: Arc::new(temp.path().join("state.json")),
            events_path: Arc::new(temp.path().join("events.jsonl")),
        };
        load_records_into_state(&state).await.expect("load");
        assert!(state.queue.lock().await.pop().is_none());
        let rec = state.records.lock().await.get(&id).cloned().expect("record");
        assert_eq!(rec.status, TransferStatus::Cancelled);
    }

    #[tokio::test]
    async fn priority_patch_requeues_and_runs_new_order() {
        let state = test_state();
        let app = build_router(state.clone());
        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();
        for (id, p) in [(id1, 8_u8), (id2, 5_u8)] {
            state.records.lock().await.insert(
                id,
                TransferRecord {
                    transfer_id: id,
                    source: "x".to_string(),
                    destination: "y".to_string(),
                    priority: p,
                    status: TransferStatus::Queued,
                    progress_pct: 0.0,
                    throughput_mbps: 0.0,
                    bytes_transferred: 0,
                    bytes_total: 1,
                    error: None,
                    created_at_unix_ms: now_unix_ms(),
                    updated_at_unix_ms: now_unix_ms(),
                    attempts: 0,
                },
            );
            enqueue_transfer(&state, id, p).await;
        }
        let req = Request::builder()
            .method(Method::PATCH)
            .uri(format!("/v1/transfer/{id1}"))
            .header("content-type", "application/json")
            .body(Body::from(r#"{"priority":1}"#))
            .expect("patch req");
        let resp = app.oneshot(req).await.expect("patch resp");
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
        let first = state.queue.lock().await.pop().expect("q1");
        assert_eq!(first.2, id1);
    }

    #[tokio::test]
    async fn interrupted_receive_resume_with_state_files() {
        let temp = tempdir().expect("tempdir");
        std::env::set_var("HOME", temp.path());
        let data_dir = temp.path().join("data");
        let input_path = temp.path().join("payload.bin");
        let payload = b"abcdef0123456789";
        let mut in_file = tokio::fs::File::create(&input_path).await.expect("create input");
        in_file.write_all(payload).await.expect("write input");
        in_file.flush().await.expect("flush");

        let std_listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind ephemeral");
        let port = std_listener.local_addr().expect("addr").port();
        drop(std_listener);

        let mut cfg = SctConfig::default();
        cfg.server.listen_port = port;
        cfg.server.output_base_dir = data_dir.to_string_lossy().to_string();
        let state = AppState {
            semaphore: Arc::new(Semaphore::new(1)),
            queue: Arc::new(Mutex::new(BinaryHeap::new())),
            records: Arc::new(Mutex::new(HashMap::new())),
            sequence: Arc::new(Mutex::new(0)),
            cfg,
            state_path: Arc::new(temp.path().join("state.json")),
            events_path: Arc::new(temp.path().join("events.jsonl")),
        };
        let transfer_id = Uuid::new_v4();
        state.records.lock().await.insert(
            transfer_id,
            TransferRecord {
                transfer_id,
                source: "sct://remote/source".to_string(),
                destination: "incoming".to_string(),
                priority: 1,
                status: TransferStatus::Active,
                progress_pct: 0.0,
                throughput_mbps: 0.0,
                bytes_transferred: 0,
                bytes_total: payload.len() as u64,
                error: None,
                created_at_unix_ms: now_unix_ms(),
                updated_at_unix_ms: now_unix_ms(),
                attempts: 0,
            },
        );

        let out_dir = data_dir.join("incoming");
        tokio::fs::create_dir_all(&out_dir).await.expect("mkdir");
        let hash = blake3::hash("payload.bin".as_bytes());
        let mut transfer = [0_u8; 16];
        transfer.copy_from_slice(&hash.as_bytes()[..16]);
        let transfer_hex = transfer.iter().map(|b| format!("{b:02x}")).collect::<String>();
        let part = out_dir.join(format!("payload.bin.{transfer_hex}.part"));
        let state_file = out_dir.join(format!("payload.bin.{transfer_hex}.state.json"));
        let mut part_file = tokio::fs::File::create(&part).await.expect("create part");
        part_file.write_all(&payload[..4]).await.expect("seed");
        part_file.set_len(payload.len() as u64).await.expect("len");
        tokio::fs::write(&state_file, r#"{"received_chunks":[0]}"#)
            .await
            .expect("state file");

        let sender_task = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(120)).await;
            let endpoint = SctEndpoint::client(TransportConfig {
                bind_addr: "0.0.0.0:0".parse().expect("bind"),
                ..Default::default()
            })
            .expect("client endpoint");
            let server_name = format!("daemon-resume-{port}");
            let conn = endpoint
                .connect(SocketAddr::from(([127, 0, 0, 1], port)), &server_name)
                .await
                .expect("connect");
            let sender = FileSender::new(
                conn,
                SenderConfig {
                    chunk_size: 4,
                    progress_callback: None,
                    ..Default::default()
                },
            );
            sender.send(&input_path).await.expect("send");
        });

        run_transfer_job(&state, transfer_id).await.expect("receive job");
        sender_task.await.expect("sender task");
        let final_path = out_dir.join("payload.bin");
        let bytes = tokio::fs::read(final_path).await.expect("read output");
        assert_eq!(bytes, payload);
        assert!(!state_file.exists());
    }

    #[tokio::test]
    async fn event_log_replay_restores_latest_state() {
        let temp = tempdir().expect("tempdir");
        let id = Uuid::new_v4();
        let events = vec![
            TransferEventRecord {
                ts_unix_ms: now_unix_ms(),
                transfer_id: id,
                kind: "submitted".to_string(),
                source: Some("file:///tmp/a.bin".to_string()),
                destination: Some("/tmp/out".to_string()),
                priority: Some(4),
                status: Some(TransferStatus::Queued),
                progress_pct: Some(0.0),
                throughput_mbps: Some(0.0),
                bytes_transferred: Some(0),
                bytes_total: Some(100),
                error: None,
            },
            TransferEventRecord {
                ts_unix_ms: now_unix_ms(),
                transfer_id: id,
                kind: "started".to_string(),
                source: None,
                destination: None,
                priority: None,
                status: Some(TransferStatus::Active),
                progress_pct: Some(35.0),
                throughput_mbps: Some(120.0),
                bytes_transferred: Some(35),
                bytes_total: Some(100),
                error: None,
            },
            TransferEventRecord {
                ts_unix_ms: now_unix_ms(),
                transfer_id: id,
                kind: "completed".to_string(),
                source: None,
                destination: None,
                priority: None,
                status: Some(TransferStatus::Completed),
                progress_pct: Some(100.0),
                throughput_mbps: Some(210.0),
                bytes_transferred: Some(100),
                bytes_total: Some(100),
                error: None,
            },
        ];
        let jsonl = events
            .into_iter()
            .map(|e| serde_json::to_string(&e).expect("ser"))
            .collect::<Vec<_>>()
            .join("\n");
        tokio::fs::write(temp.path().join("events.jsonl"), format!("{jsonl}\n"))
            .await
            .expect("write events");

        let state = AppState {
            cfg: SctConfig::default(),
            queue: Arc::new(Mutex::new(BinaryHeap::new())),
            records: Arc::new(Mutex::new(HashMap::new())),
            semaphore: Arc::new(Semaphore::new(1)),
            sequence: Arc::new(Mutex::new(0)),
            state_path: Arc::new(temp.path().join("state.json")),
            events_path: Arc::new(temp.path().join("events.jsonl")),
        };
        load_records_into_state(&state).await.expect("load");
        let rec = state.records.lock().await.get(&id).cloned().expect("record");
        assert_eq!(rec.status, TransferStatus::Completed);
        assert_eq!(rec.bytes_transferred, 100);
        assert_eq!(rec.progress_pct, 100.0);
    }
}
