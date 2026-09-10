use super::*;
use crate::transport::CallOptions;

#[path = "fixture.rs"]
mod fixture;
use fixture::Case;

macro_rules! operation {
    ($name:ident, $id:literal $(, $params:ty)?) => {
        #[tokio::test]
        async fn $name() {
            for full in [false, true] {
                let case = Case::new($id, full);
                for (status, invalid) in [(200, None), (403, None), (200, Some(b"not-json".as_slice())), (200, Some(b"{}".as_slice()))] {
                    if case.binary && invalid.is_some() { continue; }
                    let mut server = case.server(status, invalid).await;
                    let client = server.client();
                    let result = super::$name(&client, $(&serde_json::from_value::<$params>(case.input.clone()).unwrap(),)? CallOptions::default()).await;
                    if status != 200 {
                        assert!(matches!(result, Err(crate::Error::Http { status: 403, .. })));
                    } else if invalid.is_some() {
                        assert!(matches!(result, Err(crate::Error::Json(_))), "{result:?}");
                    } else {
                        let actual = serde_json::to_value(result.unwrap()).unwrap();
                        if case.binary { assert_eq!(actual, serde_json::json!([0, 255, 13, 10])); }
                        else {
                            for (key, value) in case.response.as_object().unwrap() {
                                assert_eq!(&actual[key], value, "response field {key}");
                            }
                        }
                    }
                    case.assert_request(&mut server).await;
                }
            }
        }
    };
}

operation!(get_hypervisor_info, "getHypervisorInfo");
operation!(get_hypervisor_logs, "getHypervisorLogs", GetHypervisorLogsParams);
operation!(list_profiles, "listProfiles");
operation!(get_update_status, "getUpdateStatus");
operation!(update_hypervisor, "updateHypervisor", UpdateHypervisorParams);
operation!(create_vm, "createVm", CreateVmParams);
operation!(list_vms, "listVms");
operation!(get_vm_changes, "getVmChanges", GetVmChangesParams);
operation!(delete_vm, "deleteVm", DeleteVmParams);
operation!(exec_vm, "execVm", ExecVmParams);
operation!(download_vm_file, "downloadVmFile", DownloadVmFileParams);
operation!(upload_vm_file, "uploadVmFile", UploadVmFileParams);
operation!(list_vm_files, "listVmFiles", ListVmFilesParams);
operation!(fork_vm, "forkVm", ForkVmParams);
operation!(get_vm_history, "getVmHistory", GetVmHistoryParams);
operation!(get_vm_info, "getVmInfo", GetVmInfoParams);
operation!(get_vm_logs, "getVmLogs", GetVmLogsParams);
operation!(pause_vm, "pauseVm", PauseVmParams);
operation!(resume_vm, "resumeVm", ResumeVmParams);
operation!(list_vm_snapshots, "listVmSnapshots", ListVmSnapshotsParams);
operation!(
    get_vm_snapshots_status,
    "getVmSnapshotsStatus",
    GetVmSnapshotsStatusParams
);
operation!(start_vm, "startVm", StartVmParams);
operation!(get_vm_stats_detail, "getVmStatsDetail", GetVmStatsDetailParams);
operation!(get_vm_stats_summary, "getVmStatsSummary", GetVmStatsSummaryParams);
operation!(get_vm_status, "getVmStatus", GetVmStatusParams);
operation!(stop_vm, "stopVm", StopVmParams);
operation!(get_vm_timeline, "getVmTimeline", GetVmTimelineParams);

#[test]
fn every_contract_operation_has_http_cases() {
    let covered = include_str!("tests.rs")
        .split("\noperation!(")
        .skip(1)
        .filter_map(|call| call.split('"').nth(1).map(str::to_owned))
        .collect();
    assert_eq!(Case::operation_ids(), covered);
}

#[test]
fn typed_parameters_reject_unknown_enums_and_negative_unsigned_limits() {
    assert!(serde_json::from_value::<GetVmHistoryParams>(serde_json::json!({"id":"vm","layer":"made-up"})).is_err());
    assert!(serde_json::from_value::<GetVmHistoryParams>(serde_json::json!({"id":"vm","limit":-1})).is_err());
    assert!(serde_json::from_value::<GetHypervisorLogsParams>(serde_json::json!({"name":"/tmp/secret"})).is_err());
}
