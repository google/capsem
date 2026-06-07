//! Capsem DNS proxy: host-side resolver + security gate.
//!
//! The capsem DNS proxy replaced the pre-T3 in-guest dnsmasq fake
//! (which returned the sentinel `10.0.0.1` for every name) with a
//! real recursive resolver running on the host, gated by canonical
//! `dns.query` security rules. Pre-T3 the guest's resolver had
//! no view into "is this domain blocked" -- the MITM proxy could only
//! reject *connections* after the TLS handshake started. With T3 the
//! decision moves up the stack: a blocked domain returns NXDOMAIN at
//! resolution time, the guest's `getaddrinfo` returns NOTFOUND, and the
//! application sees a clean "no such host" error rather than a TLS
//! reset that some libraries silently retry.
//!
//! ## Module layout
//!
//! - `server`: the [`DnsHandler`] -- bytes-in / bytes-out async
//!   processor. Decodes the query (via `parsers::dns_parser`), evaluates
//!   the security-event rules for the qname/qtype, and either synthesizes
//!   an NXDOMAIN response or forwards to the upstream
//!   resolver. Returns a [`server::DnsHandlerResult`] carrying the
//!   answer bytes plus structured metadata for telemetry (decision,
//!   matched_rule, upstream_resolver_ms, rcode).
//! - `resolver`: the [`DnsResolver`] -- a UDP-based forwarder that
//!   sends raw query bytes to one of N configured upstream nameservers
//!   (default `1.1.1.1:53`, `8.8.8.8:53`) and returns the raw response
//!   bytes. Transparent: the upstream's CNAMEs, TTLs, and EDNS extras
//!   pass through untouched.
//!
//! ## Why not hickory-server?
//!
//! The plan called for "hickory-server-based DNS proxy". After looking
//! at the API we found `hickory_server`'s `RequestHandler` trait is
//! tightly coupled to its own `Request` / `Response` types built around
//! owned UDP/TCP server-side state. We accept raw bytes from a vsock
//! envelope, so the cleanest path is `hickory-proto` (wire codec) +
//! a thin async handler wrapping our security rules. Half
//! the dep weight, none of the impedance mismatch. The guest agent
//! depends on neither -- it only forwards bytes.

pub mod cache;
pub mod resolver;
pub mod server;
pub mod telemetry;

pub use cache::{DnsAnswerCache, DEFAULT_CAPACITY, DEFAULT_MAX_TTL_SECS, MIN_TTL_SECS};
pub use resolver::{DnsResolver, DEFAULT_UPSTREAMS};
pub use server::{DnsHandler, DnsHandlerResult, SharedPolicy};
pub use telemetry::{build_dns_event, security_event_from_dns_event};
