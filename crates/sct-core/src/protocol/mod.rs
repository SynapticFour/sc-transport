use anyhow::{bail, Result};
use bincode::config::standard;
use bincode::serde::{decode_from_slice, encode_to_vec};
use serde::{de::DeserializeOwned, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub use sct_proto::{
    ChunkDescriptor, CompressionType, FinalAck, ManifestAck, ReceiverFeedbackFrame,
    TransferComplete, TransferManifest,
};

/// Control-plane frames (manifest, acks, feedback). Chunk hashes for a 20 GiB / 4 MiB
/// transfer are ~160 KiB; 64 MiB leaves headroom without allowing a 4 GiB alloc DoS.
pub const MAX_CONTROL_FRAME_BYTES: usize = 64 * 1024 * 1024;
/// ChunkDescriptor bincode is tiny; 1 MiB is already a hostile peer.
pub const MAX_CHUNK_DESCRIPTOR_BYTES: usize = 1024 * 1024;

pub fn encode<T: Serialize>(msg: &T) -> Result<Vec<u8>> {
    Ok(encode_to_vec(msg, standard())?)
}

pub fn decode<T: DeserializeOwned>(buf: &[u8]) -> Result<T> {
    let (msg, _): (T, usize) = decode_from_slice(buf, standard())?;
    Ok(msg)
}

pub fn check_frame_len(len: usize, max: usize) -> Result<()> {
    if len == 0 || len > max {
        bail!("frame length {len} outside 1..={max}");
    }
    Ok(())
}

pub async fn write_framed<T: Serialize, W: AsyncWrite + Unpin>(
    writer: &mut W,
    msg: &T,
) -> Result<()> {
    let payload = encode(msg)?;
    check_frame_len(payload.len(), MAX_CONTROL_FRAME_BYTES)?;
    writer.write_u32(payload.len() as u32).await?;
    writer.write_all(&payload).await?;
    writer.flush().await?;
    Ok(())
}

pub async fn read_framed<T: DeserializeOwned, R: AsyncRead + Unpin>(reader: &mut R) -> Result<T> {
    let len = reader.read_u32().await? as usize;
    check_frame_len(len, MAX_CONTROL_FRAME_BYTES)?;
    let mut payload = vec![0_u8; len];
    reader.read_exact(&mut payload).await?;
    decode(&payload)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sct_proto::ManifestAck;

    #[tokio::test]
    async fn roundtrips_framed_message() {
        let (mut a, mut b) = tokio::io::duplex(1024);
        let msg = ManifestAck {
            accepted: true,
            message: Some("ok".to_string()),
            received_chunks: vec![0, 4],
            chunk_hashes: vec![],
        };
        write_framed(&mut a, &msg).await.expect("write");
        let got: ManifestAck = read_framed(&mut b).await.expect("read");
        assert!(got.accepted);
        assert_eq!(got.message.as_deref(), Some("ok"));
        assert_eq!(got.received_chunks, vec![0, 4]);
        assert!(got.chunk_hashes.is_empty());
    }
}
