use std::collections::HashSet;

#[test]
fn delta_skips_matching_chunk_and_keeps_changed() {
    // Simuliert: 3 Chunks, Chunk 1 identisch, Chunks 0 und 2 verändert.
    let chunk_size = 64usize;
    let data = vec![0xABu8; chunk_size * 3];

    // Receiver-Hashes: Chunk 0 = null (fehlt), Chunk 1 = korrekt, Chunk 2 = falsch
    let mut receiver_hashes: Vec<[u8; 32]> = vec![[0u8; 32]; 3];
    // Chunk 1: identisch mit Sender
    receiver_hashes[1] = *blake3::hash(&data[chunk_size..chunk_size * 2]).as_bytes();
    // Chunk 2: alter, falscher Hash
    receiver_hashes[2] = [0xFF; 32];

    let mut skip: HashSet<u64> = HashSet::new();
    for idx in 0u64..3 {
        if let Some(&rh) = receiver_hashes.get(idx as usize) {
            if rh == [0u8; 32] {
                continue;
            }
            let off = idx as usize * chunk_size;
            let end = off + chunk_size;
            let lh = *blake3::hash(&data[off..end]).as_bytes();
            if lh == rh {
                skip.insert(idx);
            }
        }
    }

    assert!(
        skip.contains(&1),
        "Chunk 1 (identisch) muss übersprungen werden"
    );
    assert!(
        !skip.contains(&0),
        "Chunk 0 (fehlt) darf nicht übersprungen werden"
    );
    assert!(
        !skip.contains(&2),
        "Chunk 2 (verändert) darf nicht übersprungen werden"
    );
}
