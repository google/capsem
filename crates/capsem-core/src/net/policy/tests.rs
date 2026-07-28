use super::*;

fn dev_policy() -> NetworkMechanics {
    NetworkMechanics::default_dev()
}

// -- DomainMatcher::parse --

#[test]
fn parse_exact() {
    let m = DomainMatcher::parse("github.com");
    assert!(matches!(m, DomainMatcher::Exact(_)));
    assert_eq!(m.pattern_str(), "github.com");
}

#[test]
fn parse_wildcard() {
    let m = DomainMatcher::parse("*.github.com");
    assert!(matches!(m, DomainMatcher::Wildcard(_)));
    assert_eq!(m.pattern_str(), "*.github.com");
}

#[test]
fn parse_uppercased_normalized() {
    let m = DomainMatcher::parse("GitHub.COM");
    assert!(m.matches("github.com"));
}

// -- log_bodies default --

#[test]
fn log_bodies_default_true() {
    let policy = dev_policy();
    assert!(policy.log_bodies);
}

// =====================================================================
// (T3.d) -- DnsRedirect rule tests
// =====================================================================

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

fn redirect(pattern: &str, qtype: Option<u16>, ips: Vec<IpAddr>) -> DnsRedirect {
    DnsRedirect::new(pattern, qtype, ips, 60)
}

#[test]
fn find_redirect_exact_match_a_qtype() {
    let mut p = NetworkMechanics::new();
    p.dns_redirects.push(redirect(
        "anthropic.com",
        Some(1),
        vec![IpAddr::V4(Ipv4Addr::new(10, 20, 30, 40))],
    ));
    let r = p.find_dns_redirect("anthropic.com", 1).unwrap();
    assert_eq!(r.matcher.pattern_str(), "anthropic.com");
    assert_eq!(r.answers.len(), 1);
    assert_eq!(r.ttl, 60);
}

#[test]
fn find_redirect_qtype_filter_misses() {
    let mut p = NetworkMechanics::new();
    p.dns_redirects.push(redirect(
        "anthropic.com",
        Some(1), // A only
        vec![IpAddr::V4(Ipv4Addr::new(10, 20, 30, 40))],
    ));
    // AAAA query (qtype=28) on the same name -- no match.
    assert!(p.find_dns_redirect("anthropic.com", 28).is_none());
}

#[test]
fn find_redirect_any_qtype_matches_aaaa() {
    let mut p = NetworkMechanics::new();
    p.dns_redirects.push(redirect(
        "anthropic.com",
        None, // any qtype
        vec![IpAddr::V6(Ipv6Addr::LOCALHOST)],
    ));
    let r_a = p.find_dns_redirect("anthropic.com", 1).unwrap();
    assert!(r_a.qtype.is_none());
    let r_aaaa = p.find_dns_redirect("anthropic.com", 28).unwrap();
    assert!(r_aaaa.qtype.is_none());
}

#[test]
fn find_redirect_wildcard_subdomain_match() {
    let mut p = NetworkMechanics::new();
    p.dns_redirects.push(redirect(
        "*.openai.com",
        None,
        vec![IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))],
    ));
    assert!(p.find_dns_redirect("api.openai.com", 1).is_some());
    assert!(p.find_dns_redirect("foo.openai.com", 28).is_some());
    // Wildcard does NOT match the base.
    assert!(p.find_dns_redirect("openai.com", 1).is_none());
}

#[test]
fn find_redirect_first_match_wins() {
    let mut p = NetworkMechanics::new();
    p.dns_redirects.push(redirect(
        "anthropic.com",
        None,
        vec![IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1))],
    ));
    p.dns_redirects.push(redirect(
        "anthropic.com",
        None,
        vec![IpAddr::V4(Ipv4Addr::new(2, 2, 2, 2))],
    ));
    let r = p.find_dns_redirect("anthropic.com", 1).unwrap();
    assert_eq!(r.answers, vec![IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1))]);
}

#[test]
fn find_redirect_no_match_returns_none() {
    let mut p = NetworkMechanics::new();
    p.dns_redirects.push(redirect(
        "anthropic.com",
        Some(1),
        vec![IpAddr::V4(Ipv4Addr::LOCALHOST)],
    ));
    assert!(p.find_dns_redirect("example.com", 1).is_none());
}

#[test]
fn find_redirect_empty_list_returns_none() {
    let p = NetworkMechanics::new();
    assert!(p.find_dns_redirect("anything.com", 1).is_none());
}

#[test]
fn dns_redirects_default_empty() {
    let p = NetworkMechanics::new();
    assert!(p.dns_redirects.is_empty());
    let p2 = NetworkMechanics::default_dev();
    assert!(p2.dns_redirects.is_empty());
}

#[test]
fn dns_redirect_empty_answers_is_legal() {
    // Empty `answers` is the "name exists, no record of that
    // type" signal -- still a valid policy entry.
    let mut p = NetworkMechanics::new();
    p.dns_redirects
        .push(redirect("nodata.example.com", None, vec![]));
    let r = p.find_dns_redirect("nodata.example.com", 1).unwrap();
    assert!(r.answers.is_empty());
}
