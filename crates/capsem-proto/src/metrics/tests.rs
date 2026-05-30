use super::*;

fn point<'a>(points: &'a [OtelMetricPoint], name: &str) -> &'a OtelMetricPoint {
    points
        .iter()
        .find(|point| point.name == name)
        .unwrap_or_else(|| panic!("missing point {name}"))
}

#[test]
fn otel_metric_points_include_resource_and_block_counters() {
    let mut snapshot = VmMetricsSnapshot::empty("vm-otel", true, 1_700_000_123_456);
    snapshot.resources.configured_ram_mb = 2048;
    snapshot.resources.configured_vcpus = 2;
    snapshot.resources.host_process_rss_bytes = Some(256 * 1024 * 1024);
    snapshot.resources.host_cpu_time_micros = Some(1_250_000);
    snapshot.hypervisor.block.queue_notifications_total = 5_908;
    snapshot.hypervisor.block.queue_drains_total = 1_638;
    snapshot.hypervisor.block.descriptors_drained_total = 25_264;
    snapshot.hypervisor.block.used_entries_total = 25_264;
    snapshot.hypervisor.block.read_ops_total = 8_578;
    snapshot.hypervisor.block.bytes_read_total = 31_394_816;
    snapshot.hypervisor.block.async_queue_full_total = 2;
    snapshot.hypervisor.block.async_in_flight = 3;

    let points = snapshot.otel_metric_points();

    let ram = point(&points, "capsem.vm.resource.configured_ram");
    assert_eq!(ram.kind, OtelMetricKind::Gauge);
    assert_eq!(ram.unit, "MiBy");
    assert_eq!(ram.value, 2048.0);

    let rss = point(&points, "capsem.vm.resource.host_process_rss");
    assert_eq!(rss.unit, "By");
    assert_eq!(rss.value, (256 * 1024 * 1024) as f64);

    let notifications = point(&points, "capsem.vm.block.queue_notifications");
    assert_eq!(notifications.kind, OtelMetricKind::Counter);
    assert_eq!(notifications.value, 5_908.0);

    let bytes_read = point(&points, "capsem.vm.block.bytes_read");
    assert_eq!(bytes_read.unit, "By");
    assert_eq!(bytes_read.value, 31_394_816.0);

    let in_flight = point(&points, "capsem.vm.block.async_in_flight");
    assert_eq!(in_flight.kind, OtelMetricKind::Gauge);
    assert_eq!(in_flight.value, 3.0);

    let queue_full = point(&points, "capsem.vm.block.async_queue_full");
    assert_eq!(queue_full.kind, OtelMetricKind::Counter);
    assert_eq!(queue_full.value, 2.0);

    assert!(points.iter().all(|point| point.source_vm_id == "vm-otel"
        && point.persistent
        && point.captured_at_unix_ms == 1_700_000_123_456));
}

#[test]
fn otel_metric_points_use_bounded_attributes() {
    let mut snapshot = VmMetricsSnapshot::empty("vm-otel", false, 1);
    snapshot.hypervisor.block.queue_notifications_total = 1;

    let points = snapshot.otel_metric_points();
    for point in points {
        for attribute in point.attributes {
            assert!(
                matches!(attribute.key.as_str(), "component" | "backend"),
                "unexpected high-cardinality metric attribute {}",
                attribute.key
            );
        }
    }
}
