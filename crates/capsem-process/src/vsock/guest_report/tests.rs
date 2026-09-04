use super::*;

fn stage(name: &str, ms: u64) -> BootStage {
    BootStage {
        name: name.to_string(),
        duration_ms: ms,
    }
}

#[test]
fn keeps_well_formed_stages_in_order() {
    let kept = record_boot_timing(vec![stage("kernel", 1200), stage("erofs", 20), stage("agent_start", 0)]);
    let names: Vec<&str> = kept.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["kernel", "erofs", "agent_start"]);
}

#[test]
fn drops_names_that_are_empty_too_long_or_not_identifiers() {
    let kept = record_boot_timing(vec![
        stage("", 1),
        stage(&"x".repeat(65), 1),
        stage("net-proxy", 1),
        stage("<script>", 1),
        stage("ok_stage", 1),
    ]);
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].name, "ok_stage");
}

#[test]
fn drops_absurd_durations_and_caps_the_stage_count() {
    let mut stages: Vec<BootStage> = (0..40).map(|i| stage(&format!("s{i}"), i)).collect();
    stages.push(stage("forever", MAX_STAGE_MS + 1));
    let kept = record_boot_timing(stages);
    assert_eq!(kept.len(), MAX_BOOT_STAGES);
    assert!(kept.iter().all(|s| s.name != "forever"));
}

#[test]
fn only_pong_is_a_liveness_reply() {
    assert!(is_guest_liveness_message(&GuestToHost::Pong));
    assert!(!is_guest_liveness_message(&GuestToHost::BootReady));
    assert!(!is_guest_liveness_message(&GuestToHost::ShutdownComplete));
}
