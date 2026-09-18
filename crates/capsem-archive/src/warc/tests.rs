use std::io::Read;

use super::*;

const DATE: &str = "2026-09-17T10:11:12Z";

fn record<'a>(body: &'a [u8], content_type: Option<&'a str>) -> WarcRecord<'a> {
    WarcRecord {
        record_id: "urn:capsem:0123456789ab:response",
        target_uri: "https://example.test/answer",
        date: DATE,
        content_type,
        truncated: false,
        body,
    }
}

/// Split a `.warc.gz` into its gzip members and inflate each one.
///
/// Member by member on purpose: this is what a tool seeking to one record
/// does, and a decoder that swallowed the whole stream would pass even if the
/// writer emitted a single member for every record.
fn members(mut bytes: &[u8]) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    while !bytes.is_empty() {
        let mut decoder = flate2::bufread::GzDecoder::new(bytes);
        let mut member = Vec::new();
        decoder.read_to_end(&mut member).expect("a complete gzip member");
        bytes = decoder.into_inner();
        out.push(member);
    }
    out
}

fn write_one(rec: &WarcRecord<'_>) -> Vec<u8> {
    let mut buffer = Vec::new();
    write_record(&mut buffer, rec).expect("write record");
    buffer
}

#[test]
fn a_record_is_one_gzip_member_with_the_standard_headers_in_order() {
    let body = br#"{"answer":"yes"}"#;
    let bytes = write_one(&record(body, Some("application/json")));

    let members = members(&bytes);
    assert_eq!(members.len(), 1, "one record is one gzip member");
    let text = String::from_utf8(members[0].clone()).expect("this record's bytes are text");
    let expected = format!(
        "WARC/1.1\r\n\
         WARC-Type: resource\r\n\
         WARC-Record-ID: <urn:capsem:0123456789ab:response>\r\n\
         WARC-Target-URI: https://example.test/answer\r\n\
         WARC-Date: 2026-09-17T10:11:12Z\r\n\
         WARC-Block-Digest: blake3:{digest}\r\n\
         Content-Type: application/json\r\n\
         Content-Length: 16\r\n\
         \r\n\
         {{\"answer\":\"yes\"}}\r\n\r\n",
        digest = blake3::hash(body).to_hex()
    );
    assert_eq!(
        text, expected,
        "the record must be the exact header block, the body, and the two CRLFs that end it"
    );
}

#[test]
fn the_block_digest_is_blake3_of_the_body() {
    let body = b"digest me";
    let text = String::from_utf8(members(&write_one(&record(body, None)))[0].clone()).expect("text");
    assert!(
        text.contains(&format!(
            "WARC-Block-Digest: blake3:{}\r\n",
            blake3::hash(body).to_hex()
        )),
        "{text}"
    );
}

#[test]
fn a_body_with_no_content_type_is_declared_as_octet_stream() {
    let text = String::from_utf8(members(&write_one(&record(b"anything", None)))[0].clone()).expect("text");
    assert!(text.contains("Content-Type: application/octet-stream\r\n"), "{text}");
}

#[test]
fn an_empty_body_is_a_zero_length_record_that_still_round_trips() {
    let bytes = write_one(&record(b"", Some("text/plain")));
    let text = String::from_utf8(members(&bytes)[0].clone()).expect("text");
    assert!(text.contains("Content-Length: 0\r\n"), "{text}");
    assert!(
        text.ends_with("Content-Length: 0\r\n\r\n\r\n\r\n"),
        "an empty block is still followed by the record terminator: {text:?}"
    );
}

/// The block is length-delimited, not sentinel-delimited. A body that contains
/// CRLFs, or the literal bytes of a WARC version line, is transported intact --
/// a writer that escaped or a reader that scanned for a sentinel would both
/// hand back something other than what the session captured.
#[test]
fn a_body_is_transported_intact_whatever_it_looks_like() {
    for body in [
        b"line one\r\nline two\r\n\r\nline three".to_vec(),
        b"WARC/1.1\r\nWARC-Type: revisit\r\nContent-Length: 0\r\n\r\n".to_vec(),
        vec![0u8, 255, 13, 10, 13, 10],
    ] {
        let member = members(&write_one(&record(&body, Some("application/octet-stream"))))
            .pop()
            .expect("one member");
        let header_end = member
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .expect("a blank line ends the headers")
            + 4;
        let block = &member[header_end..member.len() - 4];
        assert_eq!(block, body.as_slice(), "the body must survive byte for byte");

        let header = String::from_utf8(member[..header_end].to_vec()).expect("headers are text");
        assert!(
            header.contains(&format!("Content-Length: {}\r\n", body.len())),
            "the length must describe the whole body: {header}"
        );
    }
}

#[test]
fn a_truncated_body_carries_the_specs_own_field_and_its_real_length() {
    let body = b"the first kilobyte of something longer";
    let mut rec = record(body, Some("text/plain"));
    rec.truncated = true;
    let text = String::from_utf8(members(&write_one(&rec))[0].clone()).expect("text");
    assert!(text.contains("WARC-Truncated: length\r\n"), "{text}");
    assert!(
        text.contains(&format!("Content-Length: {}\r\n", body.len())),
        "the length must be what was written, not what was cut: {text}"
    );
}

#[test]
fn two_records_concatenate_into_one_stream_of_two_members() {
    let mut stream = Vec::new();
    write_record(&mut stream, &record(b"first", Some("text/plain"))).expect("first");
    let mut second = record(b"second", Some("text/plain"));
    second.record_id = "urn:capsem:0123456789ac:request";
    write_record(&mut stream, &second).expect("second");

    let members = members(&stream);
    assert_eq!(members.len(), 2, "each record is its own seekable member");
    assert!(members[0].ends_with(b"first\r\n\r\n"), "first member");
    assert!(members[1].ends_with(b"second\r\n\r\n"), "second member");
    assert!(
        String::from_utf8_lossy(&members[1]).contains("urn:capsem:0123456789ac:request"),
        "the second member carries the second record's id"
    );
}

/// Header injection. A value carrying a line break would end the field early
/// and let the rest of it forge headers of its own, so it is refused by name.
#[test]
fn a_line_break_in_a_header_value_is_refused_by_field_name() {
    let cases: [(&str, WarcRecord<'_>); 4] = [
        (
            "content_type",
            WarcRecord {
                content_type: Some("text/html\r\nWARC-Type: revisit"),
                ..record(b"body", None)
            },
        ),
        (
            "target_uri",
            WarcRecord {
                target_uri: "https://example.test/a\nWARC-Target-URI: https://elsewhere.test/",
                ..record(b"body", None)
            },
        ),
        (
            "record_id",
            WarcRecord {
                record_id: "urn:capsem:x\r\nWARC-Type: warcinfo",
                ..record(b"body", None)
            },
        ),
        (
            "date",
            WarcRecord {
                date: "2026-09-17T10:11:12Z\r\nWARC-Date: 1999-01-01T00:00:00Z",
                ..record(b"body", None)
            },
        ),
    ];

    for (field, rec) in cases {
        let mut buffer = Vec::new();
        let error = write_record(&mut buffer, &rec).expect_err("a forged header must be refused");
        assert!(
            error.to_string().contains(field),
            "the error must name the offending field, got: {error}"
        );
        assert!(
            buffer.is_empty(),
            "a refused record must not leave a half-written member behind"
        );
    }
}
