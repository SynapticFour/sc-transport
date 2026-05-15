# Transport decisions

Project-wide ADRs live in [`DECISIONS.md`](../DECISIONS.md) at the repo root.

### 2026-05 Speculative Duplication aktiviert

**Entscheidung:** `duplicate_budget=4` mit Receiver-Dedup per Chunk-Index (kein Proto-Upgrade nötig).
**Alternativen (für später):** Sequenznummer-basierte Dedup, oder QUIC-Stream-IDs als Dedup-Key.
**Tests:** `duplicate_chunks_are_deduplicated` (`transfer_smoke`), `loopback_with_duplication` (`e2e_loopback`).
**Erhöhung:** `duplicate_budget=8` nach weiteren Loss-Szenarien.

### 2026-05 RTT-Varianz EWMA für Stabilizer

**Entscheidung:** Quadratische RTT-Abweichung vom `min_rtt` als EWMA (`0.9/0.1`) in BBR und `HybridCongestionController`; `rtt_variance_trend` speist Queue-Pressure im Predictive-Stabilizer.
**Alternativen:** Rohe `abs(rtt-min)` (zu rauschig), QUIC-internal RTTVAR ohne App-Sicht.
**Tests:** `congestion::tests::rtt_variance_ewma_tracks_rtt_jitter`.
