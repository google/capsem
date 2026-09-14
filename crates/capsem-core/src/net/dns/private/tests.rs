use super::*;

#[test]
fn names_under_the_zone_are_private_questions_and_others_are_not() {
    assert_eq!(
        private_question("beta.team.capsem.internal", 1),
        Some(PrivateQuestion::Name("beta.team".into()))
    );
    assert_eq!(
        private_question("beta.capsem.internal", 28),
        Some(PrivateQuestion::Name("beta".into()))
    );
    assert_eq!(
        private_question("capsem.internal", 1),
        Some(PrivateQuestion::Name(String::new()))
    );
    assert_eq!(private_question("notcapsem.internal", 1), None);
    assert_eq!(private_question("example.com", 1), None);
    assert_eq!(private_question("beta.team.capsem.internal.example.com", 1), None);
}

#[test]
fn reverse_questions_inside_the_pool_are_private_and_outside_are_not() {
    assert_eq!(
        private_question("200.7.129.10.in-addr.arpa", 12),
        Some(PrivateQuestion::Reverse(Ipv4Addr::new(10, 129, 7, 200)))
    );
    assert_eq!(
        private_question("2.0.128.10.in-addr.arpa", 1),
        Some(PrivateQuestion::Reverse(Ipv4Addr::new(10, 128, 0, 2)))
    );
    assert_eq!(private_question("1.1.1.1.in-addr.arpa", 12), None, "outside the pool");
    assert_eq!(
        private_question("1.0.0.10.in-addr.arpa", 12),
        None,
        "the guest's own network is not the pool"
    );
    assert_eq!(private_question("x.0.128.10.in-addr.arpa", 12), None, "not an address");
    assert_eq!(private_question("0.128.10.in-addr.arpa", 12), None, "not four octets");
}
