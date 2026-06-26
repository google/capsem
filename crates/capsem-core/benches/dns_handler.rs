use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use capsem_core::net::dns::{DnsHandler, DnsResolver};
use capsem_core::net::policy::NetworkMechanics;
use capsem_core::net::policy_config::{
    SecurityPluginConfig, SecurityRuleProfile, SecurityRuleSet, SecurityRuleSource,
};
use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};
use hickory_proto::op::{Message, MessageType, OpCode, Query};
use hickory_proto::rr::{Name, RecordType};

fn build_query_bytes(name: &str, qtype: RecordType, id: u16) -> Vec<u8> {
    let mut msg = Message::new(id, MessageType::Query, OpCode::Query);
    msg.metadata.recursion_desired = true;
    let name = Name::from_ascii(name).expect("benchmark qname is valid");
    msg.add_query(Query::query(name, qtype));
    msg.to_vec().expect("benchmark query encodes")
}

fn local_nxdomain_handler() -> DnsHandler {
    let policy = Arc::new(RwLock::new(Arc::new(NetworkMechanics::new())));
    let profile = SecurityRuleProfile::parse_toml("").expect("empty rule profile parses");
    let rules = SecurityRuleSet::compile_profile(&profile, SecurityRuleSource::User)
        .expect("empty rule profile compiles");
    let security_rules = Arc::new(RwLock::new(Arc::new(rules)));
    let plugin_policy = Arc::new(RwLock::new(BTreeMap::<String, SecurityPluginConfig>::new()));
    let resolver = Arc::new(DnsResolver::with_upstreams(Vec::new()));
    DnsHandler::new(policy, security_rules, plugin_policy, resolver)
}

fn bench_local_nxdomain(c: &mut Criterion) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("benchmark runtime builds");
    let handler = local_nxdomain_handler();
    let query = build_query_bytes("load-test.capsem-bogus.", RecordType::A, 0x1234);

    let mut group = c.benchmark_group("dns_handler");
    group.throughput(Throughput::Elements(1));
    group.bench_function("local_nxdomain_no_upstream", |b| {
        b.iter(|| {
            let result = runtime.block_on(handler.handle(black_box(&query)));
            black_box(result);
        });
    });
    group.finish();
}

criterion_group!(benches, bench_local_nxdomain);
criterion_main!(benches);
