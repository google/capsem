use axum::body::Body;
use axum::http::Response;
use serde_json::{json, Value};

use crate::test_contract::Case;
use crate::test_gateway::Server;

pub fn reply(operation: &str) -> Value {
    let mut value = Case::new(operation, false).response;
    if value.get("id").is_some() {
        value["id"] = json!("vm-1");
    }
    if value.get("name").is_some() || operation == "getVmInfo" {
        value["name"] = json!("work");
    }
    if operation == "listVms" {
        value["sandboxes"] = json!([reply("getVmInfo")]);
    }
    value
}

pub async fn gateway() -> Server {
    Server::respond(|parts| {
        let path = parts.uri.path();
        let operation = match path {
            "/status" => "getHypervisorInfo",
            "/vms/list" => "listVms",
            "/vms/create" => "createVm",
            "/host-logs/service" => "getHypervisorLogs",
            "/update/apply" => "updateHypervisor",
            "/vms/vm-1/info" => "getVmInfo",
            "/vms/vm-1/exec" => "execVm",
            "/vms/vm-1/start" => "startVm",
            "/vms/vm-1/stop" => "stopVm",
            "/vms/vm-1/pause" => "pauseVm",
            "/vms/vm-1/resume" => "resumeVm",
            "/vms/vm-1/delete" => "deleteVm",
            "/vms/vm-1/fork" => "forkVm",
            "/vms/vm-1/logs" => "getVmLogs",
            "/vms/vm-1/history" => "getVmHistory",
            "/vms/vm-1/files/list" => "listVmFiles",
            "/vms/vm-1/changes" => "getVmChanges",
            "/vms/vm-1/timeline" => "getVmTimeline",
            "/vms/vm-1/snapshots/list" => "listVmSnapshots",
            "/vms/vm-1/snapshots/status" => "getVmSnapshotsStatus",
            "/vms/vm-1/stats/summary" => "getVmStatsSummary",
            "/vms/vm-1/stats/detail" => "getVmStatsDetail",
            "/vms/vm-1/files/content" if parts.method == "POST" => "uploadVmFile",
            "/vms/vm-1/files/content" => return Response::new(Body::from(vec![0, 255, 13, 10])),
            _ => {
                return Response::builder()
                    .status(404)
                    .body(Body::from("missing fixture"))
                    .unwrap()
            }
        };
        Response::new(Body::from(serde_json::to_vec(&reply(operation)).unwrap()))
    })
    .await
}

pub async fn request(server: &mut Server, path: &str) -> Value {
    let (parts, body) = server.received.recv().await.unwrap();
    assert_eq!(parts.uri.path(), path);
    assert_eq!(parts.headers["authorization"], "Bearer private-token");
    serde_json::from_slice(&body).unwrap_or(Value::Null)
}
