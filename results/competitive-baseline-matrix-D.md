# Competitive Baseline Matrix

- baseline repeats: 5
- compared transports: sc transports + raw TCP + raw UDP baselines

## excellent / medium
- sse: p50=140105.08 eps, 50GiB=0.1 min, 100GiB=0.2 min, fallback=0, dropped=0
- baseline-udp: p50=118899.70 eps, 50GiB=0.1 min, 100GiB=0.2 min, fallback=0, dropped=0
- quic-datagram: p50=85898.29 eps, 50GiB=0.2 min, 100GiB=0.3 min, fallback=199, dropped=1
- baseline-tcp: p50=33533.64 eps, 50GiB=0.4 min, 100GiB=0.8 min, fallback=0, dropped=0
- quic-stream: p50=4123.07 eps, 50GiB=3.3 min, 100GiB=6.6 min, fallback=0, dropped=0

## excellent / large
- baseline-udp: p50=129345.19 eps, 50GiB=0.0 min, 100GiB=0.1 min, fallback=0, dropped=0
- quic-datagram: p50=49481.99 eps, 50GiB=0.1 min, 100GiB=0.1 min, fallback=200, dropped=0
- quic-stream: p50=18426.10 eps, 50GiB=0.2 min, 100GiB=0.4 min, fallback=0, dropped=0
- baseline-tcp: p50=17604.98 eps, 50GiB=0.2 min, 100GiB=0.4 min, fallback=0, dropped=0
- sse: p50=10711.66 eps, 50GiB=0.3 min, 100GiB=0.6 min, fallback=0, dropped=0

## good / medium
- baseline-tcp: p50=530.97 eps, 50GiB=25.7 min, 100GiB=51.4 min, fallback=0, dropped=0
- baseline-udp: p50=530.83 eps, 50GiB=25.7 min, 100GiB=51.4 min, fallback=0, dropped=0
- quic-stream: p50=377.89 eps, 50GiB=36.1 min, 100GiB=72.3 min, fallback=0, dropped=0
- sse: p50=355.80 eps, 50GiB=38.4 min, 100GiB=76.7 min, fallback=0, dropped=0
- quic-datagram: p50=353.22 eps, 50GiB=38.7 min, 100GiB=77.3 min, fallback=199, dropped=1

## good / large
- baseline-udp: p50=532.76 eps, 50GiB=6.4 min, 100GiB=12.8 min, fallback=0, dropped=0
- baseline-tcp: p50=524.65 eps, 50GiB=6.5 min, 100GiB=13.0 min, fallback=0, dropped=0
- sse: p50=352.97 eps, 50GiB=9.7 min, 100GiB=19.3 min, fallback=0, dropped=0
- quic-datagram: p50=350.15 eps, 50GiB=9.7 min, 100GiB=19.5 min, fallback=199, dropped=1
- quic-stream: p50=243.24 eps, 50GiB=14.0 min, 100GiB=28.1 min, fallback=0, dropped=0

## poor / medium
- baseline-tcp: p50=128.97 eps, 50GiB=105.9 min, 100GiB=211.7 min, fallback=0, dropped=0
- quic-stream: p50=122.03 eps, 50GiB=111.9 min, 100GiB=223.8 min, fallback=0, dropped=0
- baseline-udp: p50=120.69 eps, 50GiB=113.1 min, 100GiB=226.3 min, fallback=0, dropped=5
- quic-datagram: p50=120.33 eps, 50GiB=113.5 min, 100GiB=226.9 min, fallback=188, dropped=12
- sse: p50=119.91 eps, 50GiB=113.9 min, 100GiB=227.7 min, fallback=0, dropped=0

## poor / large
- quic-stream: p50=125.86 eps, 50GiB=27.1 min, 100GiB=54.2 min, fallback=0, dropped=0
- baseline-tcp: p50=125.03 eps, 50GiB=27.3 min, 100GiB=54.6 min, fallback=0, dropped=0
- quic-datagram: p50=120.07 eps, 50GiB=28.4 min, 100GiB=56.9 min, fallback=188, dropped=12
- baseline-udp: p50=120.05 eps, 50GiB=28.4 min, 100GiB=56.9 min, fallback=0, dropped=5
- sse: p50=119.99 eps, 50GiB=28.4 min, 100GiB=56.9 min, fallback=0, dropped=0

## very-poor / medium
- quic-stream: p50=68.01 eps, 50GiB=200.8 min, 100GiB=401.5 min, fallback=0, dropped=0
- quic-datagram: p50=67.12 eps, 50GiB=203.4 min, 100GiB=406.8 min, fallback=200, dropped=0
- sse: p50=67.08 eps, 50GiB=203.5 min, 100GiB=407.1 min, fallback=0, dropped=0
- baseline-tcp: p50=65.78 eps, 50GiB=207.6 min, 100GiB=415.1 min, fallback=0, dropped=0
- baseline-udp: p50=55.16 eps, 50GiB=247.5 min, 100GiB=495.1 min, fallback=0, dropped=15

## very-poor / large
- quic-stream: p50=67.44 eps, 50GiB=50.6 min, 100GiB=101.2 min, fallback=0, dropped=0
- sse: p50=67.16 eps, 50GiB=50.8 min, 100GiB=101.6 min, fallback=0, dropped=0
- quic-datagram: p50=66.97 eps, 50GiB=51.0 min, 100GiB=101.9 min, fallback=200, dropped=0
- baseline-tcp: p50=64.95 eps, 50GiB=52.6 min, 100GiB=105.1 min, fallback=0, dropped=0
- baseline-udp: p50=54.87 eps, 50GiB=62.2 min, 100GiB=124.4 min, fallback=0, dropped=15

