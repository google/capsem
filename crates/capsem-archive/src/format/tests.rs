use super::*;
use crate::ArchiveError;

const AT: u64 = 4242;

fn segment(last: bool, raw_start: u32, raw_len: u32, comp_len: u32) -> SegmentHeader {
    SegmentHeader {
        last,
        raw_start,
        raw_len,
        comp_len,
        hash: [7; 32],
    }
}

fn is_bad_segment(result: Result<SegmentHeader>) -> bool {
    matches!(result, Err(ArchiveError::BadSegment(AT)))
}

const ARCHIVE_BYTES: [u8; 16] = [
    0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x46, 0x17, 0x98, 0x19, 0x1a, 0x1b, 0x1c, 0x1d, 0x1e, 0x1f,
];
const GENERATION_BYTES: [u8; 16] = [
    0x20, 0x21, 0x22, 0x23, 0x24, 0x25, 0x46, 0x27, 0xa8, 0x29, 0x2a, 0x2b, 0x2c, 0x2d, 0x2e, 0x2f,
];

fn identities() -> (ArchiveId, GenerationId) {
    (
        ArchiveId::from_bytes(ARCHIVE_BYTES).unwrap(),
        GenerationId::from_bytes(GENERATION_BYTES).unwrap(),
    )
}

#[test]
fn file_header_has_the_exact_v3_golden_bytes_and_round_trips() {
    let (archive_id, generation_id) = identities();
    let header = encode_file_header(archive_id, generation_id);
    assert_eq!(
        &header[..48],
        &[
            b'C', b'A', b'P', b'S', b'E', b'M', b'B', b'L', 3, 0, 0, 0, 80, 0, 0, 0, 0x10, 0x11, 0x12, 0x13, 0x14,
            0x15, 0x46, 0x17, 0x98, 0x19, 0x1a, 0x1b, 0x1c, 0x1d, 0x1e, 0x1f, 0x20, 0x21, 0x22, 0x23, 0x24, 0x25, 0x46,
            0x27, 0xa8, 0x29, 0x2a, 0x2b, 0x2c, 0x2d, 0x2e, 0x2f,
        ]
    );
    assert_eq!(
        &header[48..],
        &[
            0x82, 0x1a, 0xeb, 0x26, 0x07, 0x9e, 0x7e, 0x03, 0x97, 0x62, 0xbb, 0x20, 0xec, 0x05, 0x79, 0xc2, 0xc7, 0xf1,
            0x31, 0x80, 0xa1, 0x75, 0x41, 0xd0, 0xc2, 0xe8, 0x82, 0x32, 0xb4, 0xee, 0x12, 0xd8,
        ]
    );
    assert_eq!(
        decode_file_header(&header).unwrap(),
        FileHeader {
            archive_id,
            generation_id
        }
    );
}

#[test]
fn a_version_one_archive_is_refused() {
    let (archive_id, generation_id) = identities();
    let mut header = encode_file_header(archive_id, generation_id);
    header[8..10].copy_from_slice(&1u16.to_le_bytes());
    assert!(matches!(decode_file_header(&header), Err(ArchiveError::BadFileHeader)));
}

#[test]
fn another_magic_or_a_short_file_header_is_refused() {
    let (archive_id, generation_id) = identities();
    let mut header = encode_file_header(archive_id, generation_id);
    header[0] = b'X';
    assert!(matches!(decode_file_header(&header), Err(ArchiveError::BadFileHeader)));
    let header = encode_file_header(archive_id, generation_id);
    assert!(matches!(
        decode_file_header(&header[..FILE_HEADER_BYTES - 1]),
        Err(ArchiveError::BadFileHeader)
    ));
}

#[test]
fn file_header_rejects_flags_length_uuid_and_hash_damage() {
    let (archive_id, generation_id) = identities();
    let good = encode_file_header(archive_id, generation_id);
    for index in [10, 11, 12, 13, 14, 15, 16, 22, 24, 38, 40, 48, 79] {
        let mut damaged = good;
        damaged[index] ^= 0x01;
        assert!(
            matches!(decode_file_header(&damaged), Err(ArchiveError::BadFileHeader)),
            "byte {index}"
        );
    }
}

#[test]
fn generation_names_are_canonical_and_checked_against_the_header() {
    let (archive_id, generation_id) = identities();
    assert_eq!(generation_id.file_name(), "g-2021222324254627a8292a2b2c2d2e2f.cbl");
    assert_eq!(
        GenerationId::from_file_name(&generation_id.file_name()).unwrap(),
        generation_id
    );
    for bad in [
        "g-2021222324254627A8292a2b2c2d2e2f.cbl",
        "g-2021222324254627a8292a2b2c2d2e2.cbl",
        "x-2021222324254627a8292a2b2c2d2e2f.cbl",
        "g-2021222324254627a8292a2b2c2d2e2f.cbl/extra",
    ] {
        assert!(GenerationId::from_file_name(bad).is_err(), "{bad}");
    }
    let header = decode_file_header(&encode_file_header(archive_id, generation_id)).unwrap();
    header
        .validate(archive_id, generation_id, &generation_id.file_name())
        .unwrap();
    let other = GenerationId::new_v4();
    assert!(header.validate(archive_id, other, &other.file_name()).is_err());
}

#[test]
fn block_header_round_trips_its_codec() {
    let header = encode_block_header(CODEC_DEFLATE);
    assert_eq!(&header[..4], b"BLK2");
    assert_eq!(parse_block_header(&header, AT).unwrap(), CODEC_DEFLATE);
}

#[test]
fn an_unknown_codec_is_refused_by_name() {
    let header = encode_block_header(2);
    assert!(matches!(
        parse_block_header(&header, AT),
        Err(ArchiveError::UnsupportedCodec {
            block_offset: AT,
            codec: 2
        })
    ));
}

#[test]
fn block_flags_and_reserved_bytes_must_be_zero() {
    for index in 5..BLOCK_HEADER_BYTES {
        let mut header = encode_block_header(CODEC_DEFLATE);
        header[index] = 1;
        assert!(
            matches!(
                parse_block_header(&header, AT),
                Err(ArchiveError::UnsupportedCodec { .. })
            ),
            "byte {index} set"
        );
    }
}

#[test]
fn another_block_magic_is_a_bad_header() {
    let mut header = encode_block_header(CODEC_DEFLATE);
    header[3] = b'1';
    assert!(matches!(
        parse_block_header(&header, AT),
        Err(ArchiveError::BadBlockHeader(AT))
    ));
}

#[test]
fn segment_header_round_trips() {
    for header in [
        segment(false, 0, 10, 12),
        segment(true, 10, 0, 2),
        segment(true, 3, 5, 9),
    ] {
        let bytes = encode_segment_header(&header);
        assert_eq!(&bytes[..4], b"SGMT");
        assert_eq!(parse_segment_header(&bytes, AT, header.raw_start).unwrap(), header);
    }
}

#[test]
fn a_segment_must_continue_where_the_last_one_ended() {
    let bytes = encode_segment_header(&segment(false, 10, 5, 7));
    assert!(is_bad_segment(parse_segment_header(&bytes, AT, 9)));
    assert!(is_bad_segment(parse_segment_header(&bytes, AT, 11)));
}

#[test]
fn only_a_final_segment_may_be_empty() {
    let bytes = encode_segment_header(&segment(false, 0, 0, 5));
    assert!(is_bad_segment(parse_segment_header(&bytes, AT, 0)));
    let bytes = encode_segment_header(&segment(true, 0, 0, 2));
    parse_segment_header(&bytes, AT, 0).expect("an empty final segment ends a stream");
}

#[test]
fn a_raw_extent_past_the_block_ceiling_is_refused() {
    let at_ceiling = u32::try_from(MAX_BLOCK_RAW_BYTES).unwrap();
    let bytes = encode_segment_header(&segment(false, 0, at_ceiling, 10));
    parse_segment_header(&bytes, AT, 0).expect("exactly the ceiling fits");
    let bytes = encode_segment_header(&segment(false, 1, at_ceiling, 10));
    assert!(is_bad_segment(parse_segment_header(&bytes, AT, 1)));
    let bytes = encode_segment_header(&segment(false, 0, u32::MAX, 10));
    assert!(is_bad_segment(parse_segment_header(&bytes, AT, 0)));
}

#[test]
fn a_comp_len_beyond_deflate_expansion_is_refused_before_reading() {
    let limit = 100 + u32::try_from(MAX_SEGMENT_EXPANSION).unwrap();
    let bytes = encode_segment_header(&segment(false, 0, 100, limit));
    parse_segment_header(&bytes, AT, 0).expect("the expansion bound itself is accepted");
    let bytes = encode_segment_header(&segment(false, 0, 100, limit + 1));
    assert!(is_bad_segment(parse_segment_header(&bytes, AT, 0)));
    let bytes = encode_segment_header(&segment(false, 0, 100, u32::MAX));
    assert!(is_bad_segment(parse_segment_header(&bytes, AT, 0)));
}

#[test]
fn unknown_segment_flags_reserved_bytes_and_magic_are_refused() {
    let good = encode_segment_header(&segment(false, 0, 10, 12));
    for (index, value) in [(0, b'X'), (4, 0x02), (4, 0x80), (5, 1), (6, 1), (7, 1)] {
        let mut bytes = good;
        bytes[index] = value;
        assert!(
            is_bad_segment(parse_segment_header(&bytes, AT, 0)),
            "byte {index} = {value:#x}"
        );
    }
}
