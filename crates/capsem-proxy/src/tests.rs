use super::*;

#[test]
fn generation_parser_requires_canonical_nonzero_hex() {
    assert_eq!(
        parse_generation("07070707070707070707070707070707").unwrap().as_bytes(),
        [7; 16]
    );
    for invalid in [
        "",
        "00000000000000000000000000000000",
        "0707070707070707070707070707070",
        "070707070707070707070707070707070",
        "0707070707070707070707070707070G",
        "0707070707070707070707070707070A",
    ] {
        assert!(parse_generation(invalid).is_err(), "accepted {invalid}");
    }
}
