use std::io::{self, Write};

use super::LimitedWriter;

#[test]
fn exact_limit_is_accepted_and_overflow_never_reaches_the_sink() {
    let mut bytes = Vec::new();
    let mut writer = LimitedWriter::new(&mut bytes, 4);
    writer.write_all(b"abcd").unwrap();
    assert_eq!(writer.write(b"e").unwrap_err().kind(), io::ErrorKind::InvalidData);
    assert_eq!(bytes, b"abcd");
}

#[test]
fn oversized_first_write_is_refused_without_allocating_a_payload() {
    let mut bytes = Vec::new();
    assert!(LimitedWriter::new(&mut bytes, 1).write_all(b"ab").is_err());
    assert_eq!(bytes.capacity(), 0);
}

#[test]
fn partial_writes_charge_only_the_bytes_actually_written() {
    let mut storage = [0; 2];
    let mut writer = LimitedWriter::new(&mut storage[..], 4);
    assert_eq!(writer.write(b"abcd").unwrap(), 2);
    assert_eq!(writer.write(b"ef").unwrap(), 0);
    assert_eq!(writer.write(b"efg").unwrap_err().kind(), io::ErrorKind::InvalidData);
}
