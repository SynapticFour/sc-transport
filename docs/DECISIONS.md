# Transport decisions

Project-wide ADRs live in [`DECISIONS.md`](../DECISIONS.md) at the repo root.

### 2026-05-16 - QUIC persistent uni: Mutex → Write-Task (geplant, separater PR)

- **Context:** `QuicStreamTransport` hält den persistenten Client-Uni-Stream unter `shared_uni_stream: Mutex<Option<SendStream>>`. Kleine Events serialisieren zwar schnell, aber jeder Send nimmt den Mutex; große Events sind bereits per `open_uni()` entkoppelt (Schwellwert), dennoch bleibt der Mutex die zentrale Engstelle und Reconnect-/Fehlerpfade sind im `send_event`-Kontext verteilt.
- **Decision (langfristig, kein sofortiger Fix):** Ersetzen durch ein **Write-Task-Pattern**: beim Verbindungsaufbau `mpsc::channel` (z. B. Kapazität **256** Frames), ein `tokio::spawn`-Task besitzt den Uni-Stream, empfängt fertig gerahmte `Vec<u8>` / `Bytes`, schreibt sequentiell mit `write_framed_bytes`, bricht bei Fehler ab und kapselt **Reconnect an einer Stelle**. Send-Pfad für kleine Events: `write_tx.send(framed).await` statt Mutex-Guard; Backpressure über Kanal-Länge.
- **Consequences:** Kein Mutex auf dem Hot Path; begrenzte Queue = sichtbare Backpressure statt implizitem Blocking; klarere Ownership des Streams; Implementierung muss Shutdown (`Drop` / explizites Close), Flush-Intervall, und Koexistenz mit `send_events_batch` (bestehendes `finish_persistent_uni_stream`) sauber definieren.
- **Tracking:** Umsetzung als **eigener PR** nach Abstimmung; bis dahin bleibt der Mutex-Pfad + Large-Frame-`open_uni()`-Bypass bestehen (`crates/sc-transport-quic`).

### 2026-05 Speculative Duplication aktiviert

**Entscheidung:** `duplicate_budget=4` mit Receiver-Dedup per Chunk-Index (kein Proto-Upgrade nötig).
**Alternativen (für später):** Sequenznummer-basierte Dedup, oder QUIC-Stream-IDs als Dedup-Key.
**Tests:** `duplicate_chunks_are_deduplicated` (`transfer_smoke`), `loopback_with_duplication` (`e2e_loopback`).
**Erhöhung:** `duplicate_budget=8` nach weiteren Loss-Szenarien.

### 2026-05 Oracle Cloud entfernt

Oracle Cloud Always Free hat uns bei der Account-Erstellung
abgewiesen (bekanntes Problem). Ersatz:
- Fly.io: kostenlos, zwei Regionen (fra/iad), sofort nutzbar
- AWS EC2 t4g.small: ~$0.034/Test-Run, aus AWS Activate Credits
Terraform für Oracle wurde vollständig entfernt.

### 2026-05 RTT-Varianz EWMA für Stabilizer

**Entscheidung:** Quadratische RTT-Abweichung vom `min_rtt` als EWMA (`0.9/0.1`) in BBR und `HybridCongestionController`; `rtt_variance_trend` speist Queue-Pressure im Predictive-Stabilizer.
**Alternativen:** Rohe `abs(rtt-min)` (zu rauschig), QUIC-internal RTTVAR ohne App-Sicht.
**Tests:** `congestion::tests::rtt_variance_ewma_tracks_rtt_jitter`.
