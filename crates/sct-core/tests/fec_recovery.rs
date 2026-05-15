//! RS recovery: sender `encode_block` shards must match receiver `try_fec_group_recover`.

use reed_solomon_erasure::galois_8::ReedSolomon;
use sct_core::adaptive::{FecEncoder, Packet, PacketId, PacketMeta};
use sct_core::protocol::{encode, ChunkDescriptor, CompressionType};
use sct_proto::ChecksumAlg;

const CHUNK_SIZE: usize = 64 * 1024;

/// Matches [`FileSender::build_chunk_payload_at`] field layout (offset, fec_group, checksum).
fn framed_chunk_sender_style(
    index: u64,
    body: &[u8],
    chunk_size: usize,
    data_shards: usize,
) -> Vec<u8> {
    let off = index as usize * chunk_size;
    let desc = ChunkDescriptor {
        index,
        offset: off as u64,
        compressed_size: body.len() as u32,
        uncompressed_size: body.len() as u32,
        checksum: *blake3::hash(body).as_bytes(),
        was_compressed: false,
        is_parity: false,
        parity_index: 0,
        fec_group: index / data_shards.max(1) as u64,
    };
    let desc_bytes = encode(&desc).expect("desc");
    let mut out = Vec::with_capacity(4 + desc_bytes.len() + body.len());
    out.extend_from_slice(&(desc_bytes.len() as u32).to_be_bytes());
    out.extend_from_slice(&desc_bytes);
    out.extend_from_slice(body);
    out
}

fn framed_chunk(index: u64, body: &[u8]) -> Vec<u8> {
    framed_chunk_sender_style(index, body, 64, 4)
}

#[test]
fn rs_recovers_missing_framed_data_shard_like_receiver() {
    let data_shards = 2usize;
    let parity_shards = 1usize;
    let enc = FecEncoder {
        data_shards,
        parity_shards,
    };
    let p0 = Packet {
        id: PacketId(0),
        seq: 0,
        payload: framed_chunk(0, b"chunk-zero-bytes!!"),
        is_parity: false,
        meta: PacketMeta {
            id: 0,
            priority: 1,
            deadline: None,
            size: 0,
        },
        fec_group: 0,
        reconstructable: false,
        parity_index: 0,
    };
    let p1 = Packet {
        id: PacketId(1),
        seq: 1,
        payload: framed_chunk(1, b"chunk-one-bytes!!!"),
        is_parity: false,
        meta: PacketMeta {
            id: 1,
            priority: 1,
            deadline: None,
            size: 0,
        },
        fec_group: 0,
        reconstructable: false,
        parity_index: 0,
    };
    let encoded = enc.encode_block(&[p0.clone(), p1.clone()]);
    assert_eq!(encoded.len(), 3);
    assert!(encoded[2].is_parity);

    // Receiver layout: data slots = full framed payloads; parity slot = raw RS row only.
    let parity_pkt = &encoded[2];
    let parity_desc_len = u32::from_be_bytes(parity_pkt.payload[0..4].try_into().unwrap()) as usize;
    let parity_desc_bytes = &parity_pkt.payload[4..4 + parity_desc_len];
    let parity_body = &parity_pkt.payload[4 + parity_desc_len..];
    let parity_desc: ChunkDescriptor =
        sct_core::protocol::decode(parity_desc_bytes).expect("parity desc");

    let group: Vec<Option<Vec<u8>>> =
        vec![Some(p0.payload.clone()), None, Some(parity_body.to_vec())];

    let rs = ReedSolomon::new(data_shards, parity_shards).expect("rs");
    let max_len = group.iter().flatten().map(|v| v.len()).max().unwrap_or(0);
    let mut shards: Vec<Option<Vec<u8>>> = group
        .iter()
        .map(|opt| {
            opt.as_ref().map(|v| {
                let mut x = v.clone();
                x.resize(max_len, 0);
                x
            })
        })
        .collect();
    rs.reconstruct_data(&mut shards).expect("reconstruct_data");
    let recovered = shards[1].clone().expect("shard 1");
    assert_eq!(recovered, p1.payload);
    let _ = (parity_desc, ChecksumAlg::Blake3, CompressionType::None);
}

#[test]
fn rs_recovers_with_four_data_two_parity() {
    let data_shards = 4usize;
    let parity_shards = 2usize;
    let enc = FecEncoder {
        data_shards,
        parity_shards,
    };
    let packets: Vec<Packet> = (0..data_shards)
        .map(|i| {
            let i = i as u64;
            let body: Vec<u8> = (0..CHUNK_SIZE)
                .map(|b| ((i as usize + b) % 251) as u8)
                .collect();
            Packet {
                id: PacketId(i),
                seq: i,
                payload: framed_chunk_sender_style(i, &body, CHUNK_SIZE, data_shards),
                is_parity: false,
                meta: PacketMeta {
                    id: i,
                    priority: 1,
                    deadline: None,
                    size: 0,
                },
                fec_group: 0,
                reconstructable: false,
                parity_index: 0,
            }
        })
        .collect();
    let encoded = enc.encode_block(&packets);
    assert_eq!(encoded.len(), data_shards + parity_shards);

    let mut group: Vec<Option<Vec<u8>>> = (0..data_shards + parity_shards).map(|_| None).collect();
    group[0] = Some(packets[0].payload.clone());
    group[2] = Some(packets[2].payload.clone());
    group[3] = Some(packets[3].payload.clone());
    for (pi, pkt) in encoded[data_shards..].iter().enumerate() {
        let desc_len = u32::from_be_bytes(pkt.payload[0..4].try_into().unwrap()) as usize;
        let body = pkt.payload[4 + desc_len..].to_vec();
        group[data_shards + pi] = Some(body);
    }

    let rs = ReedSolomon::new(data_shards, parity_shards).expect("rs");
    let max_len = group.iter().flatten().map(|v| v.len()).max().unwrap_or(0);
    let mut shards: Vec<Option<Vec<u8>>> = group
        .iter()
        .map(|opt| {
            opt.as_ref().map(|v| {
                let mut x = v.clone();
                x.resize(max_len, 0);
                x
            })
        })
        .collect();
    rs.reconstruct_data(&mut shards).expect("reconstruct_data");
    assert_eq!(shards[1].as_ref().unwrap(), &packets[1].payload);
}

/// `data_shards=2`, `parity_shards=2` — chunk 1 missing; encode input reversed like the scheduler.
#[test]
fn rs_recovers_two_data_two_parity_missing_shard_one() {
    let data_shards = 2usize;
    let parity_shards = 2usize;
    let enc = FecEncoder {
        data_shards,
        parity_shards,
    };
    let packets: Vec<Packet> = (0..data_shards)
        .map(|i| {
            let i = i as u64;
            let body: Vec<u8> = (0..CHUNK_SIZE)
                .map(|b| ((i as usize + b) % 251) as u8)
                .collect();
            Packet {
                id: PacketId(i),
                seq: i,
                payload: framed_chunk_sender_style(i, &body, CHUNK_SIZE, data_shards),
                is_parity: false,
                meta: PacketMeta {
                    id: i,
                    priority: 1,
                    deadline: None,
                    size: 0,
                },
                fec_group: 0,
                reconstructable: false,
                parity_index: 0,
            }
        })
        .collect();
    let encoded = enc.encode_block(&[packets[1].clone(), packets[0].clone()]);
    let mut group: Vec<Option<Vec<u8>>> = (0..data_shards + parity_shards).map(|_| None).collect();
    for (pi, pkt) in encoded[data_shards..].iter().enumerate() {
        let desc_len = u32::from_be_bytes(pkt.payload[0..4].try_into().unwrap()) as usize;
        group[data_shards + pi] = Some(pkt.payload[4 + desc_len..].to_vec());
    }
    group[0] = Some(packets[0].payload.clone());
    let rs = ReedSolomon::new(data_shards, parity_shards).expect("rs");
    let max_len = group.iter().flatten().map(|v| v.len()).max().unwrap_or(0);
    let mut shards: Vec<Option<Vec<u8>>> = group
        .iter()
        .map(|opt| {
            opt.as_ref().map(|v| {
                let mut x = v.clone();
                x.resize(max_len, 0);
                x
            })
        })
        .collect();
    rs.reconstruct_data(&mut shards).expect("reconstruct_data");
    assert_eq!(shards[1].as_ref().unwrap(), &packets[1].payload);
}

/// Out-of-order arrival (0,2,3 + both parity) before shard 1 — mirrors production `data_shards=4`.
#[test]
fn rs_recovers_out_of_order_four_data_two_parity() {
    let data_shards = 4usize;
    let parity_shards = 2usize;
    let enc = FecEncoder {
        data_shards,
        parity_shards,
    };
    let packets: Vec<Packet> = (0..data_shards)
        .map(|i| {
            let i = i as u64;
            let body: Vec<u8> = (0..CHUNK_SIZE)
                .map(|b| ((i as usize + b) % 251) as u8)
                .collect();
            Packet {
                id: PacketId(i),
                seq: i,
                payload: framed_chunk_sender_style(i, &body, CHUNK_SIZE, data_shards),
                is_parity: false,
                meta: PacketMeta {
                    id: i,
                    priority: 1,
                    deadline: None,
                    size: 0,
                },
                fec_group: 0,
                reconstructable: false,
                parity_index: 0,
            }
        })
        .collect();
    let encoded = enc.encode_block(&packets);

    // Simulate streams: chunks 0, 2, 3, then parity — shard 1 still missing.
    let mut group: Vec<Option<Vec<u8>>> = (0..data_shards + parity_shards).map(|_| None).collect();
    group[0] = Some(packets[0].payload.clone());
    group[2] = Some(packets[2].payload.clone());
    group[3] = Some(packets[3].payload.clone());
    for (pi, pkt) in encoded[data_shards..].iter().enumerate() {
        let desc_len = u32::from_be_bytes(pkt.payload[0..4].try_into().unwrap()) as usize;
        group[data_shards + pi] = Some(pkt.payload[4 + desc_len..].to_vec());
    }

    let rs = ReedSolomon::new(data_shards, parity_shards).expect("rs");
    let max_len = group.iter().flatten().map(|v| v.len()).max().unwrap_or(0);
    let mut shards: Vec<Option<Vec<u8>>> = group
        .iter()
        .map(|opt| {
            opt.as_ref().map(|v| {
                let mut x = v.clone();
                x.resize(max_len, 0);
                x
            })
        })
        .collect();
    rs.reconstruct_data(&mut shards).expect("reconstruct_data");
    assert_eq!(shards[1].as_ref().unwrap(), &packets[1].payload);
}
