use super::*;

#[tokio::test(flavor = "current_thread")]
async fn optional_metric_export_async_probe() {
    let Ok(mode) = std::env::var("CAPSEM_TEST_ASYNC_METRIC_MODE") else {
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let (_settings, _corp, _path) = install_empty_settings_env(&directory);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    // Synthetic loopback collector only; no ambient headers or destination.
    std::env::set_var(
        "OTEL_EXPORTER_OTLP_ENDPOINT",
        format!("http://{}", listener.local_addr().unwrap()),
    );
    let (sent, mut received) = tokio::sync::mpsc::channel(4);
    let collector = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new().route(
                "/v1/metrics",
                axum::routing::post(move |_: axum::body::Bytes| async move {
                    sent.send(()).await.unwrap();
                    axum::http::StatusCode::NO_CONTENT
                }),
            ),
        )
        .await
        .unwrap();
    });
    if mode == "off" {
        std::env::remove_var("OTEL_EXPORTER_OTLP_ENDPOINT");
    }
    if mode == "invalid" {
        std::env::set_var("OTEL_EXPORTER_OTLP_METRICS_ENDPOINT", "not a URL");
    }
    let exporter = crate::service_runtime::telemetry_export::install().await;
    if matches!(mode.as_str(), "off" | "invalid") {
        assert!(exporter.is_none());
        assert!(received.try_recv().is_err());
    } else {
        let exporter = exporter.expect("optional exporter installs from async startup");
        metrics::counter!(capsem_telemetry::db::DB_WRITE_OPS_TOTAL).increment(1);
        if mode == "duplicate" {
            assert!(crate::service_runtime::telemetry_export::install().await.is_none());
        }
        if mode == "shutdown" {
            exporter.shutdown().await;
        } else if mode == "cancel" {
            let (ready, waiting) = tokio::sync::oneshot::channel();
            let task = tokio::spawn(async move {
                let _exporter = exporter;
                ready.send(()).unwrap();
                std::future::pending::<()>().await;
            });
            waiting.await.unwrap();
            task.abort();
            assert!(task.await.unwrap_err().is_cancelled());
        } else {
            drop(exporter);
        }
        tokio::time::timeout(std::time::Duration::from_secs(3), received.recv())
            .await
            .unwrap()
            .unwrap();
    }
    collector.abort();
}

#[tokio::test]
async fn optional_metrics_install_and_drop_are_safe_on_the_async_service() {
    for mode in ["normal", "shutdown", "cancel", "duplicate", "off", "invalid"] {
        let mut command = isolated_test_command(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "tests::telemetry_export::lifecycle::optional_metric_export_async_probe",
                "--nocapture",
            ])
            .env("CAPSEM_TEST_ASYNC_METRIC_MODE", mode);
        let output = tokio::time::timeout(std::time::Duration::from_secs(15), command.output())
            .await
            .unwrap()
            .unwrap();
        assert!(
            output.status.success(),
            "{mode}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!String::from_utf8_lossy(&output.stderr).contains("panicked"));
    }
}
