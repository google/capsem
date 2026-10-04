use super::*;
use crate::client::tests::fake_service::FakeService;
use crate::{Cli, Commands};
use clap::Parser;
use serde_json::json;

fn images(argv: &[&str]) -> ImagesArgs {
    let argv = ["capsem", "images"].iter().chain(argv).copied();
    match Cli::parse_from(argv).command.unwrap() {
        Commands::Images(args) => args,
        _ => panic!("expected images"),
    }
}

#[test]
fn images_alone_lists_and_pull_takes_one_image() {
    let list = images(&[]);
    assert!(list.command.is_none() && !list.refresh && !list.json);
    let list = images(&["--refresh", "--json"]);
    assert!(list.refresh && list.json);
    match images(&["pull", "codex-cli", "--registry-user", "me"]).command {
        Some(ImageCommands::Pull {
            image, registry_user, ..
        }) => {
            assert_eq!(image, "codex-cli");
            assert_eq!(registry_user.as_deref(), Some("me"));
        }
        None => panic!("expected images pull"),
    }
    for argv in [
        vec!["capsem", "images", "pull"],
        vec!["capsem", "images", "--json", "pull", "codex-cli"],
        vec!["capsem", "images", "pull", "a", "b"],
    ] {
        assert!(Cli::try_parse_from(&argv).is_err(), "accepted {argv:?}");
    }
}

fn listing() -> serde_json::Value {
    json!({
        "catalog": {"reference": "ghcr.io/google/capsem/catalog:stable", "digest": format!("sha256:{}", "9".repeat(64)), "channel": "stable"},
        "images": [
            {"name": "codex-cli", "description": "OpenAI Codex CLI", "architectures": ["amd64", "arm64"],
             "image": format!("ghcr.io/google/capsem/codex-cli@sha256:{}", "ab".repeat(32)), "cached": "unknown"},
            {"name": "elsewhere", "description": "Another machine's", "architectures": ["amd64"], "cached": "unknown"}
        ]
    })
}

#[test]
fn the_table_names_each_image_its_architectures_and_a_short_digest() {
    let list: ImageListResponse = serde_json::from_value(listing()).unwrap();
    assert_eq!(
        render(&list),
        "Catalog ghcr.io/google/capsem/catalog:stable (stable, sha256:999999999999)\n\
         NAME                 ARCH           DIGEST               DESCRIPTION\n\
         codex-cli            amd64,arm64    sha256:abababababab  OpenAI Codex CLI\n\
         elsewhere            amd64          -                    Another machine's\n"
    );
    let none: ImageListResponse = serde_json::from_value(json!({"images": []})).unwrap();
    assert!(render(&none).starts_with("No image catalog"), "{}", render(&none));
}

#[tokio::test]
async fn listing_reads_the_service_and_refresh_asks_it_to_reread() {
    let service = FakeService::start();
    service
        .route("GET", "/images", 200, listing())
        .route("GET", "/images?refresh=true", 200, listing());
    run(&service.client, &images(&[])).await.unwrap();
    run(&service.client, &images(&["--refresh"])).await.unwrap();
    assert_eq!(service.calls(), ["GET /images", "GET /images?refresh=true"]);
}

#[tokio::test]
async fn pull_sends_the_name_for_the_service_to_resolve() {
    let service = FakeService::start();
    let resolved = format!("ghcr.io/google/capsem/codex-cli@sha256:{}", "b".repeat(64));
    service.route(
        "POST",
        "/images/pull",
        200,
        json!({"image": "codex-cli", "resolved": resolved, "digest": format!("sha256:{}", "7".repeat(64))}),
    );
    run(&service.client, &images(&["pull", "codex-cli"])).await.unwrap();
    assert_eq!(
        service.find("POST", "/images/pull")[0].json(),
        json!({"image": "codex-cli"}),
        "anonymous pulls send no registry access"
    );
}

#[tokio::test]
async fn a_refused_pull_is_an_error_with_the_services_reason() {
    let service = FakeService::start();
    service.route(
        "POST",
        "/images/pull",
        403,
        json!({"error": "image source docker.io/library/redis is not allowed"}),
    );
    let error = run(&service.client, &images(&["pull", "docker://redis"]))
        .await
        .unwrap_err();
    assert!(format!("{error:#}").contains("not allowed"), "{error:#}");
}

#[tokio::test]
async fn pull_refuses_what_names_no_image_before_asking_the_service() {
    let service = FakeService::start();
    for image in ["not an image", "Codex", "library/redis"] {
        let error = run(&service.client, &images(&["pull", image])).await.unwrap_err();
        assert!(format!("{error:#}").contains("catalog name"), "{image}: {error:#}");
    }
    assert!(service.calls().is_empty(), "{:?}", service.calls());
}
