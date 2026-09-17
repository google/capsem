use super::*;
use crate::ArchiveError;

const AT: u64 = FILE_HEADER_BYTES as u64;

fn roundtrip(raw: &[u8]) -> Vec<u8> {
    let block = encode_block(raw);
    let (header, comp) = split_block(&block, AT).expect("split");
    decode_block(&header, comp, AT).expect("decode")
}

#[test]
fn file_header_round_trips() {
    let header = encode_file_header();
    assert_eq!(header.len(), FILE_HEADER_BYTES);
    decode_file_header(&header).expect("own header accepted");
}

#[test]
fn a_header_with_another_magic_is_rejected() {
    let mut header = encode_file_header();
    header[0] = b'X';
    assert!(matches!(decode_file_header(&header), Err(ArchiveError::BadFileHeader)));
}

#[test]
fn a_header_with_another_version_is_rejected() {
    let mut header = encode_file_header();
    header[8..10].copy_from_slice(&(FILE_VERSION + 1).to_le_bytes());
    assert!(matches!(decode_file_header(&header), Err(ArchiveError::BadFileHeader)));
}

#[test]
fn a_short_header_is_rejected() {
    let header = encode_file_header();
    assert!(matches!(
        decode_file_header(&header[..FILE_HEADER_BYTES - 1]),
        Err(ArchiveError::BadFileHeader)
    ));
}

#[test]
fn a_block_round_trips_and_deflate_shrinks_repetition() {
    let raw = b"the same sentence, over and over. ".repeat(500);
    let block = encode_block(&raw);
    assert!(
        block.len() < raw.len() / 4,
        "repetitive input should deflate hard: {} -> {}",
        raw.len(),
        block.len()
    );
    assert_eq!(roundtrip(&raw), raw);
}

#[test]
fn an_empty_block_round_trips() {
    assert_eq!(roundtrip(b""), Vec::<u8>::new());
}

#[test]
fn a_tampered_last_byte_is_rejected() {
    let raw = b"bodies that must come back exactly as written".repeat(20);
    let mut block = encode_block(&raw);
    let last = block.len() - 1;
    block[last] ^= 0xff;
    let (header, comp) = split_block(&block, AT).expect("split");
    let error = decode_block(&header, comp, AT).expect_err("tampered payload rejected");
    assert!(
        matches!(error, ArchiveError::Integrity(AT) | ArchiveError::Inflate(AT, _)),
        "unexpected error: {error:?}"
    );
}

/// Deflate carries no checksum of its own here (raw deflate, not zlib), so
/// the blake3 field is the only thing standing between an edited payload and
/// a caller. A wrong hash must be refused even though the block inflates
/// perfectly.
#[test]
fn a_block_whose_hash_does_not_match_is_rejected() {
    let raw = b"bytes that must not be returned unverified".repeat(10);
    let mut block = encode_block(&raw);
    block[12] ^= 0x01;
    let (header, comp) = split_block(&block, AT).expect("split");
    assert!(matches!(
        decode_block(&header, comp, AT),
        Err(ArchiveError::Integrity(AT))
    ));
}

#[test]
fn an_absurd_raw_len_is_rejected_before_allocating() {
    let mut block = encode_block(b"small");
    block[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(matches!(split_block(&block, AT), Err(ArchiveError::BadBlockHeader(AT))));
    let header: &[u8; BLOCK_HEADER_BYTES] = block[..BLOCK_HEADER_BYTES].try_into().unwrap();
    assert!(matches!(
        parse_block_header(header, AT),
        Err(ArchiveError::BadBlockHeader(AT))
    ));
}

#[test]
fn an_absurd_comp_len_is_rejected_before_allocating() {
    let mut block = encode_block(b"small");
    block[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(matches!(split_block(&block, AT), Err(ArchiveError::BadBlockHeader(AT))));
    let header: &[u8; BLOCK_HEADER_BYTES] = block[..BLOCK_HEADER_BYTES].try_into().unwrap();
    assert!(matches!(
        parse_block_header(header, AT),
        Err(ArchiveError::BadBlockHeader(AT))
    ));
}

#[test]
fn a_comp_len_longer_than_the_slice_is_rejected() {
    let block = encode_block(b"a body worth a few bytes");
    let truncated = &block[..block.len() - 1];
    assert!(matches!(
        split_block(truncated, AT),
        Err(ArchiveError::BadBlockHeader(AT))
    ));
}

#[test]
fn another_block_magic_is_rejected() {
    let mut block = encode_block(b"body");
    block[0] = b'X';
    assert!(matches!(split_block(&block, AT), Err(ArchiveError::BadBlockHeader(AT))));
}

#[test]
fn a_block_shorter_than_its_header_is_rejected() {
    let block = encode_block(b"body");
    assert!(matches!(
        split_block(&block[..BLOCK_HEADER_BYTES - 1], AT),
        Err(ArchiveError::BadBlockHeader(AT))
    ));
}
