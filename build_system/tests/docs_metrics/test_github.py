from capsem_builder.docs_metrics.github import GitHub, monthly_outcomes


def test_pagination_keeps_records_beyond_first_page():
    calls = []

    def fetch(path):
        calls.append(path)
        return (
            {"workflow_runs": [{"id": i} for i in range(100)]}
            if path.endswith("&page=1")
            else {"workflow_runs": [{"id": 100}]}
        )

    rows = GitHub(fetch=fetch).pages("repos/example/project/actions/runs", "workflow_runs")
    assert len(rows) == 101
    assert rows[-1]["id"] == 100
    assert len(calls) == 2


def test_outcomes_include_empty_months_and_keep_cancellations_separate():
    rows = [
        {
            "started_at_utc": "2025-10-01T00:00:00Z",
            "conclusion": "success",
            "summed_job_seconds": "120",
        },
        {
            "started_at_utc": "2025-12-01T00:00:00Z",
            "conclusion": "failure",
            "summed_job_seconds": "3600",
        },
        {
            "started_at_utc": "2025-12-02T00:00:00Z",
            "conclusion": "cancelled",
            "summed_job_seconds": "30",
        },
    ]
    result = monthly_outcomes(rows)
    assert [r["month"] for r in result] == ["2025-10", "2025-11", "2025-12"]
    assert result[1]["failure"] == result[1]["success"] == 0
    assert result[2]["failure"] == result[2]["cancelled"] == 1
    assert result[2]["failedJobHours"] == 1


def test_success_intervals_are_separate_by_lane_and_count_reruns():
    from capsem_builder.docs_metrics.github import success_intervals

    def row(index, lane, conclusion):
        return {
            "run_id": str(index),
            "attempt": 1,
            "lane": lane,
            "conclusion": conclusion,
            "completed_at_utc": f"2026-10-{index:02}T00:00:00Z",
            "started_at_utc": f"2026-10-{index:02}T00:00:00Z",
            "duration_seconds": 120,
            "version": "v1.0.0",
            "run_url": "https://example.org/run",
        }

    records = [
        row(1, "binary", "failure"),
        row(2, "runtime", "failure"),
        row(3, "binary", "cancelled"),
        row(4, "binary", "success"),
        row(5, "binary", "success"),
        row(6, "runtime", "success"),
        row(7, "binary", "failure"),
    ]
    result = success_intervals(list(reversed(records)))
    assert [r["failures"] for r in result] == [1, 0, 1]
    assert [r["cancelled"] for r in result] == [1, 0, 0]
    assert [r["minutes"] for r in result] == [2, 2, 2]
