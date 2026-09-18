//! The reader worker: one thread owning one query-only connection.
//!
//! Route code never touches this thread. It sends intent down the channel and
//! the worker answers; how the ledger is opened, how its freshness is decided
//! and how the SQL is executed all stay here.

use super::*;

pub(super) fn reader_loop(path: PathBuf, rx: mpsc::Receiver<ReadRequest>) {
    let started = Instant::now();
    // WAL makes the file readable while its writer commits, so this reader
    // queries `main` and holds no copy of it, whichever process writes.
    let reader = match DbReader::open(&path) {
        Ok(reader) => reader,
        Err(error) => {
            tracing::error!(
                db_path = %path.display(),
                operation = "reader_worker_open",
                error = %error,
                "session db reader worker failed"
            );
            return;
        }
    };
    tracing::debug!(
        db_path = %path.display(),
        operation = "reader_worker_open",
        duration_ms = elapsed_ms(started),
        "session db reader worker opened"
    );

    while let Ok(request) = rx.recv() {
        match request {
            ReadRequest::Ready { reply } => {
                let started = Instant::now();
                let result =
                    observe_change(&reader).and_then(|observed| reader.ready().map(|()| commit(&reader, observed)));
                match &result {
                    Ok(_) => tracing::debug!(
                        db_path = %path.display(),
                        operation = "ready_execute",
                        duration_ms = elapsed_ms(started),
                        "session db readiness completed"
                    ),
                    Err(error) => tracing::error!(
                        db_path = %path.display(),
                        operation = "ready_execute",
                        duration_ms = elapsed_ms(started),
                        error = %error,
                        "session db readiness failed"
                    ),
                }
                let _ = reply.send(result);
            }
            ReadRequest::Query { sql, params, reply } => {
                let started = Instant::now();
                let sql_hash = sql_fingerprint(&sql);
                let params_count = params.len();
                // The row data needs no freshness step -- the reader's `main`
                // is the file. The observation is still made and
                // committed, so every read path moves the handle's epochs the
                // same way and no caller can be told the ledger stood still.
                let result = observe_change(&reader).and_then(|observed| {
                    let executed = reader.query_raw_with_params(&sql, &params);
                    record_query_metrics("execute", started, params_count, &executed);
                    executed.map(|value| Observed {
                        changed: commit(&reader, observed),
                        value,
                    })
                });
                match &result {
                    Ok(_) => tracing::debug!(
                        db_path = %path.display(),
                        operation = "query_execute",
                        sql_hash,
                        params_count,
                        duration_ms = elapsed_ms(started),
                        "session db query completed"
                    ),
                    Err(error) => tracing::error!(
                        db_path = %path.display(),
                        operation = "query_execute",
                        sql_hash,
                        params_count,
                        duration_ms = elapsed_ms(started),
                        error = %error,
                        "session db query failed"
                    ),
                }
                let _ = reply.send(result);
            }
            ReadRequest::QueryMany {
                queries,
                cache_valid,
                reply,
            } => {
                let started = Instant::now();
                let query_count = queries.len();
                let params_count: usize = queries.iter().map(|(_, params)| params.len()).sum();
                let result = observe_change(&reader).and_then(|observed| {
                    if cache_valid && observed.is_none() {
                        // The polled aggregates of an idle session: the caller
                        // already holds this answer and the ledger has not moved.
                        return Ok(QueryManyReply::CacheStillValid);
                    }
                    execute_query_many(&reader, queries).map(|results| QueryManyReply::Executed {
                        changed: commit(&reader, observed),
                        results,
                    })
                });
                match &result {
                    Ok(_) => tracing::debug!(
                        db_path = %path.display(),
                        operation = "query_many_execute",
                        query_count,
                        params_count,
                        duration_ms = elapsed_ms(started),
                        "session db query batch completed"
                    ),
                    Err(error) => tracing::error!(
                        db_path = %path.display(),
                        operation = "query_many_execute",
                        query_count,
                        params_count,
                        duration_ms = elapsed_ms(started),
                        error = %error,
                        "session db query batch failed"
                    ),
                }
                let _ = reply.send(result);
            }
            #[cfg(test)]
            ReadRequest::Introspect { reply } => {
                let result = reader
                    .attached_schemas()
                    .and_then(|attached_schemas| {
                        Ok(ReaderIntrospection {
                            attached_schemas,
                            disk_syncs: reader.disk_syncs(),
                            queries_executed: reader.queries_executed(),
                            busy_timeout_ms: reader.busy_timeout_ms()?,
                        })
                    })
                    .map_err(|error| error.to_string());
                let _ = reply.send(result);
            }
            ReadRequest::Shutdown => {
                tracing::debug!(
                    db_path = %path.display(),
                    operation = "reader_worker_shutdown",
                    "session db reader worker shutting down"
                );
                break;
            }
        }
    }
}

/// The ledger's new `data_version`, when another connection committed since
/// this worker last looked.
///
/// SQLite's own `data_version` is the single answer to "is what I cached still
/// current", for a handle that owns the writer as much as for one that does
/// not: the writer's connection is another connection, and its flush is the
/// commit that makes accepted rows readable.
fn observe_change(reader: &DbReader) -> DbResult<Option<i64>> {
    reader.observe_data_version().map_err(|error| error.to_string())
}

/// Commit an observation now that the work depending on it has succeeded, and
/// report whether there was one.
///
/// Recording it any earlier would strand the change: a request that observed a
/// commit and then failed would leave the reader believing it had accounted
/// for rows it never read, and every later poll would answer from a cache
/// built before them -- until the writer happened to commit again.
fn commit(reader: &DbReader, observed: Option<i64>) -> bool {
    match observed {
        Some(data_version) => {
            reader.commit_observed_version(data_version);
            true
        }
        None => false,
    }
}

fn execute_query_many(reader: &DbReader, queries: Vec<DbQueryOwned>) -> DbResult<Vec<String>> {
    let mut results = Vec::with_capacity(queries.len());
    for (sql, params) in queries {
        let started = Instant::now();
        let result = reader.query_raw_with_params(&sql, &params);
        record_query_metrics("execute_many", started, params.len(), &result);
        results.push(result?);
    }
    Ok(results)
}
