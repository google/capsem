"""The load benchmark measures the resources its admitted work consumes."""

from decimal import Decimal

from helpers.process_resources import ProcessTreeSampler, ProcessUsage, TreeSnapshot


def snapshot(*rows: ProcessUsage) -> TreeSnapshot:
    return TreeSnapshot({(row.pid, row.started_at): row for row in rows})


def usage(
    pid: int,
    name: str,
    *,
    cpu: str,
    rss: int,
    pss: int,
    uss: int,
    fds: int,
    started_at: float = 1.0,
) -> ProcessUsage:
    return ProcessUsage(pid, started_at, name, Decimal(cpu), rss, pss, uss, fds)


def test_sampler_reports_tree_and_named_process_pressure() -> None:
    sampler = ProcessTreeSampler(None)
    sampler.observe(
        snapshot(
            usage(1, "capsem-service", cpu="10.0", rss=100, pss=80, uss=60, fds=5),
            usage(2, "capsem-proxy", cpu="20.0", rss=200, pss=150, uss=120, fds=20),
        )
    )
    sampler.observe(
        snapshot(
            usage(1, "capsem-service", cpu="10.2", rss=110, pss=85, uss=65, fds=6),
            usage(2, "capsem-proxy", cpu="21.5", rss=500, pss=420, uss=390, fds=400),
            usage(3, "capsem-exec", cpu="0.1", rss=50, pss=40, uss=35, fds=4),
        )
    )

    assert sampler.metrics() == {
        "resources.total.cpu_s": {"unit": "seconds", "samples": [1.8]},
        "resources.total.peak_rss_bytes": {"unit": "bytes", "samples": [660]},
        "resources.total.peak_rss_delta_bytes": {
            "unit": "bytes",
            "samples": [360],
        },
        "resources.total.peak_pss_bytes": {"unit": "bytes", "samples": [545]},
        "resources.total.peak_pss_delta_bytes": {
            "unit": "bytes",
            "samples": [315],
        },
        "resources.total.peak_uss_bytes": {"unit": "bytes", "samples": [490]},
        "resources.total.peak_uss_delta_bytes": {
            "unit": "bytes",
            "samples": [310],
        },
        "resources.total.peak_fds": {"unit": "count", "samples": [410]},
        "resources.total.peak_fd_delta": {"unit": "count", "samples": [385]},
        "resources.total.peak_processes": {"unit": "count", "samples": [3]},
        "resources.capsem_service.cpu_s": {"unit": "seconds", "samples": [0.2]},
        "resources.capsem_service.peak_rss_bytes": {
            "unit": "bytes",
            "samples": [110],
        },
        "resources.capsem_service.peak_rss_delta_bytes": {
            "unit": "bytes",
            "samples": [10],
        },
        "resources.capsem_service.peak_pss_bytes": {
            "unit": "bytes",
            "samples": [85],
        },
        "resources.capsem_service.peak_pss_delta_bytes": {
            "unit": "bytes",
            "samples": [5],
        },
        "resources.capsem_service.peak_uss_bytes": {
            "unit": "bytes",
            "samples": [65],
        },
        "resources.capsem_service.peak_uss_delta_bytes": {
            "unit": "bytes",
            "samples": [5],
        },
        "resources.capsem_service.peak_fds": {"unit": "count", "samples": [6]},
        "resources.capsem_service.peak_fd_delta": {
            "unit": "count",
            "samples": [1],
        },
        "resources.capsem_proxy.cpu_s": {"unit": "seconds", "samples": [1.5]},
        "resources.capsem_proxy.peak_rss_bytes": {
            "unit": "bytes",
            "samples": [500],
        },
        "resources.capsem_proxy.peak_rss_delta_bytes": {
            "unit": "bytes",
            "samples": [300],
        },
        "resources.capsem_proxy.peak_pss_bytes": {
            "unit": "bytes",
            "samples": [420],
        },
        "resources.capsem_proxy.peak_pss_delta_bytes": {
            "unit": "bytes",
            "samples": [270],
        },
        "resources.capsem_proxy.peak_uss_bytes": {
            "unit": "bytes",
            "samples": [390],
        },
        "resources.capsem_proxy.peak_uss_delta_bytes": {
            "unit": "bytes",
            "samples": [270],
        },
        "resources.capsem_proxy.peak_fds": {"unit": "count", "samples": [400]},
        "resources.capsem_proxy.peak_fd_delta": {
            "unit": "count",
            "samples": [380],
        },
    }


def test_process_that_exits_keeps_its_last_cpu_sample() -> None:
    sampler = ProcessTreeSampler(None)
    sampler.observe(snapshot(usage(7, "capsem-proxy", cpu="3", rss=20, pss=15, uss=10, fds=2)))
    sampler.observe(snapshot(usage(7, "capsem-proxy", cpu="4", rss=30, pss=25, uss=20, fds=3)))
    sampler.observe(snapshot())

    assert sampler.metrics()["resources.capsem_proxy.cpu_s"]["samples"] == [1.0]


def test_sampler_attributes_dedicated_ledger_pressure() -> None:
    sampler = ProcessTreeSampler(None)
    sampler.observe(snapshot(usage(8, "capsem-ledger", cpu="2", rss=100, pss=80, uss=70, fds=5)))
    sampler.observe(snapshot(usage(8, "capsem-ledger", cpu="3", rss=250, pss=220, uss=200, fds=7)))

    metrics = sampler.metrics()
    assert metrics["resources.capsem_ledger.cpu_s"]["samples"] == [1.0]
    assert metrics["resources.capsem_ledger.peak_pss_bytes"]["samples"] == [220]
    assert metrics["resources.capsem_ledger.peak_pss_delta_bytes"]["samples"] == [140]


def test_platform_without_proportional_memory_omits_those_metrics() -> None:
    sampler = ProcessTreeSampler(None)
    sampler.observe(
        snapshot(ProcessUsage(1, 1.0, "capsem-service", Decimal(1), 100, None, None, 2))
    )

    assert not any("pss" in name or "uss" in name for name in sampler.metrics())
