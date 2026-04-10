use sct_proto::CompressionType;

pub fn maybe_compress(input: &[u8], mode: &CompressionType) -> anyhow::Result<Vec<u8>> {
    match mode {
        CompressionType::None => Ok(input.to_vec()),
        CompressionType::Zstd { level } => Ok(zstd::stream::encode_all(input, *level)?),
    }
}

pub fn maybe_decompress(input: &[u8], mode: &CompressionType) -> anyhow::Result<Vec<u8>> {
    match mode {
        CompressionType::None => Ok(input.to_vec()),
        CompressionType::Zstd { .. } => Ok(zstd::stream::decode_all(input)?),
    }
}
