use capsem_core::security_packs::{
    compile_detection_ir_to_cel_detection_rules, parse_detection_ir_v1_json, DetectionIRV1,
};
use capsem_security_engine::CelDetectionEvaluator;
use criterion::{black_box, criterion_group, criterion_main, Criterion};

const GOOGLE_SECRET_IR_JSON: &str =
    include_str!("../../../data/detection/ir/google-secret-egress.json");

fn google_secret_ir() -> DetectionIRV1 {
    parse_detection_ir_v1_json(GOOGLE_SECRET_IR_JSON).unwrap()
}

fn hundred_rule_ir() -> DetectionIRV1 {
    let mut ir = google_secret_ir();
    let template = ir.rules[0].clone();
    ir.rules = (0..100)
        .map(|index| {
            let mut rule = template.clone();
            rule.id = format!("detect-google-secret-{index:03}");
            rule.source_id = rule.id.clone();
            rule.sigma_id = Some(format!("sigma-google-secret-{index:03}"));
            rule
        })
        .collect();
    ir
}

fn bench_detection_ir_parse(c: &mut Criterion) {
    let mut group = c.benchmark_group("security_packs_detection_ir_parse");

    group.bench_function("parse_validate_google_secret_fixture", |b| {
        b.iter(|| black_box(parse_detection_ir_v1_json(black_box(GOOGLE_SECRET_IR_JSON))).unwrap());
    });

    group.finish();
}

fn bench_detection_ir_lowering(c: &mut Criterion) {
    let single_rule = google_secret_ir();
    let hundred_rules = hundred_rule_ir();
    let mut group = c.benchmark_group("security_packs_detection_ir_lowering");

    group.bench_function("lower_google_secret_fixture_to_cel_rules", |b| {
        b.iter(|| {
            let rules = compile_detection_ir_to_cel_detection_rules(black_box(&single_rule))
                .expect("fixture should lower");
            black_box(rules.len())
        });
    });

    group.bench_function("lower_100_http_rules_to_cel_rules", |b| {
        b.iter(|| {
            let rules = compile_detection_ir_to_cel_detection_rules(black_box(&hundred_rules))
                .expect("fixture should lower");
            black_box(rules.len())
        });
    });

    group.bench_function("lower_and_compile_100_http_rules", |b| {
        b.iter(|| {
            let rules = compile_detection_ir_to_cel_detection_rules(black_box(&hundred_rules))
                .expect("fixture should lower");
            let evaluator = CelDetectionEvaluator::compile(black_box(rules)).unwrap();
            black_box(evaluator)
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_detection_ir_parse,
    bench_detection_ir_lowering
);
criterion_main!(benches);
