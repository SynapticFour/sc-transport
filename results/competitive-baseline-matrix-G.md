# Competitive Baseline Matrix

- baseline repeats: 5
- compared transports: sc transports + raw TCP + raw UDP baselines

## excellent / medium
- baseline-udp: p50=118255.73 eps, 50GiB=0.1 min, 100GiB=0.2 min, fallback=0, dropped=0
- quic-datagram: p50=82191.72 eps, 50GiB=0.2 min, 100GiB=0.3 min, fallback=199, dropped=1
- sse: p50=47888.65 eps, 50GiB=0.3 min, 100GiB=0.6 min, fallback=0, dropped=0
- quic-stream: p50=44419.77 eps, 50GiB=0.3 min, 100GiB=0.6 min, fallback=0, dropped=0
- baseline-tcp: p50=41562.07 eps, 50GiB=0.3 min, 100GiB=0.7 min, fallback=0, dropped=0

## excellent / large
- baseline-udp: p50=129764.80 eps, 50GiB=0.0 min, 100GiB=0.1 min, fallback=0, dropped=0
- baseline-tcp: p50=13686.91 eps, 50GiB=0.2 min, 100GiB=0.5 min, fallback=0, dropped=0
- quic-stream: p50=13094.00 eps, 50GiB=0.3 min, 100GiB=0.5 min, fallback=0, dropped=0
- quic-datagram: p50=12150.67 eps, 50GiB=0.3 min, 100GiB=0.6 min, fallback=200, dropped=0
- sse: p50=10832.60 eps, 50GiB=0.3 min, 100GiB=0.6 min, fallback=0, dropped=0

## good / medium
- baseline-udp: p50=531.70 eps, 50GiB=25.7 min, 100GiB=51.4 min, fallback=0, dropped=0
- baseline-tcp: p50=530.60 eps, 50GiB=25.7 min, 100GiB=51.5 min, fallback=0, dropped=0
- quic-stream: p50=360.75 eps, 50GiB=37.8 min, 100GiB=75.7 min, fallback=0, dropped=0
- sse: p50=353.86 eps, 50GiB=38.6 min, 100GiB=77.2 min, fallback=0, dropped=0
- quic-datagram: p50=353.33 eps, 50GiB=38.6 min, 100GiB=77.3 min, fallback=199, dropped=1

## good / large
- baseline-udp: p50=530.32 eps, 50GiB=6.4 min, 100GiB=12.9 min, fallback=0, dropped=0
- baseline-tcp: p50=528.02 eps, 50GiB=6.5 min, 100GiB=12.9 min, fallback=0, dropped=0
- sse: p50=354.47 eps, 50GiB=9.6 min, 100GiB=19.3 min, fallback=0, dropped=0
- quic-datagram: p50=339.91 eps, 50GiB=10.0 min, 100GiB=20.1 min, fallback=199, dropped=1
- quic-stream: p50=261.58 eps, 50GiB=13.0 min, 100GiB=26.1 min, fallback=0, dropped=0

## poor / medium
- baseline-tcp: p50=127.33 eps, 50GiB=107.2 min, 100GiB=214.5 min, fallback=0, dropped=0
- quic-stream: p50=122.29 eps, 50GiB=111.7 min, 100GiB=223.3 min, fallback=0, dropped=0
- quic-datagram: p50=121.08 eps, 50GiB=112.8 min, 100GiB=225.5 min, fallback=188, dropped=12
- baseline-udp: p50=120.83 eps, 50GiB=113.0 min, 100GiB=226.0 min, fallback=0, dropped=5
- sse: p50=120.07 eps, 50GiB=113.7 min, 100GiB=227.4 min, fallback=0, dropped=0

## poor / large
- baseline-tcp: p50=125.90 eps, 50GiB=27.1 min, 100GiB=54.2 min, fallback=0, dropped=0
- baseline-udp: p50=120.20 eps, 50GiB=28.4 min, 100GiB=56.8 min, fallback=0, dropped=5
- sse: p50=120.09 eps, 50GiB=28.4 min, 100GiB=56.8 min, fallback=0, dropped=0
- quic-datagram: p50=119.93 eps, 50GiB=28.5 min, 100GiB=56.9 min, fallback=188, dropped=12
- quic-stream: p50=116.71 eps, 50GiB=29.2 min, 100GiB=58.5 min, fallback=0, dropped=0

## very-poor / medium
- quic-stream: p50=67.64 eps, 50GiB=201.9 min, 100GiB=403.7 min, fallback=0, dropped=0
- sse: p50=67.24 eps, 50GiB=203.1 min, 100GiB=406.1 min, fallback=0, dropped=0
- quic-datagram: p50=66.84 eps, 50GiB=204.3 min, 100GiB=408.5 min, fallback=200, dropped=0
- baseline-tcp: p50=65.28 eps, 50GiB=209.1 min, 100GiB=418.3 min, fallback=0, dropped=0
- baseline-udp: p50=54.44 eps, 50GiB=250.8 min, 100GiB=501.6 min, fallback=0, dropped=15

## very-poor / large
- quic-stream: p50=67.57 eps, 50GiB=50.5 min, 100GiB=101.0 min, fallback=0, dropped=0
- quic-datagram: p50=67.14 eps, 50GiB=50.8 min, 100GiB=101.7 min, fallback=200, dropped=0
- sse: p50=66.88 eps, 50GiB=51.0 min, 100GiB=102.1 min, fallback=0, dropped=0
- baseline-tcp: p50=64.20 eps, 50GiB=53.2 min, 100GiB=106.3 min, fallback=0, dropped=0
- baseline-udp: p50=54.75 eps, 50GiB=62.3 min, 100GiB=124.7 min, fallback=0, dropped=15

