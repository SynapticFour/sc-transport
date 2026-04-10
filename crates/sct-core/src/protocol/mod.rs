use anyhow::Result;
use bincode::config::standard;
use bincode::serde::{decode_from_slice, encode_to_vec};
use serde::{de::DeserializeOwned, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub use sct_proto::{
    ChunkDescriptor, CompressionType, FinalAck, ManifestAck, TransferComplete, TransferManifest,
};

pub fn encode<T: Serialize>(msg: &T) -> Result<Vec<u8>> {
    Ok(encode_to_vec(msg, standard())?)
}

pub fn decode<T: DeserializeOwned>(buf: &[u8]) -> Result<T> {
    let (msg, _): (T, usize) = decode_from_slice(buf, standard())?;
    Ok(msg)
}

pub async fn write_framed<T: Serialize, W: AsyncWrite + Unpin>(writer: &mut W, msg: &T) -> Result<()> {
    let payload = encode(msg)?;
    writer.write_u32(payload.len() as u32).await?;
    writer.write_all(&payload).await?;
    writer.flush().await?;
    Ok(())
}

pub async fn read_framed<T: DeserializeOwned, R: AsyncRead + Unpin>(reader: &mut R) -> Result<T> {
    let len = reader.read_u32().await? as usize;
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
        };
        write_framed(&mut a, &msg).await.expect("write");
        let got: ManifestAck = read_framed(&mut b).await.expect("read");
        assert!(got.accepted);
        assert_eq!(got.message.as_deref(), Some("ok"));
        assert_eq!(got.received_chunks, vec![0, 4]);
    }
}
