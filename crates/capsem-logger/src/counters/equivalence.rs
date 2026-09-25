//! The writer's snapshot against the rows it counts.
//!
//! Every test here writes a session through the real writer, flushes it, and
//! holds the snapshot the ledger persisted to what the oracle aggregates from
//! the flushed rows. The op sequences are random but seeded, and include the
//! shapes that make counting hard: ops the schema refuses (so their batch
//! fails and is retried op by op), duplicate and orphan completions, asks
//! opened and resolved, and one event matching several rules.

use std::path::Path;

use serde_json::json;

use super::fixtures::*;
use super::oracle::counters_from_rows;
use crate::counters::LedgerCounters;
use crate::db::{snapshot_session_ledger, DbHandle};
use crate::writer::WriteOp;

/// A small seeded generator; the suite must replay the same sequence.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, bound: u64) -> u64 {
        self.next() % bound
    }

    fn pick<'a>(&mut self, items: &[&'a str]) -> &'a str {
        items[self.below(items.len() as u64) as usize]
    }
}

fn hex_id(value: u64) -> String {
    format!("{:012x}", value & 0xffff_ffff_ffff)
}

fn credential(value: u64) -> String {
    format!("credential:blake3:{:064x}", value)
}

/// One random op. Roughly one in twenty is refused by the schema.
fn random_op(rng: &mut Rng, step: u64) -> Vec<WriteOp> {
    let ts = 1_700_000_000.0 + step as f64;
    match rng.below(12) {
        0 => vec![net(
            rng.pick(&["allowed", "denied", "error", "redirected"]),
            rng.below(500),
            rng.below(900),
        )],
        1 => {
            let origin = rng.pick(&["native", "mcp_proxy", "local", "builtin"]);
            let tool = rng.pick(&["bash", "srv__search", "fetch_http", "write_file"]);
            vec![model(
                rng.pick(&["anthropic", "openai"]),
                [Some("m1"), Some("m2"), None][rng.below(3) as usize],
                rng.below(1000),
                rng.below(1000),
                rng.below(10_000) as f64 / 1_000_000.0 + 0.000_000_4,
                &[(tool, origin)],
            )]
        }
        2 => vec![mcp(
            rng.pick(&["tools/call", "tools/call", "tools/list"]),
            rng.pick(&["github", "linear"]),
            rng.pick(&["search", "create"]),
            rng.below(50),
        )],
        3 => vec![file(
            rng.pick(&["created", "modified", "deleted", "read"]),
            "/workspace/a",
        )],
        4 => vec![exec(rng.below(6))],
        5 => vec![exec_done(rng.below(8))],
        6 => vec![audit(rng.pick(&["/bin/ls", "/usr/bin/git", "/bin/sh"]), ts)],
        7 => {
            // "observed" is not an outcome the schema accepts: a refused op.
            let outcome = rng.pick(&["captured", "brokered", "injected", "injected", "observed"]);
            let provider = [Some("github"), Some("aws"), None][rng.below(3) as usize];
            vec![substitution(&credential(rng.below(3)), outcome, provider, ts)]
        }
        8 | 9 => {
            let event_id = hex_id(0xe000 + step);
            let payload = json!({
                "plugin_executions": [{"plugin_id": rng.pick(&["dlp", "broker"]), "stage": "pre",
                                        "applied": rng.below(2) == 0, "duration_us": rng.below(100)}],
                "detections": [{"source": "plugin", "plugin_id": "dlp"}],
                "credential_observations": [{"credential_ref": credential(rng.below(3)), "source": "header"}],
            });
            // One event matching one to three rules, recorded together.
            (0..=rng.below(3))
                .map(|rule_index| {
                    rule(
                        &event_id,
                        &format!("rule.{rule_index}"),
                        rng.pick(&["allow", "block", "ask"]),
                        rng.pick(&["none", "low", "high"]),
                        (step * 10 + rng.below(3)) as i64,
                        payload.clone(),
                    )
                })
                .collect()
        }
        10 => {
            let ask_id = hex_id(0xa000 + rng.below(6));
            vec![ask(&ask_id, rng.pick(&["pending", "pending", "approved", "denied"]))]
        }
        _ => {
            // An ask id the schema refuses.
            vec![ask("not-hex", "pending")]
        }
    }
}

async fn write_session(db: &DbHandle, seed: u64, steps: u64, first_step: u64) {
    let mut rng = Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
    for step in first_step..first_step + steps {
        for op in random_op(&mut rng, step) {
            db.write(op).await.unwrap();
        }
    }
}

async fn assert_snapshot_matches_rows(path: &Path, context: &str) -> LedgerCounters {
    let reader = DbHandle::open_external_reader(path).unwrap();
    let snapshot = reader.ledger_counters().await.unwrap();
    let oracle = counters_from_rows(&reader).await;
    assert_eq!(snapshot, oracle, "{context}: snapshot and rows disagree");
    snapshot
}

#[tokio::test]
async fn random_sessions_match_the_rows_they_wrote() {
    let mut seen = Vec::new();
    for seed in 0..12 {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.db");
        let db = DbHandle::open(&path).unwrap();
        write_session(&db, seed, 120, 0).await;
        db.flush().await.unwrap();
        seen.push(assert_snapshot_matches_rows(&path, &format!("seed {seed}")).await);
    }
    // Agreement on empty sections proves nothing. Every counter the
    // generator can reach must have been reached by some seed.
    let reached = |name: &str, hit: fn(&LedgerCounters) -> bool| {
        assert!(seen.iter().any(hit), "no seed reached {name}");
    };
    reached("net decisions", |c| c.net.denied > 0 && c.net.error > 0);
    reached("model usage", |c| {
        c.model.total.cost_micro_usd > 0 && c.model.by_model.len() > 1
    });
    reached("counted tools", |c| c.tools.by_tool.len() > 2);
    reached("mcp tools", |c| !c.tools.mcp.is_empty());
    reached("exec completions", |c| c.exec.completed > 0);
    reached("audit", |c| c.audit.by_exe.len() > 1);
    reached("rule matches", |c| c.security.by_rule.len() > 1);
    reached("open asks", |c| !c.security.open_asks.is_empty());
    reached("plugins", |c| {
        c.plugins.values().any(|p| p.applied > 0 && p.skipped > 0)
    });
    reached("credentials", |c| {
        c.credentials
            .values()
            .any(|k| k.injected_substitutions > 0 && k.observations > 0)
    });
}

#[tokio::test]
async fn a_reopened_ledger_resumes_counting_where_it_left_off() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.db");
    let db = DbHandle::open(&path).unwrap();
    write_session(&db, 7, 80, 0).await;
    db.flush().await.unwrap();
    drop(db);

    let db = DbHandle::open(&path).unwrap();
    // Exec ids restart with the process; a completion must not reach a start
    // an earlier writer left behind, and must not be counted as if it did.
    write_session(&db, 8, 80, 1_000).await;
    db.flush().await.unwrap();
    assert_snapshot_matches_rows(&path, "after reopen").await;
}

#[tokio::test]
async fn a_rolled_back_flush_leaves_the_snapshot_with_its_rows() {
    let _fault = crate::writer::DISK_FLUSH_FAULT_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.db");
    let db = DbHandle::open(&path).unwrap();
    write_session(&db, 3, 60, 0).await;
    db.flush().await.unwrap();
    let before = DbHandle::open_external_reader(&path)
        .unwrap()
        .ledger_counters()
        .await
        .unwrap();

    crate::writer::fail_disk_flushes_for_path_for_tests(&path, 1);
    write_session(&db, 4, 60, 500).await;
    assert!(db.flush().await.is_err(), "the injected flush failure must surface");
    // Neither the rows nor the snapshot reached the disk.
    assert_snapshot_matches_rows(&path, "after the failed flush").await;
    let unchanged = DbHandle::open_external_reader(&path)
        .unwrap()
        .ledger_counters()
        .await
        .unwrap();
    assert_eq!(unchanged, before, "a rolled-back flush must not publish counts");

    // The retry carries both halves.
    db.flush().await.unwrap();
    assert_snapshot_matches_rows(&path, "after the retried flush").await;
}

#[tokio::test]
async fn a_fork_carries_the_snapshot_of_the_rows_it_copied() {
    let src = tempfile::tempdir().unwrap();
    let dst = tempfile::tempdir().unwrap();
    let path = src.path().join("session.db");
    let db = DbHandle::open(&path).unwrap();
    write_session(&db, 11, 100, 0).await;
    db.flush().await.unwrap();
    snapshot_session_ledger(src.path(), dst.path()).unwrap();
    assert_snapshot_matches_rows(&dst.path().join("session.db"), "fork").await;

    // The fork keeps counting from the copy, not from zero.
    let forked = DbHandle::open(&dst.path().join("session.db")).unwrap();
    write_session(&forked, 12, 40, 2_000).await;
    forked.flush().await.unwrap();
    assert_snapshot_matches_rows(&dst.path().join("session.db"), "fork after writes").await;
}

#[tokio::test]
async fn a_fresh_ledger_reads_as_counting_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.db");
    let _db = DbHandle::open(&path).unwrap();
    let reader = DbHandle::open_external_reader(&path).unwrap();
    assert_eq!(reader.ledger_counters().await.unwrap(), Default::default());
}
