use super::*;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::time::Instant;

fn spec(cable: u32, last: u8) -> CableSpec {
    CableSpec {
        cable,
        address: Ipv4Addr::new(10, 128, cable as u8, last),
        prefix: 24,
    }
}

#[test]
fn a_cable_spec_makes_the_pump_arguments() {
    assert_eq!(
        spec(2, 9).arguments(),
        ["--cable", "2", "--address", "10.128.2.9", "--prefix", "24"]
    );
}

/// A stand-in pump: records its arguments, one line per start, and stays up
/// until killed.
fn stand_in(dir: &tempfile::TempDir) -> (std::path::PathBuf, std::path::PathBuf) {
    let log = dir.path().join("starts.log");
    let script = dir.path().join("pump.sh");
    let mut file = std::fs::File::create(&script).unwrap();
    writeln!(file, "#!/bin/sh\necho \"$$ $@\" >> '{}'\nexec sleep 600", log.display()).unwrap();
    drop(file);
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    (script, log)
}

/// The start lines so far, once there are at least `count`.
fn starts(log: &std::path::Path, count: usize) -> Vec<(u32, String)> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let lines: Vec<(u32, String)> = std::fs::read_to_string(log)
            .unwrap_or_default()
            .lines()
            .filter_map(|line| line.split_once(' '))
            .map(|(pid, arguments)| (pid.parse().unwrap(), arguments.to_string()))
            .collect();
        if lines.len() >= count {
            return lines;
        }
        assert!(
            Instant::now() < deadline,
            "only {} of {count} starts: {lines:?}",
            lines.len()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn alive(pid: u32) -> bool {
    nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid as i32), None).is_ok()
}

#[test]
fn plugging_starts_one_pump_per_cable_and_plugging_again_keeps_it() {
    let dir = tempfile::tempdir().unwrap();
    let (script, log) = stand_in(&dir);
    let cables = Cables::new(script);
    cables.plug(spec(1, 2));
    cables.plug(spec(2, 2));
    let started = starts(&log, 2);
    let mut arguments: Vec<&str> = started.iter().map(|(_, arguments)| arguments.as_str()).collect();
    arguments.sort_unstable();
    assert_eq!(
        arguments,
        [
            "--cable 1 --address 10.128.1.2 --prefix 24",
            "--cable 2 --address 10.128.2.2 --prefix 24"
        ]
    );
    cables.plug(spec(1, 2));
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(starts(&log, 2).len(), 2, "the same cable plugged again is left running");
    for (pid, _) in &started {
        assert!(alive(*pid));
    }
    cables.unplug(1);
    cables.unplug(2);
}

#[test]
fn unplugging_stops_that_cables_pump_for_good_and_leaves_the_others() {
    let dir = tempfile::tempdir().unwrap();
    let (script, log) = stand_in(&dir);
    let cables = Cables::new(script);
    cables.plug(spec(1, 2));
    cables.plug(spec(2, 2));
    let started = starts(&log, 2);
    let pid_of = |cable: &str| {
        started
            .iter()
            .find(|(_, arguments)| arguments.contains(cable))
            .unwrap()
            .0
    };
    cables.unplug(1);
    assert!(!alive(pid_of("--cable 1 ")), "the unplugged cable's pump is gone");
    assert!(alive(pid_of("--cable 2 ")), "the other cable's pump stays");
    std::thread::sleep(Duration::from_millis(1_500));
    assert_eq!(starts(&log, 2).len(), 2, "an unplugged pump is never restarted");
    cables.unplug(1);
    cables.unplug(99);
    cables.unplug(2);
    assert!(!alive(pid_of("--cable 2 ")));
}

#[test]
fn a_pump_that_dies_is_started_again() {
    let dir = tempfile::tempdir().unwrap();
    let (script, log) = stand_in(&dir);
    let cables = Cables::new(script);
    cables.plug(spec(1, 2));
    let (first, _) = starts(&log, 1)[0].clone();
    nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(first as i32),
        nix::sys::signal::Signal::SIGKILL,
    )
    .unwrap();
    let restarted = starts(&log, 2);
    assert_ne!(restarted[1].0, first);
    assert_eq!(restarted[1].1, restarted[0].1, "restarted with the same cable");
    cables.unplug(1);
}

#[test]
fn replugging_a_cable_with_a_new_address_restarts_its_pump() {
    let dir = tempfile::tempdir().unwrap();
    let (script, log) = stand_in(&dir);
    let cables = Cables::new(script);
    cables.plug(spec(1, 2));
    let (first, _) = starts(&log, 1)[0].clone();
    cables.plug(spec(1, 7));
    let restarted = starts(&log, 2);
    assert!(!alive(first), "the pump with the old address is gone");
    assert_eq!(restarted[1].1, "--cable 1 --address 10.128.1.7 --prefix 24");
    cables.unplug(1);
}
