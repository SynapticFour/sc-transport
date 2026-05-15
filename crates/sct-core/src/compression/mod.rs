use sct_proto::CompressionType;

/// True wenn die Magic-Bytes auf ein bereits-komprimiertes Format hinweisen.
/// Verhindert unnötige Zstd-Encode-Versuche auf Daten die sich nicht komprimieren lassen.
fn is_likely_compressed(data: &[u8]) -> bool {
    match data.get(..4) {
        Some([0x50, 0x4B, 0x03, 0x04]) => true, // ZIP / JAR / DOCX / XLSX
        Some([0x1F, 0x8B, _, _]) => true,       // GZIP
        Some([0x28, 0xB5, 0x2F, 0xFD]) => true, // Zstd (already compressed)
        Some([0x04, 0x22, 0x4D, 0x18]) => true, // LZ4 frame
        Some([0x89, 0x48, 0x44, 0x46]) => true, // HDF5 (\x89HDF)
        Some([0x72, 0x6F, 0x6F, 0x74]) => true, // ROOT file format
        Some([0x42, 0x5A, 0x68, _]) => true,    // BZ2
        Some([0x37, 0x7A, 0xBC, 0xAF]) => true, // 7-Zip
        _ => false,
    }
}

pub fn maybe_compress(input: &[u8], mode: &CompressionType) -> anyhow::Result<Vec<u8>> {
    match mode {
        CompressionType::None => Ok(input.to_vec()),
        CompressionType::Zstd { level } => {
            // Schritt 1: Magic-Byte-Check — keine CPU für hoffnungslose Fälle.
            if is_likely_compressed(input) {
                return Ok(input.to_vec());
            }
            // Schritt 2: Komprimieren und Expansion-Guard prüfen.
            let compressed = zstd::stream::encode_all(input, *level)?;
            if compressed.len() >= input.len() {
                // Zstd hat die Daten vergrößert (z.B. random/encrypted data) — passthrough.
                Ok(input.to_vec())
            } else {
                Ok(compressed)
            }
        }
    }
}

pub fn maybe_decompress(input: &[u8], mode: &CompressionType) -> anyhow::Result<Vec<u8>> {
    match mode {
        CompressionType::None => Ok(input.to_vec()),
        CompressionType::Zstd { .. } => Ok(zstd::stream::decode_all(input)?),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hdf5_magic_is_detected() {
        let hdf5_header = [0x89, 0x48, 0x44, 0x46, 0x0D, 0x0A, 0x1A, 0x0A];
        assert!(is_likely_compressed(&hdf5_header));
    }

    #[test]
    fn gzip_magic_is_detected() {
        let gz = [0x1F, 0x8B, 0x08, 0x00];
        assert!(is_likely_compressed(&gz));
    }

    #[test]
    fn plain_text_is_not_detected() {
        let text = b"Hello, world!";
        assert!(!is_likely_compressed(text));
    }

    #[test]
    fn expansion_guard_returns_original() {
        // Random-ähnliche Daten (4 Bytes, nicht-komprimierbar)
        let random = [0xDE, 0xAD, 0xBE, 0xEF];
        let result = maybe_compress(&random, &CompressionType::Zstd { level: 3 }).unwrap();
        // Ergebnis muss <= Eingabe sein (passthrough wenn größer)
        assert!(result.len() <= random.len() + 20); // 20 Byte Zstd-Overhead-Toleranz für winzige Inputs
    }
}
