use sc_transport_core::HttpSseTransport;
use sc_transport_core::Transport;
use sc_transport_quic::QuicStreamTransport;

#[tokio::test]
async fn sse_is_always_available_fallback_target() {
    let sse = HttpSseTransport::new();
    let quic = QuicStreamTransport::new();

    assert_eq!(sse.name(), "sse");
    assert_eq!(quic.name(), "quic-stream");
    assert!(!sse.supports_unreliable());
    assert!(!quic.supports_unreliable());
}
