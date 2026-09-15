use super::*;

#[test]
fn memory_sizes_are_exact_and_bounded() {
    for (text, memory, mb) in [
        ("8G", Memory::Gigabytes(8), 8192),
        ("512m", Memory::Megabytes(512), 512),
        ("1g", Memory::Gigabytes(1), 1024),
    ] {
        let parsed: Memory = text.parse().unwrap();
        assert_eq!(parsed, memory);
        assert_eq!(parsed.megabytes().unwrap(), mb);
    }
    for text in [
        "",
        "G",
        "0G",
        "01G",
        "1.5G",
        "-1G",
        "+1G",
        "8GB",
        "8",
        "8Ｔ",
        "18446744073709551616M",
        "18446744073709551615G",
    ] {
        assert!(text.parse::<Memory>().is_err(), "{text}");
    }
    assert!(Memory::Megabytes(0).megabytes().is_err());
    assert!(Memory::Gigabytes(u64::MAX).megabytes().is_err());
}
