use super::*;

use super::fixtures::{answer_to, query};

#[test]
fn parses_id_name_type_and_class_and_lowercases_the_name() {
    let q = parse_question(&query(0xBEEF, "Load-Test.Example.COM")).unwrap();
    assert_eq!(q.id, 0xBEEF);
    assert_eq!(q.name, b"\x09load-test\x07example\x03com");
    assert_eq!(q.qtype, 1);
    assert_eq!(q.qclass, 1);
    assert_eq!(q.end, 12 + 1 + 9 + 1 + 7 + 1 + 3 + 1 + 4);
}

#[test]
fn refuses_what_it_cannot_verify() {
    assert!(parse_question(&[0u8; 11]).is_none(), "short header");
    let mut two_questions = query(1, "a.example");
    two_questions[5] = 2;
    assert!(parse_question(&two_questions).is_none(), "qdcount != 1");
    let mut pointer = query(1, "a.example");
    pointer[12] = 0xC0;
    assert!(
        parse_question(&pointer).is_none(),
        "compression pointer in the question"
    );
    let mut truncated = query(1, "a.example");
    truncated.truncate(truncated.len() - 2);
    assert!(parse_question(&truncated).is_none(), "question section cut short");
    let long_label = "x".repeat(64);
    assert!(parse_question(&query(1, &long_label)).is_none(), "label over 63 octets");
    let long_name = (0..5).map(|_| "y".repeat(60)).collect::<Vec<_>>().join(".");
    assert!(parse_question(&query(1, &long_name)).is_none(), "name over 255 octets");
}

#[test]
fn an_answer_must_carry_the_same_id_question_and_response_bit() {
    let q = parse_question(&query(7, "svc.example")).unwrap();
    assert!(q.is_answered_by(&answer_to(&query(7, "svc.example"))));
    assert!(
        q.is_answered_by(&answer_to(&query(7, "SVC.Example"))),
        "case-insensitive name"
    );
    assert!(!q.is_answered_by(&query(7, "svc.example")), "response bit clear");
    assert!(
        !q.is_answered_by(&answer_to(&query(8, "svc.example"))),
        "other transaction id"
    );
    assert!(!q.is_answered_by(&answer_to(&query(7, "other.example"))), "other name");
    let mut aaaa = answer_to(&query(7, "svc.example"));
    let end = aaaa.len();
    aaaa[end - 3] = 28;
    assert!(!q.is_answered_by(&aaaa), "other record type");
    assert!(!q.is_answered_by(&[0x00, 0x07, 0x81]), "not even a header");
}

#[test]
fn a_dot_inside_one_label_is_not_two_labels() {
    let normal = query(7, "a.b.example");
    let mut other = normal.clone();
    other[12] = 3;
    other[14] = b'.';
    let expected = parse_question(&normal).unwrap();
    assert!(
        !expected.is_answered_by(&answer_to(&other)),
        "label boundaries are part of the DNS name"
    );
}

#[test]
fn opcode_is_part_of_answer_identity_and_survives_servfail() {
    let mut query = query(7, "svc.example");
    let expected = parse_question(&query).unwrap();
    query[2] |= 5 << 3;
    assert!(!expected.is_answered_by(&answer_to(&query)));
    assert!(parse_question(&query)
        .unwrap()
        .is_answered_by(&servfail_for(&query).unwrap()));
}

#[test]
fn the_name_length_limit_includes_the_root_label() {
    let name = ["x".repeat(63), "y".repeat(63), "z".repeat(63), "w".repeat(61)].join(".");
    assert!(parse_question(&query(1, &name)).is_some());
    assert!(parse_question(&query(1, &(name + "w"))).is_none());
}

#[test]
fn servfail_keeps_the_id_and_question_and_invents_no_records() {
    let q = query(0x1234, "dead.example");
    let sf = servfail_for(&q).unwrap();
    assert_eq!(&sf[0..2], &[0x12, 0x34]);
    assert_eq!(sf[2], 0x81, "QR set, RD kept");
    assert_eq!(sf[3], 0x02, "rcode SERVFAIL");
    assert_eq!(&sf[4..6], &[0, 1], "one question");
    assert_eq!(&sf[6..12], &[0; 6], "no answer, authority or additional records");
    assert_eq!(&sf[12..], &q[12..], "question echoed");
    assert!(servfail_for(&[1, 2, 3]).is_none(), "nothing to build from");
}
