# Competitive Baseline Matrix

- baseline repeats: 5
- compared transports: sc transports + raw TCP + raw UDP baselines

## excellent / medium
- sse: p50=122652.36 eps, 50GiB=0.1 min, 100GiB=0.2 min, fallback=0, dropped=0
- baseline-udp: p50=108572.33 eps, 50GiB=0.1 min, 100GiB=0.3 min, fallback=0, dropped=0
- quic-datagram: p50=82149.52 eps, 50GiB=0.2 min, 100GiB=0.3 min, fallback=199, dropped=1
- baseline-tcp: p50=30625.95 eps, 50GiB=0.4 min, 100GiB=0.9 min, fallback=0, dropped=0
- quic-stream: p50=4043.54 eps, 50GiB=3.4 min, 100GiB=6.8 min, fallback=0, dropped=0

## excellent / large
- baseline-udp: p50=133965.65 eps, 50GiB=0.0 min, 100GiB=0.1 min, fallback=0, dropped=0
- quic-datagram: p50=34521.21 eps, 50GiB=0.1 min, 100GiB=0.2 min, fallback=200, dropped=0
- quic-stream: p50=15553.10 eps, 50GiB=0.2 min, 100GiB=0.4 min, fallback=0, dropped=0
- baseline-tcp: p50=10723.38 eps, 50GiB=0.3 min, 100GiB=0.6 min, fallback=0, dropped=0
- sse: p50=10352.41 eps, 50GiB=0.3 min, 100GiB=0.7 min, fallback=0, dropped=0

## good / medium
- baseline-udp: p50=530.97 eps, 50GiB=25.7 min, 100GiB=51.4 min, fallback=0, dropped=0
- baseline-tcp: p50=529.77 eps, 50GiB=25.8 min, 100GiB=51.5 min, fallback=0, dropped=0
- quic-stream: p50=366.84 eps, 50GiB=37.2 min, 100GiB=74.4 min, fallback=0, dropped=0
- quic-datagram: p50=354.97 eps, 50GiB=38.5 min, 100GiB=76.9 min, fallback=199, dropped=1
- sse: p50=353.02 eps, 50GiB=38.7 min, 100GiB=77.4 min, fallback=0, dropped=0

## good / large
- baseline-udp: p50=532.66 eps, 50GiB=6.4 min, 100GiB=12.8 min, fallback=0, dropped=0
- baseline-tcp: p50=525.43 eps, 50GiB=6.5 min, 100GiB=13.0 min, fallback=0, dropped=0
- sse: p50=354.88 eps, 50GiB=9.6 min, 100GiB=19.2 min, fallback=0, dropped=0
- quic-datagram: p50=349.59 eps, 50GiB=9.8 min, 100GiB=19.5 min, fallback=199, dropped=1
- quic-stream: p50=248.52 eps, 50GiB=13.7 min, 100GiB=27.5 min, fallback=0, dropped=0

## poor / medium
- baseline-tcp: p50=127.58 eps, 50GiB=107.0 min, 100GiB=214.0 min, fallback=0, dropped=0
- quic-stream: p50=122.85 eps, 50GiB=111.1 min, 100GiB=222.3 min, fallback=0, dropped=0
- quic-datagram: p50=120.72 eps, 50GiB=113.1 min, 100GiB=226.2 min, fallback=188, dropped=12
- baseline-udp: p50=120.62 eps, 50GiB=113.2 min, 100GiB=226.4 min, fallback=0, dropped=5
- sse: p50=120.26 eps, 50GiB=113.5 min, 100GiB=227.1 min, fallback=0, dropped=0

## poor / large
- baseline-tcp: p50=126.04 eps, 50GiB=27.1 min, 100GiB=54.2 min, fallback=0, dropped=0
- quic-stream: p50=125.83 eps, 50GiB=27.1 min, 100GiB=54.3 min, fallback=0, dropped=0
- sse: p50=120.25 eps, 50GiB=28.4 min, 100GiB=56.8 min, fallback=0, dropped=0
- baseline-udp: p50=119.77 eps, 50GiB=28.5 min, 100GiB=57.0 min, fallback=0, dropped=5
- quic-datagram: p50=119.55 eps, 50GiB=28.6 min, 100GiB=57.1 min, fallback=188, dropped=12

## very-poor / medium
- quic-stream: p50=67.68 eps, 50GiB=201.7 min, 100GiB=403.5 min, fallback=0, dropped=0
- quic-datagram: p50=67.12 eps, 50GiB=203.4 min, 100GiB=406.8 min, fallback=200, dropped=0
- sse: p50=67.10 eps, 50GiB=203.5 min, 100GiB=406.9 min, fallback=0, dropped=0
- baseline-tcp: p50=65.12 eps, 50GiB=209.7 min, 100GiB=419.3 min, fallback=0, dropped=0
- baseline-udp: p50=55.59 eps, 50GiB=245.6 min, 100GiB=491.2 min, fallback=0, dropped=15

## very-poor / large
- sse: p50=67.13 eps, 50GiB=50.9 min, 100GiB=101.7 min, fallback=0, dropped=0
- quic-stream: p50=67.02 eps, 50GiB=50.9 min, 100GiB=101.9 min, fallback=0, dropped=0
- quic-datagram: p50=66.91 eps, 50GiB=51.0 min, 100GiB=102.0 min, fallback=200, dropped=0
- baseline-tcp: p50=63.96 eps, 50GiB=53.4 min, 100GiB=106.7 min, fallback=0, dropped=0
- baseline-udp: p50=55.26 eps, 50GiB=61.8 min, 100GiB=123.5 min, fallback=0, dropped=15

