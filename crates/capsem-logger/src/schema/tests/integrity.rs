use super::*;

fn memory_integrity_database(name: &str) -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    create_tables(&conn).unwrap();
    create_memory_tables(&conn, &memory_uri_for_name(name)).unwrap();
    conn
}

#[test]
fn memory_sync_empty_flush_refuses_missing_tables_and_id_columns() {
    for (index, corruption) in [
        "DROP TABLE main.net_events",
        "DROP TABLE mem.net_events",
        "ALTER TABLE main.net_events RENAME COLUMN id TO lost_id",
        "ALTER TABLE mem.net_events RENAME COLUMN id TO lost_id",
    ]
    .iter()
    .enumerate()
    {
        let conn = memory_integrity_database(&format!("memory-sync-empty-{index}"));
        let watermarks = MemoryFlushWatermarks::new();
        assert!(flush_memory_tables_to_disk(&conn, ["net_events"], &watermarks)
            .unwrap()
            .is_empty());
        conn.execute_batch(corruption).unwrap();
        assert!(
            flush_memory_tables_to_disk(&conn, ["net_events"], &watermarks).is_err(),
            "a warmed empty flush acknowledged broken schema: {corruption}"
        );
    }
}

#[test]
fn memory_sync_missing_disk_table_preserves_pending_rows_until_explicit_repair() {
    let conn = memory_integrity_database("memory-sync-repair");
    let watermarks = initial_memory_flush_watermarks(&conn, ["net_events"]).unwrap();
    conn.execute_batch(
        "INSERT INTO mem.net_events(event_id,timestamp,domain,decision)
         VALUES('012345abcdef','t','fixture','allowed'); DROP TABLE main.net_events",
    )
    .unwrap();
    assert!(flush_memory_tables_to_disk(&conn, ["net_events"], &watermarks).is_err());
    let pending: i64 = conn
        .query_row("SELECT COUNT(*) FROM mem.net_events", [], |row| row.get(0))
        .unwrap();
    assert_eq!(pending, 1);
    assert_eq!(watermarks["net_events"], 0);
    create_tables(&conn).unwrap(); // Explicit fixture repair, never a production fallback.
    let advanced = flush_memory_tables_to_disk(&conn, ["net_events"], &watermarks).unwrap();
    assert_eq!(advanced["net_events"], 1);
    let event: String = conn
        .query_row("SELECT event_id FROM main.net_events", [], |row| row.get(0))
        .unwrap();
    assert_eq!(event, "012345abcdef");
    let pending: i64 = conn
        .query_row("SELECT COUNT(*) FROM mem.net_events", [], |row| row.get(0))
        .unwrap();
    assert_eq!(pending, 0);
}

#[test]
fn memory_sync_sequence_seed_refuses_missing_tables() {
    for schema in ["main", "mem"] {
        let conn = memory_integrity_database(&format!("memory-sync-seed-{schema}"));
        conn.execute_batch(&format!("DROP TABLE {schema}.net_events")).unwrap();
        assert!(
            seed_memory_sequences(&conn, ["net_events"]).is_err(),
            "missing {schema}.net_events was ignored"
        );
    }
}

#[test]
fn memory_sync_initial_watermark_refuses_missing_disk_table() {
    let conn = memory_integrity_database("memory-sync-watermark");
    conn.execute_batch("DROP TABLE main.net_events").unwrap();
    assert!(initial_memory_flush_watermarks(&conn, ["net_events"]).is_err());
}

#[test]
fn memory_sync_refuses_unknown_hot_table_names() {
    let conn = memory_integrity_database("memory-sync-unknown");
    let watermarks = MemoryFlushWatermarks::new();
    assert!(seed_memory_sequences(&conn, ["unknown_ledger"]).is_err());
    assert!(initial_memory_flush_watermarks(&conn, ["unknown_ledger"]).is_err());
    assert!(flush_memory_tables_to_disk(&conn, ["unknown_ledger"], &watermarks).is_err());
}

fn seed_integrity_rows(conn: &Connection) {
    let hash = format!("blake3:{}", "0123456789abcdef".repeat(4));
    conn.execute_batch(
        "INSERT INTO net_events(timestamp,domain,decision) VALUES('t','fixture','allowed');
         INSERT INTO model_calls(timestamp,provider,method,path) VALUES('t','fixture','POST','/');
         INSERT INTO tool_calls(call_index,call_id,tool_name) VALUES(0,'call','fixture');
         INSERT INTO tool_responses(model_call_id,call_id) VALUES(1,'call');
         INSERT INTO fs_events(timestamp,action,path) VALUES('t','read','/fixture');
         INSERT INTO exec_events(timestamp,exec_id,command,target) VALUES('t',1,'true','vm');
         INSERT INTO dns_events(timestamp,qname,qtype,qclass,rcode,decision)
             VALUES('t','fixture',1,1,0,'allowed');
         INSERT INTO audit_events(timestamp,pid,ppid,uid,exe,argv) VALUES('t',1,0,0,'fixture','[]');
         INSERT INTO security_rule_events(timestamp_unix_ms,event_id,event_type,rule_id,rule_action,rule_json)
             VALUES(0,'000000000001','http.request','fixture','allow','{}');
         INSERT INTO security_decision_events(timestamp_unix_ms,event_id,event_type,stage,actor,
             previous_decision,requested_decision,effective_decision)
             VALUES(0,'000000000001','http.request','rule','fixture','allow','allow','allow');
         INSERT INTO security_ask_events(timestamp_unix_ms,ask_id,event_id,event_type,rule_id,rule_name,status,rule_json)
             VALUES(0,'000000000001','000000000001','http.request','fixture','fixture','pending','{}');
         INSERT INTO transport_events(event_id,timestamp_unix_ms,event_type,event_json)
             VALUES('000000000001',0,'network.connect','{}');
         INSERT INTO host_events(timestamp_unix_ms,kind,actor,detail)
             VALUES(0,'service_started','fixture',x'80');
         INSERT INTO body_blocks(block_offset,raw_len,disk_len,sealed_at) VALUES(80,1,1,'t');",
    )
    .unwrap();
    for sql in [
        "INSERT INTO model_items(model_call_id,timestamp,provider,path,kind,item_index,content_hash)
         VALUES(1,'t','fixture','/','request',0,?1)",
        "INSERT INTO event_body_blobs(event_id,event_type,source_table,direction,original_bytes,
             stored_bytes,truncated,body_hash,block_offset,body_offset,body_len,created_at)
         VALUES('000000000001','http.request','net_events','request',1,1,0,?1,80,0,1,'t')",
        "INSERT INTO policy_mutation_events(timestamp_unix_ms,mutation_id,actor,category,filename,
             affected_path,target_kind,target_key,operation,old_hash,old_size,new_hash,new_size,status)
         VALUES(0,'000000000001','fixture','rules','fixture','/fixture','rule','fixture','update',?1,0,?1,0,'applied')",
    ] {
        let fingerprint = crate::content_digest(b"fixture content");
        let expected = if sql.contains("body_hash") || sql.contains("content_hash") {
            &fingerprint
        } else {
            &hash
        };
        conn.execute(sql, [expected]).unwrap();
    }
    conn.execute(
        "INSERT INTO substitution_events(timestamp,material_class,source,algorithm,substitution_ref,outcome)
         VALUES('t','credential','fixture','blake3',?1,'captured')",
        [format!("credential:{hash}")],
    )
    .unwrap();
}

#[test]
fn ledger_ids_reject_nul_suffixes_in_every_actual_column() {
    let conn = Connection::open_in_memory().unwrap();
    create_tables(&conn).unwrap();
    seed_integrity_rows(&conn);
    let mut checked = 0;
    for (table, columns) in REQUIRED_COLUMNS_FOR_TESTS {
        for column in *columns {
            if !matches!(*column, "event_id" | "ask_id" | "mutation_id") {
                continue;
            }
            checked += 1;
            let update = format!("UPDATE {table} SET {column}=?1");
            assert_eq!(conn.execute(&update, ["000000000001"]).unwrap(), 1, "{table}.{column}");
            let error = match conn.execute(&update, ["000000000001\0suffix"]) {
                Err(error) => error,
                Ok(_) => panic!("{table}.{column} accepted a NUL-suffixed ID"),
            };
            assert!(error.to_string().contains("CHECK"), "{table}.{column}: {error}");
        }
    }
    assert_eq!(checked, 17);
}

#[test]
fn host_events_do_not_require_hash_columns_or_a_content_hash_chain() {
    let conn = Connection::open_in_memory().unwrap();
    create_tables(&conn).unwrap();
    let columns = super::super::table_column_names(&conn, "main", "host_events").unwrap();
    assert!(!columns.iter().any(|name| name == "hash" || name == "prev_hash"));
    conn.execute(
        "INSERT INTO host_events(timestamp_unix_ms,kind,actor,detail) VALUES(1,'service_started','fixture',x'80')",
        [],
    )
    .unwrap();
    create_tables(&conn).unwrap();
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM host_events", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn obsolete_host_chain_schema_is_refused_without_migration() {
    let conn = Connection::open_in_memory().unwrap();
    create_tables(&conn).unwrap();
    conn.execute_batch("ALTER TABLE host_events ADD COLUMN hash BLOB")
        .unwrap();
    let error = create_tables(&conn).unwrap_err();
    assert!(error.to_string().contains("obsolete host hash-chain schema"));
    assert!(super::super::table_column_names(&conn, "main", "host_events")
        .unwrap()
        .iter()
        .any(|name| name == "hash"));
}

#[test]
fn stale_transport_id_checks_are_refused_without_rewriting_the_ledger() {
    let conn = Connection::open_in_memory().unwrap();
    create_tables(&conn).unwrap();
    let canonical: String = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE name='transport_events'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let stale = canonical.replace("length(CAST(event_id AS BLOB))", "length(event_id)");
    assert_ne!(stale, canonical);
    conn.execute_batch("DROP TABLE transport_events").unwrap();
    conn.execute_batch(&stale).unwrap();
    conn.execute(
        "INSERT INTO transport_events(event_id,timestamp_unix_ms,event_type,event_json)
         VALUES('000000000001',0,'network.connect','{}')",
        [],
    )
    .unwrap();
    for result in [
        validate_ready_schema(&conn),
        create_tables(&conn).map_err(|error| error.to_string()),
        create_memory_tables(&conn, &memory_uri_for_name("stale_transport_id_checks"))
            .map_err(|error| error.to_string()),
    ] {
        let error = result.expect_err("old transport ID checks must fail at each ledger boundary");
        assert!(error.contains("transport_events.event_id"), "{error}");
    }
    let unchanged: String = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE name='transport_events'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(unchanged, stale);
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM transport_events", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn digest_columns_reject_nonhex_and_nul_suffixes_in_the_actual_schema() {
    let conn = Connection::open_in_memory().unwrap();
    create_tables(&conn).unwrap();
    seed_integrity_rows(&conn);
    let tables = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let mut checked = 0;
    for table in tables {
        let columns = conn
            .prepare(&format!("PRAGMA table_info({table})"))
            .unwrap()
            .query_map([], |row| Ok((row.get::<_, String>(1)?, row.get::<_, bool>(3)?)))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        for (column, required) in columns {
            let (prefix, width) = match column.as_str() {
                "credential_ref" | "substitution_ref" => ("credential:blake3:", 64),
                "body_hash" | "content_hash" => ("xxh3-128:", 32),
                "old_hash" | "new_hash" => ("blake3:", 64),
                _ => continue,
            };
            checked += 1;
            let valid = format!("{prefix}{}", "0123456789abcdef".repeat(width / 16));
            let update = format!("UPDATE {table} SET {column}=?1");
            assert_eq!(conn.execute(&update, [&valid]).unwrap(), 1, "{table}.{column}");
            if !required {
                assert_eq!(conn.execute(&update, [None::<String>]).unwrap(), 1);
                conn.execute(&update, [&valid]).unwrap();
            }
            for invalid in [
                format!("{prefix}0{}g", "0".repeat(width - 2)),
                format!("{prefix}0{}A", "0".repeat(width - 2)),
                format!("{prefix}0{}é", "0".repeat(width - 2)),
                format!("{prefix}0{}\n", "0".repeat(width - 2)),
                format!("{valid}\0secret"),
                format!("{prefix}0{}\0", "0".repeat(width - 2)),
                format!("{prefix}{}", "0".repeat(width - 1)),
                format!("{prefix}{}", "0".repeat(width + 1)),
            ] {
                let error = match conn.execute(&update, [&invalid]) {
                    Err(error) => error,
                    Ok(_) => panic!("{table}.{column} accepted a noncanonical digest: {invalid:?}"),
                };
                assert!(error.to_string().contains("CHECK"), "{table}.{column}: {error}");
            }
        }
    }
    assert_eq!(
        checked, 16,
        "every checksum and credential-reference column is exercised"
    );
}

#[test]
fn stale_digest_checks_are_refused_without_rewriting_the_ledger() {
    let conn = Connection::open_in_memory().unwrap();
    create_tables(&conn).unwrap();
    let canonical: String = conn
        .query_row("SELECT sql FROM sqlite_master WHERE name='net_events'", [], |row| {
            row.get(0)
        })
        .unwrap();
    let stale = canonical
        .replace(&"[0-9a-f]".repeat(64), "[0-9a-f]*")
        .replace("length(CAST(credential_ref AS BLOB))", "length(credential_ref)");
    assert_ne!(stale, canonical);
    conn.execute_batch("DROP TABLE net_events").unwrap();
    conn.execute_batch(&stale).unwrap();
    conn.execute(
        "INSERT INTO net_events(timestamp,domain,decision,credential_ref) VALUES('t','fixture','allowed',?1)",
        [format!("credential:blake3:{}", "0".repeat(64))],
    )
    .unwrap();
    let ready = validate_ready_schema(&conn).expect_err("an unchanged column list cannot prove digest checks");
    assert!(ready.contains("net_events.credential_ref"), "{ready}");
    let writer = create_tables(&conn).expect_err("writer must not reopen or rewrite stale digest checks");
    assert!(writer.to_string().contains("net_events.credential_ref"), "{writer}");
    let memory = create_memory_tables(&conn, &memory_uri_for_name("stale_digest_checks"))
        .expect_err("memory staging must not inherit stale digest checks");
    assert!(memory.to_string().contains("net_events.credential_ref"), "{memory}");
    let unchanged: String = conn
        .query_row("SELECT sql FROM sqlite_master WHERE name='net_events'", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(unchanged, stale);
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM net_events", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        1
    );
}
