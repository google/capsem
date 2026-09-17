use std::time::Duration;

use serde_json::json;

use crate::models::{HistoryLayerFilter, HostLogSource, TimelineLayer};
use crate::test_gateway::Server;
use crate::*;

mod fixture;
mod interactions;
use fixture::{gateway, reply, request};

#[tokio::test]
async fn hypervisor_defaults_overrides_update_and_vm_handle_lifetime() {
    let mut server = gateway().await;
    let hv = Hypervisor::new(&server.url, "private-token").unwrap();
    hv.info().await.unwrap();
    request(&mut server, "/status").await;
    assert_eq!(hv.list().await.unwrap().sandboxes[0].id, "vm-1");
    request(&mut server, "/vms/list").await;
    let mut profile = hv.profiles().list().await.unwrap().remove(0);
    request(&mut server, "/profiles/list").await;
    for name in [None, Some(String::new())] {
        let vm = hv
            .create(CreateOptions {
                name,
                ..Default::default()
            })
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
    let network = hv.networks().create("team").await.unwrap();
    request(&mut server, "/networks").await;
    profile.id = "co-work".into();
    let vm = hv
        .create(CreateOptions {
            profile: Some(profile),
            name: Some("work".into()),
            cpus: Some(4),
            memory: Some(8),
            env: Some([("EDITOR".into(), "vim".into())].into()),
            networks: vec![network.clone()],
            ..Default::default()
        })
        .await
        .unwrap();
    let body = request(&mut server, "/vms/create").await;
    assert_eq!(body["ram_mb"], 8192);
    assert_eq!(body["cpus"], 4);
    assert_eq!(body["name"], "work");
    assert_eq!(body["persistent"], true);
    assert_eq!(body["env"]["EDITOR"], "vim");
    assert_eq!(body["networks"], json!([network.name]));
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
async fn container_and_port_resources_hide_wire_exposure_details() {
    let mut server = gateway().await;
    let hv = Hypervisor::new(&server.url, "private-token").unwrap();
    let registry = Registry {
        username: Some("robot".into()),
        password: Some("registry-secret".into()),
        ca_pem: None,
    };
    assert!(!format!("{registry:?}").contains("registry-secret"));
    let vm = hv
        .create(CreateOptions {
            env: Some([("MODE".into(), "preview".into())].into()),
            image: Some("docker://busybox:latest".into()),
            registry: Some(registry),
            ..Default::default()
        })
        .await
        .unwrap();
    let create = request(&mut server, "/vms/create").await;
    assert_eq!(create["env"], serde_json::Value::Null);
    assert_eq!(create["container"]["image"], "docker://busybox:latest");
    assert_eq!(create["container"]["env"]["MODE"], "preview");
    assert_eq!(create["container"]["registry"]["username"], "robot");
    assert_eq!(create["container"]["registry"]["password"], "registry-secret");
    assert_eq!(create["container"]["attach"], false);
    vm.container().status().await.unwrap();
    request(&mut server, "/vms/vm-1/container").await;
    let port = vm.ports().open(8080).await.unwrap();
    request(&mut server, "/vms/vm-1/exposures").await;
    assert!(!port.authenticate);
    let authenticated = vm
        .ports()
        .open_with(
            3000,
            PortOptions {
                host: 0,
                authenticate: true,
            },
        )
        .await
        .unwrap();
    request(&mut server, "/vms/vm-1/exposures").await;
    request(&mut server, "/vms/vm-1/exposures/vm-1/preview-session").await;
    assert!(authenticated.authenticate);
    assert!(authenticated.url.is_some());
    vm.ports().list().await.unwrap();
    request(&mut server, "/vms/vm-1/exposures").await;
    vm.ports().close(&port).await.unwrap();
    request(&mut server, "/vms/vm-1/exposures/vm-1").await;
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
    assert_eq!(fork.files().read("/test.bin").await.unwrap(), [0, 255, 13, 10]);
    let (parts, _) = server.received.recv().await.unwrap();
    assert_eq!(parts.uri.to_string(), "/vms/vm-1/files/content?path=%2Ftest.bin");
    fork.files().write("/test.bin", vec![0, 255, 13, 10]).await.unwrap();
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
    vm.files()
        .history(
            "cp-1",
            PageOptions {
                limit: Some(2),
                offset: Some(3),
            },
        )
        .await
        .unwrap();
    vm.files().list("/", None).await.unwrap();
    vm.files().list("/folder", Some(2)).await.unwrap();
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
            cpus: Some(0),
            ..Default::default()
        },
        CreateOptions {
            memory: Some(0),
            ..Default::default()
        },
    ] {
        assert!(matches!(hv.create(options).await, Err(Error::InvalidInput(_))));
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
async fn network_resource_uses_typed_routes_put_and_cursor_logs() {
    let mut server = gateway().await;
    let hv = Hypervisor::new(&server.url, "private-token").unwrap();
    let created = hv.networks().create("team").await.unwrap();
    request(&mut server, "/networks").await;
    assert_eq!(hv.networks().list().await.unwrap()[0].id, "net-1");
    request(&mut server, "/networks").await;
    hv.networks().inspect(&created.id).await.unwrap();
    request(&mut server, &format!("/networks/{}", created.id)).await;
    let vm = hv.vm(VmSelector::Id("vm-1".into())).unwrap();
    assert_eq!(vm.networks().list().await.unwrap()[0].id, "net-1");
    request(&mut server, "/networks").await;
    vm.networks().attach(&created).await.unwrap();
    let (parts, _) = server.received.recv().await.unwrap();
    assert_eq!(parts.method, "PUT");
    assert_eq!(parts.uri.path(), format!("/networks/{}/members/vm-1", created.id));
    vm.networks().detach(&created).await.unwrap();
    let (parts, _) = server.received.recv().await.unwrap();
    assert_eq!(parts.method, "DELETE");
    hv.networks()
        .logs(
            &created,
            NetworkLogOptions {
                cursor: Some("next".into()),
                limit: Some(4),
                event_type: Some("network.connect".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let (parts, _) = server.received.recv().await.unwrap();
    assert_eq!(parts.uri.query(), Some("cursor=next&limit=4&type=network.connect"));
    hv.networks().delete(&created).await.unwrap();
    let (parts, _) = server.received.recv().await.unwrap();
    assert_eq!(parts.method, "DELETE");
}

#[tokio::test]
async fn diagnostics_persistence_and_profile_mcp_use_typed_routes() {
    let mut server = gateway().await;
    let hv = Hypervisor::new(&server.url, "private-token").unwrap();
    let mut profile = hv.profiles().list().await.unwrap().remove(0);
    request(&mut server, "/profiles/list").await;
    profile.id = "co-work".into();
    hv.run(
        "printf hello",
        RunOptions {
            profile: Some(profile),
            timeout_secs: Some(4),
            cpus: Some(2),
            memory: Some(1),
            env: Some([("EDITOR".into(), "vim".into())].into()),
        },
    )
    .await
    .unwrap();
    assert_eq!(
        request(&mut server, "/run").await,
        json!({"command":"printf hello","profile_id":"co-work","timeout_secs":4,"ram_mb":1024,"cpus":2,"env":{"EDITOR":"vim"}})
    );
    hv.debug()
        .panics(DiagnosticOptions {
            since: Some("1h".into()),
            limit: Some(4),
        })
        .await
        .unwrap();
    let (parts, _) = server.received.recv().await.unwrap();
    assert_eq!(parts.uri.to_string(), "/panics?since=1h&limit=4");
    hv.debug()
        .triage(TriageOptions {
            since: Some("30m".into()),
            limit: Some(2),
            vm_id: Some("vm-1".into()),
        })
        .await
        .unwrap();
    let (parts, _) = server.received.recv().await.unwrap();
    assert_eq!(parts.uri.to_string(), "/triage?since=30m&limit=2&id=vm-1");
    hv.purge(true).await.unwrap();
    assert_eq!(request(&mut server, "/purge").await, json!({"all":true}));

    let vm = hv.vm(VmSelector::Id("vm-1".into())).unwrap();
    vm.persist("saved").await.unwrap();
    assert_eq!(request(&mut server, "/vms/vm-1/save").await, json!({"name":"saved"}));

    let profile = hv.profiles().list().await.unwrap().remove(0);
    request(&mut server, "/profiles/list").await;
    let mcp = hv.profiles().mcp(&profile);
    mcp.info().await.unwrap();
    request(&mut server, "/profiles/code/mcp/info").await;
    mcp.servers().await.unwrap();
    request(&mut server, "/profiles/code/mcp/servers/list").await;
    mcp.default_permission().await.unwrap();
    request(&mut server, "/profiles/code/mcp/default/info").await;
    let mcp_server = mcp.get("filesystem").await.unwrap();
    request(&mut server, "/profiles/code/mcp/servers/list").await;
    mcp_server.tools().list().await.unwrap();
    request(&mut server, "/profiles/code/mcp/servers/filesystem/tools/list").await;
    mcp_server.refresh().await.unwrap();
    request(&mut server, "/profiles/code/mcp/servers/filesystem/refresh").await;
    mcp_server.tools().call("read", json!({"path":"/tmp/a"})).await.unwrap();
    assert_eq!(
        request(&mut server, "/profiles/code/mcp/servers/filesystem/tools/read/call").await,
        json!({"path":"/tmp/a"})
    );
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
