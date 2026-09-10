use std::time::Duration;

use serde_json::json;

use crate::models::{HistoryLayerFilter, HostLogSource, TimelineLayer};
use crate::test_gateway::Server;
use crate::*;

mod fixture;
use fixture::{gateway, reply, request};

#[tokio::test]
async fn hypervisor_defaults_overrides_update_and_vm_handle_lifetime() {
    let mut server = gateway().await;
    let hv = Hypervisor::new(&server.url, "private-token").unwrap();
    hv.info().await.unwrap();
    request(&mut server, "/status").await;
    assert_eq!(hv.list().await.unwrap().sandboxes[0].id, "vm-1");
    request(&mut server, "/vms/list").await;
    for name in [None, Some(String::new())] {
        let vm = hv
            .create(
                "code",
                CreateOptions {
                    name,
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(vm.id(), Some("vm-1"));
        assert_eq!(vm.name(), Some("work"));
        let body = request(&mut server, "/vms/create").await;
        assert_eq!(body["name"], json!(null));
        assert_eq!(body["profile_id"], "code");
        assert_eq!(body["persistent"], false);
        for omitted in ["ram_mb", "cpus", "env"] {
            assert!(body.get(omitted).is_none());
        }
    }
    let vm = hv
        .create(
            "co-work",
            CreateOptions {
                name: Some("work".into()),
                vcpu: Some(4),
                memory: Some("8G".parse().unwrap()),
                env: Some([("EDITOR".into(), "vim".into())].into()),
            },
        )
        .await
        .unwrap();
    let body = request(&mut server, "/vms/create").await;
    assert_eq!(body["ram_mb"], 8192);
    assert_eq!(body["cpus"], 4);
    assert_eq!(body["name"], "work");
    assert_eq!(body["persistent"], true);
    assert_eq!(body["env"]["EDITOR"], "vim");
    hv.update().await.unwrap();
    assert_eq!(
        request(&mut server, "/update/apply").await,
        json!({"confirmed":true,"dry_run":false})
    );
    let restarted = hv.restart().await.unwrap();
    assert_eq!(restarted.status, models::RestartStatus::Accepted);
    assert_eq!(
        restarted.authentication,
        models::RestartAuthentication::NewTokenRequired
    );
    let (parts, body) = server.received.recv().await.unwrap();
    assert_eq!(parts.method, "POST");
    assert_eq!(parts.uri.path(), "/restart");
    assert!(body.is_empty());
    hv.log(
        HostLogSource::Service,
        LogOptions {
            grep: Some("ready".into()),
            tail: Some(0),
            max_bytes: Some(64),
        },
    )
    .await
    .unwrap();
    let (parts, _) = server.received.recv().await.unwrap();
    assert_eq!(
        parts.uri.to_string(),
        "/host-logs/service?grep=ready&tail=0&max_bytes=64"
    );
    drop(hv);
    assert_eq!(vm.info().await.unwrap().id, "vm-1");
    request(&mut server, "/vms/vm-1/info").await;
}

#[tokio::test]
async fn cloned_vm_handles_resolve_names_once_even_concurrently() {
    let mut server = gateway().await;
    let hv = Hypervisor::new(&server.url, "private-token").unwrap();
    let vm = hv.vm(VmSelector::Name("work".into())).unwrap();
    let clone = vm.clone();
    assert_eq!(vm.id(), None);
    assert_eq!(vm.name(), Some("work"));
    let (first, second) = tokio::join!(vm.info(), clone.info());
    assert_eq!(first.unwrap().id, "vm-1");
    assert_eq!(second.unwrap().id, "vm-1");
    request(&mut server, "/vms/list").await;
    request(&mut server, "/vms/vm-1/info").await;
    request(&mut server, "/vms/vm-1/info").await;
    assert_eq!(clone.id(), Some("vm-1"));
    vm.stop().await.unwrap();
    request(&mut server, "/vms/vm-1/stop").await;
    assert!(server.received.try_recv().is_err());
}

#[tokio::test]
async fn missing_or_ambiguous_names_are_never_used_for_mutations() {
    for matches in [0, 2] {
        let body = json!({"sandboxes": vec![reply("getVmInfo"); matches]});
        let mut server = Server::reply(200, &serde_json::to_vec(&body).unwrap(), None).await;
        let vm = VM::new(&server.url, "private-token", VmSelector::Name("work".into())).unwrap();
        let error = vm.delete().await.unwrap_err();
        assert!(matches!(error, Error::VmLookup { name, matches: count } if name == "work" && count == matches));
        assert_eq!(vm.id(), None);
        request(&mut server, "/vms/list").await;
        assert!(server.received.try_recv().is_err());
    }
}

#[tokio::test]
async fn controls_and_resources_use_the_canonical_vm_routes() {
    let mut server = gateway().await;
    let vm = VM::new(&server.url, "private-token", VmSelector::Id("vm-1".into())).unwrap();
    assert_eq!(vm.name(), None);
    vm.exec("printf hello", Some(4)).await.unwrap();
    assert_eq!(
        request(&mut server, "/vms/vm-1/exec").await,
        json!({"command":"printf hello","timeout_secs":4})
    );
    vm.start().await.unwrap();
    request(&mut server, "/vms/vm-1/start").await;
    vm.pause().await.unwrap();
    request(&mut server, "/vms/vm-1/pause").await;
    vm.resume().await.unwrap();
    request(&mut server, "/vms/vm-1/resume").await;
    vm.stop().await.unwrap();
    request(&mut server, "/vms/vm-1/stop").await;
    vm.delete().await.unwrap();
    request(&mut server, "/vms/vm-1/delete").await;
    let fork = vm.fork("branch", Some("notes".into())).await.unwrap();
    assert_eq!(
        request(&mut server, "/vms/vm-1/fork").await,
        json!({"name":"branch","description":"notes"})
    );
    drop(vm);
    fork.info().await.unwrap();
    request(&mut server, "/vms/vm-1/info").await;
    fork.snapshots().list().await.unwrap();
    request(&mut server, "/vms/vm-1/snapshots/list").await;
    fork.snapshots().status().await.unwrap();
    request(&mut server, "/vms/vm-1/snapshots/status").await;
    fork.stats().summary().await.unwrap();
    request(&mut server, "/vms/vm-1/stats/summary").await;
    fork.stats().details().await.unwrap();
    request(&mut server, "/vms/vm-1/stats/detail").await;
    assert_eq!(fork.copy().from_vm("/test.bin").await.unwrap(), [0, 255, 13, 10]);
    let (parts, _) = server.received.recv().await.unwrap();
    assert_eq!(parts.uri.to_string(), "/vms/vm-1/files/content?path=%2Ftest.bin");
    fork.copy().to_vm("/test.bin", vec![0, 255, 13, 10]).await.unwrap();
    let (parts, body) = server.received.recv().await.unwrap();
    assert_eq!(parts.method, "POST");
    assert_eq!(parts.uri.to_string(), "/vms/vm-1/files/content?path=%2Ftest.bin");
    assert_eq!(body, [0, 255, 13, 10]);
}

#[tokio::test]
async fn query_options_preserve_wire_names_enums_and_root_listing() {
    let mut server = gateway().await;
    let vm = VM::new(&server.url, "private-token", VmSelector::Id("vm-1".into())).unwrap();
    vm.log(LogOptions {
        grep: Some("hello world".into()),
        tail: Some(2),
        max_bytes: Some(64),
    })
    .await
    .unwrap();
    vm.history(HistoryOptions {
        limit: Some(2),
        offset: Some(3),
        search: Some("printf".into()),
        layer: Some(HistoryLayerFilter::Exec),
    })
    .await
    .unwrap();
    vm.timeline(TimelineOptions {
        trace_id: Some("trace".into()),
        since: Some("1h".into()),
        limit: Some(1),
        layers: Some(vec![TimelineLayer::Net, TimelineLayer::Model]),
    })
    .await
    .unwrap();
    vm.changes(
        "cp-1",
        PageOptions {
            limit: Some(2),
            offset: Some(3),
        },
    )
    .await
    .unwrap();
    vm.list("/", None).await.unwrap();
    vm.list("/folder", Some(2)).await.unwrap();
    for expected in [
        "/vms/vm-1/logs?grep=hello+world&tail=2&max_bytes=64",
        "/vms/vm-1/history?limit=2&offset=3&search=printf&layer=exec",
        "/vms/vm-1/timeline?trace_id=trace&since=1h&limit=1&layers=net%2Cmodel",
        "/vms/vm-1/changes?checkpoint=cp-1&offset=3&limit=2",
        "/vms/vm-1/files/list",
        "/vms/vm-1/files/list?path=%2Ffolder&depth=2",
    ] {
        let (parts, _) = server.received.recv().await.unwrap();
        assert_eq!(parts.uri.to_string(), expected);
    }
}

#[tokio::test]
async fn invalid_create_or_selector_is_rejected_before_http() {
    let mut server = gateway().await;
    let hv = Hypervisor::new(&server.url, "private-token").unwrap();
    for options in [
        CreateOptions {
            vcpu: Some(0),
            ..Default::default()
        },
        CreateOptions {
            memory: Some(Memory::Megabytes(0)),
            ..Default::default()
        },
    ] {
        assert!(matches!(hv.create("code", options).await, Err(Error::InvalidInput(_))));
    }
    for selector in [VmSelector::Id(String::new()), VmSelector::Name(String::new())] {
        assert!(hv.vm(selector.clone()).is_err());
        assert!(VM::new(&server.url, "private-token", selector).is_err());
    }
    assert!(Hypervisor::new(&server.url, "").is_err());
    assert!(hv.with_timeout(Duration::ZERO).is_err());
    assert!(server.received.try_recv().is_err());
}

#[tokio::test]
async fn facade_deadlines_are_forwarded_and_http_errors_stay_typed() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let hv = Hypervisor::new(&url, "private-token")
        .unwrap()
        .with_timeout(Duration::from_millis(30))
        .unwrap();
    let result = tokio::time::timeout(Duration::from_secs(5), hv.info()).await.unwrap();
    assert!(matches!(result, Err(Error::Transport(error)) if error.is_timeout()));
    let vm = VM::new(&url, "private-token", VmSelector::Id("vm-1".into())).unwrap();
    assert!(vm.clone().with_timeout(Duration::ZERO).is_err());
    let vm = vm.with_timeout(Duration::from_millis(30)).unwrap();
    let result = tokio::time::timeout(Duration::from_secs(5), vm.info()).await.unwrap();
    assert!(matches!(result, Err(Error::Transport(error)) if error.is_timeout()));
    let server = Server::reply(401, b"denied", None).await;
    let vm = VM::new(&server.url, "private-token", VmSelector::Id("vm-1".into())).unwrap();
    assert!(matches!(vm.info().await, Err(Error::Http { status: 401, body }) if body == b"denied"));
}
