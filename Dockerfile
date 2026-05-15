# Build: docker build -t sct-daemon:latest .
# Run:  docker compose up (see docker-compose.yml)

FROM rust:1-bookworm AS builder
WORKDIR /src
COPY . .
RUN cargo build --release -p sct-daemon

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=builder /src/target/release/sct-daemon /usr/local/bin/sct-daemon
WORKDIR /etc/sct
EXPOSE 7272 7273 9090
ENTRYPOINT ["sct-daemon"]
