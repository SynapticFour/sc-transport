# Competitive Baseline Matrix

- baseline repeats: 5
- compared transports: sc transports + raw TCP + raw UDP baselines

## excellent / medium
- baseline-udp: p50=119850.19 eps, 50GiB=0.1 min, 100GiB=0.2 min, fallback=0, dropped=0
- quic-datagram: p50=72987.11 eps, 50GiB=0.2 min, 100GiB=0.4 min, fallback=199, dropped=1
- sse: p50=43501.90 eps, 50GiB=0.3 min, 100GiB=0.6 min, fallback=0, dropped=0
- baseline-tcp: p50=27973.67 eps, 50GiB=0.5 min, 100GiB=1.0 min, fallback=0, dropped=0
- quic-stream: p50=1382.62 eps, 50GiB=9.9 min, 100GiB=19.7 min, fallback=0, dropped=0

## excellent / large
- baseline-udp: p50=126050.16 eps, 50GiB=0.0 min, 100GiB=0.1 min, fallback=0, dropped=0
- sse: p50=50568.90 eps, 50GiB=0.1 min, 100GiB=0.1 min, fallback=0, dropped=0
- quic-datagram: p50=36836.65 eps, 50GiB=0.1 min, 100GiB=0.2 min, fallback=200, dropped=0
- baseline-tcp: p50=15533.47 eps, 50GiB=0.2 min, 100GiB=0.4 min, fallback=0, dropped=0
- quic-stream: p50=234.62 eps, 50GiB=14.5 min, 100GiB=29.1 min, fallback=0, dropped=0

## good / medium
- baseline-udp: p50=531.28 eps, 50GiB=25.7 min, 100GiB=51.4 min, fallback=0, dropped=0
- baseline-tcp: p50=529.74 eps, 50GiB=25.8 min, 100GiB=51.5 min, fallback=0, dropped=0
- quic-stream: p50=363.59 eps, 50GiB=37.6 min, 100GiB=75.1 min, fallback=0, dropped=0
- sse: p50=354.75 eps, 50GiB=38.5 min, 100GiB=77.0 min, fallback=0, dropped=0
- quic-datagram: p50=354.73 eps, 50GiB=38.5 min, 100GiB=77.0 min, fallback=199, dropped=1

## good / large
- baseline-udp: p50=529.60 eps, 50GiB=6.4 min, 100GiB=12.9 min, fallback=0, dropped=0
- baseline-tcp: p50=524.51 eps, 50GiB=6.5 min, 100GiB=13.0 min, fallback=0, dropped=0
- sse: p50=355.32 eps, 50GiB=9.6 min, 100GiB=19.2 min, fallback=0, dropped=0
- quic-datagram: p50=353.62 eps, 50GiB=9.7 min, 100GiB=19.3 min, fallback=199, dropped=1
- quic-stream: p50=241.56 eps, 50GiB=14.1 min, 100GiB=28.3 min, fallback=0, dropped=0

## poor / medium
- baseline-tcp: p50=126.57 eps, 50GiB=107.9 min, 100GiB=215.7 min, fallback=0, dropped=0
- quic-stream: p50=120.91 eps, 50GiB=112.9 min, 100GiB=225.8 min, fallback=0, dropped=0
- sse: p50=120.10 eps, 50GiB=113.7 min, 100GiB=227.4 min, fallback=0, dropped=0
- quic-datagram: p50=119.72 eps, 50GiB=114.0 min, 100GiB=228.1 min, fallback=188, dropped=12
- baseline-udp: p50=119.13 eps, 50GiB=114.6 min, 100GiB=229.2 min, fallback=0, dropped=5

## poor / large
- baseline-tcp: p50=125.15 eps, 50GiB=27.3 min, 100GiB=54.5 min, fallback=0, dropped=0
- quic-stream: p50=124.90 eps, 50GiB=27.3 min, 100GiB=54.7 min, fallback=0, dropped=0
- baseline-udp: p50=120.59 eps, 50GiB=28.3 min, 100GiB=56.6 min, fallback=0, dropped=5
- quic-datagram: p50=120.42 eps, 50GiB=28.3 min, 100GiB=56.7 min, fallback=188, dropped=12
- sse: p50=119.77 eps, 50GiB=28.5 min, 100GiB=57.0 min, fallback=0, dropped=0

## very-poor / medium
- quic-stream: p50=68.25 eps, 50GiB=200.1 min, 100GiB=400.1 min, fallback=0, dropped=0
- sse: p50=67.15 eps, 50GiB=203.3 min, 100GiB=406.6 min, fallback=0, dropped=0
- quic-datagram: p50=66.91 eps, 50GiB=204.1 min, 100GiB=408.1 min, fallback=200, dropped=0
- baseline-tcp: p50=65.22 eps, 50GiB=209.3 min, 100GiB=418.7 min, fallback=0, dropped=0
- baseline-udp: p50=55.55 eps, 50GiB=245.8 min, 100GiB=491.5 min, fallback=0, dropped=15

## very-poor / large
- quic-stream: p50=67.13 eps, 50GiB=50.9 min, 100GiB=101.7 min, fallback=0, dropped=0
- sse: p50=67.02 eps, 50GiB=50.9 min, 100GiB=101.9 min, fallback=0, dropped=0
- quic-datagram: p50=66.95 eps, 50GiB=51.0 min, 100GiB=102.0 min, fallback=200, dropped=0
- baseline-tcp: p50=64.53 eps, 50GiB=52.9 min, 100GiB=105.8 min, fallback=0, dropped=0
- baseline-udp: p50=55.13 eps, 50GiB=61.9 min, 100GiB=123.8 min, fallback=0, dropped=15

