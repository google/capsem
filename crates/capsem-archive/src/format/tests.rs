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

#[test]
fn file_header_round_trips_and_names_version_two() {
    let header = encode_file_header();
    assert_eq!(&header[..8], b"CAPSEMBL");
    assert_eq!(u16::from_le_bytes([header[8], header[9]]), 2);
    decode_file_header(&header).expect("own header accepted");
}

#[test]
fn a_version_one_archive_is_refused() {
    let mut header = encode_file_header();
    header[8..10].copy_from_slice(&1u16.to_le_bytes());
    assert!(matches!(decode_file_header(&header), Err(ArchiveError::BadFileHeader)));
}

#[test]
fn another_magic_or_a_short_file_header_is_refused() {
    let mut header = encode_file_header();
    header[0] = b'X';
    assert!(matches!(decode_file_header(&header), Err(ArchiveError::BadFileHeader)));
    let header = encode_file_header();
    assert!(matches!(
        decode_file_header(&header[..FILE_HEADER_BYTES - 1]),
        Err(ArchiveError::BadFileHeader)
    ));
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
