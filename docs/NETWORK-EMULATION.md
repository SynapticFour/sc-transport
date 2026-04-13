# Network Emulation (Linux + macOS)

This runbook explains how to emulate WAN quality for local `sct` transfer tests.

- Linux equivalent: `tc netem`
- macOS equivalent: `pf` + `dummynet` (`pfctl` + `dnctl`)

> Note: both Linux and macOS traffic shaping commands require root privileges.

## Linux quick start (`tc netem`)

Use the built-in matrix command:

```bash
cargo run -p sct-bench -- netem-matrix \
  --interface lo \
  --profile all \
  --sizes-mib 1,16,256,1024 \
  --output-json docs/RESULTS/netem-matrix.json
```

## macOS equivalent (`pf` + `dnctl`)

macOS does not provide `tc netem`. The closest native alternative is:

- `dnctl pipe ... config ...` for delay/loss/rate
- `pfctl` rules to attach UDP traffic to that pipe

### 1) Create a temporary anchor

```bash
cat > /tmp/sct-netem-anchor.conf <<'EOF'
dummynet out quick proto udp from any to any port 7272 pipe 1
dummynet in  quick proto udp from any to any port 7272 pipe 1
EOF
```

### 2) Enable pipe profile + rules

Toronto <-> Auckland is usually long-haul intercontinental:
- RTT: often around 180-260 ms
- jitter: variable
- packet loss: low but non-zero in real paths

The profile below is a practical approximation:

```bash
# Pipe 1: bandwidth + one-way delay + packet loss ratio (plr)
sudo dnctl pipe 1 config bw 80Mbit/s delay 110ms plr 0.008

# Load anchor rules and enable PF
sudo pfctl -a com.synapticfour.sct -f /tmp/sct-netem-anchor.conf
sudo pfctl -e
```

This results in approximately:
- ~220 ms RTT (110 ms each direction)
- 0.8% loss
- 80 Mbit/s bottleneck

### 3) Run a 20 GB transfer test

Terminal A (receiver):

```bash
cargo run -p sct-cli -- recv --port 7272 --output-dir /tmp/sct-recv --once
```

Terminal B (sender):

```bash
mkdir -p /tmp/sct-send /tmp/sct-recv
mkfile -n 20g /tmp/sct-send/payload-20g.bin

time cargo run -p sct-cli -- send /tmp/sct-send/payload-20g.bin sct://127.0.0.1:7272 --compression none --quiet
```

Optional integrity check:

```bash
shasum -a 256 /tmp/sct-send/payload-20g.bin /tmp/sct-recv/payload-20g.bin
```

### 4) Cleanup (important)

```bash
sudo pfctl -a com.synapticfour.sct -F rules
sudo dnctl -q flush
sudo pfctl -d
rm -f /tmp/sct-netem-anchor.conf
```

## Additional informative profile examples

Replace only the `dnctl pipe` command, keeping the same anchor and `sct` commands.

### Good metro / regional

```bash
sudo dnctl pipe 1 config bw 900Mbit/s delay 6ms plr 0.0001
```

- Approx: 12 ms RTT, near-zero loss, high capacity

### Busy continental

```bash
sudo dnctl pipe 1 config bw 200Mbit/s delay 35ms plr 0.002
```

- Approx: 70 ms RTT, 0.2% loss

### Very degraded long-haul

```bash
sudo dnctl pipe 1 config bw 20Mbit/s delay 150ms plr 0.03
```

- Approx: 300 ms RTT, 3% loss, constrained throughput

## CI suggestion

For CI on Linux runners, prefer `sct-bench netem-matrix`.

For CI on macOS runners:
- either run unshaped synthetic benchmarks only, or
- use privileged runners and execute the `pfctl`/`dnctl` setup before transfer tests.
