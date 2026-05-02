# Competitive Baseline Matrix

- baseline repeats: 5
- compared transports: sc transports + raw TCP + raw UDP baselines

## excellent / medium
- baseline-udp: p50=100250.63 eps, 50GiB=0.1 min, 100GiB=0.3 min, fallback=0, dropped=0
- sse: p50=61871.62 eps, 50GiB=0.2 min, 100GiB=0.4 min, fallback=0, dropped=0
- quic-datagram: p50=41690.20 eps, 50GiB=0.3 min, 100GiB=0.7 min, fallback=199, dropped=1
- baseline-tcp: p50=24605.30 eps, 50GiB=0.6 min, 100GiB=1.1 min, fallback=0, dropped=0
- quic-stream: p50=2161.53 eps, 50GiB=6.3 min, 100GiB=12.6 min, fallback=0, dropped=0

## excellent / large
- baseline-udp: p50=96793.71 eps, 50GiB=0.0 min, 100GiB=0.1 min, fallback=0, dropped=0
- quic-datagram: p50=27277.37 eps, 50GiB=0.1 min, 100GiB=0.3 min, fallback=200, dropped=0
- baseline-tcp: p50=13158.25 eps, 50GiB=0.3 min, 100GiB=0.5 min, fallback=0, dropped=0
- quic-stream: p50=12153.13 eps, 50GiB=0.3 min, 100GiB=0.6 min, fallback=0, dropped=0
- sse: p50=6721.62 eps, 50GiB=0.5 min, 100GiB=1.0 min, fallback=0, dropped=0

## good / medium
- baseline-udp: p50=524.85 eps, 50GiB=26.0 min, 100GiB=52.0 min, fallback=0, dropped=0
- baseline-tcp: p50=516.29 eps, 50GiB=26.4 min, 100GiB=52.9 min, fallback=0, dropped=0
- quic-stream: p50=369.28 eps, 50GiB=37.0 min, 100GiB=73.9 min, fallback=0, dropped=0
- quic-datagram: p50=351.02 eps, 50GiB=38.9 min, 100GiB=77.8 min, fallback=199, dropped=1
- sse: p50=350.63 eps, 50GiB=38.9 min, 100GiB=77.9 min, fallback=0, dropped=0

## good / large
- baseline-udp: p50=521.41 eps, 50GiB=6.5 min, 100GiB=13.1 min, fallback=0, dropped=0
- baseline-tcp: p50=501.18 eps, 50GiB=6.8 min, 100GiB=13.6 min, fallback=0, dropped=0
- quic-datagram: p50=349.81 eps, 50GiB=9.8 min, 100GiB=19.5 min, fallback=199, dropped=1
- sse: p50=345.70 eps, 50GiB=9.9 min, 100GiB=19.7 min, fallback=0, dropped=0
- quic-stream: p50=243.64 eps, 50GiB=14.0 min, 100GiB=28.0 min, fallback=0, dropped=0

## poor / medium
- baseline-tcp: p50=125.87 eps, 50GiB=108.5 min, 100GiB=216.9 min, fallback=0, dropped=0
- quic-stream: p50=123.17 eps, 50GiB=110.8 min, 100GiB=221.7 min, fallback=0, dropped=0
- quic-datagram: p50=120.44 eps, 50GiB=113.4 min, 100GiB=226.7 min, fallback=188, dropped=12
- baseline-udp: p50=119.59 eps, 50GiB=114.2 min, 100GiB=228.3 min, fallback=0, dropped=5
- sse: p50=118.70 eps, 50GiB=115.0 min, 100GiB=230.0 min, fallback=0, dropped=0

## poor / large
- quic-stream: p50=124.84 eps, 50GiB=27.3 min, 100GiB=54.7 min, fallback=0, dropped=0
- baseline-tcp: p50=123.85 eps, 50GiB=27.6 min, 100GiB=55.1 min, fallback=0, dropped=0
- baseline-udp: p50=120.32 eps, 50GiB=28.4 min, 100GiB=56.7 min, fallback=0, dropped=5
- sse: p50=119.47 eps, 50GiB=28.6 min, 100GiB=57.1 min, fallback=0, dropped=0
- quic-datagram: p50=118.80 eps, 50GiB=28.7 min, 100GiB=57.5 min, fallback=188, dropped=12

## very-poor / medium
- quic-stream: p50=66.90 eps, 50GiB=204.1 min, 100GiB=408.1 min, fallback=0, dropped=0
- sse: p50=66.79 eps, 50GiB=204.4 min, 100GiB=408.9 min, fallback=0, dropped=0
- quic-datagram: p50=66.69 eps, 50GiB=204.7 min, 100GiB=409.5 min, fallback=200, dropped=0
- baseline-tcp: p50=64.04 eps, 50GiB=213.2 min, 100GiB=426.4 min, fallback=0, dropped=0
- baseline-udp: p50=54.12 eps, 50GiB=252.3 min, 100GiB=504.6 min, fallback=0, dropped=15

## very-poor / large
- quic-stream: p50=66.78 eps, 50GiB=51.1 min, 100GiB=102.2 min, fallback=0, dropped=0
- sse: p50=66.64 eps, 50GiB=51.2 min, 100GiB=102.4 min, fallback=0, dropped=0
- quic-datagram: p50=66.47 eps, 50GiB=51.3 min, 100GiB=102.7 min, fallback=200, dropped=0
- baseline-tcp: p50=63.30 eps, 50GiB=53.9 min, 100GiB=107.8 min, fallback=0, dropped=0
- baseline-udp: p50=54.40 eps, 50GiB=62.7 min, 100GiB=125.5 min, fallback=0, dropped=15

