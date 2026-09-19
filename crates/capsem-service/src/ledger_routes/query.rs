//! How a ledger route reads a session ledger, and how it fails.
//!
//! Every route here opens the session's shared handle, runs its statements,
//! and maps rows onto ledger types through these helpers, so a route that
//! moves between the single-query rail and the batch rail keeps the log line
//! and the error message its callers already know.

use super::*;

pub(crate) fn ledger_route_error(
    vm_id: &str,
    ledger: &str,
    operation: &str,
    db_path: &StdPath,
    error: impl std::fmt::Display,
) -> AppError {
    let error = error.to_string();
    error!(
        vm_id,
        ledger,
        operation,
        db_path = %db_path.display(),
        error = %error,
        "session ledger route DB operation failed"
    );
    AppError(
        StatusCode::INTERNAL_SERVER_ERROR,
        format!("failed to {operation} {ledger} ledger for {vm_id}: {error}"),
    )
}

pub(crate) async fn open_ready_session_db(
    state: &ServiceState,
    vm_id: &str,
    ledger: &str,
    db_path: &StdPath,
) -> Result<Arc<capsem_logger::DbHandle>, AppError> {
    if !db_path.exists() {
        error!(
            vm_id,
            ledger,
            operation = "ready",
            db_path = %db_path.display(),
            "session ledger DB is absent"
        );
        return Err(ledger_route_error(vm_id, ledger, "ready", db_path, "session.db absent"));
    }
    let db = match state.session_db_handle(vm_id) {
        Some(handle) if handle.path() == db_path => handle,
        Some(handle) => {
            warn!(
                vm_id,
                ledger,
                operation = "replace_stale_session_db_handle",
                cached_db_path = %handle.path().display(),
                db_path = %db_path.display(),
                "session DB handle path did not match resolved session path"
            );
            state.unregister_session_db_handle(vm_id);
            let session_dir = db_path
                .parent()
                .ok_or_else(|| ledger_route_error(vm_id, ledger, "resolve session dir", db_path, "missing parent"))?;
            state
                .register_session_db_handle(vm_id, session_dir)
                .map_err(|error| ledger_route_error(vm_id, ledger, "open", db_path, error))?
        }
        None => {
            let session_dir = db_path
                .parent()
                .ok_or_else(|| ledger_route_error(vm_id, ledger, "resolve session dir", db_path, "missing parent"))?;
            let handle = state
                .register_session_db_handle(vm_id, session_dir)
                .map_err(|error| ledger_route_error(vm_id, ledger, "open", db_path, error))?;
            info!(
                vm_id,
                ledger,
                operation = "lazy_register_session_db_handle",
                db_path = %db_path.display(),
                "registered missing session DB handle for route"
            );
            handle
        }
    };
    db.ready()
        .await
        .map_err(|error| ledger_route_error(vm_id, ledger, "ready", db_path, error))?;
    Ok(db)
}

/// One failed ledger read, logged and mapped to the route's 500.
///
/// Every read of a session ledger reports its failure through here, whether
/// it went down the single-query rail or the batch one, so a route that moves
/// between them keeps the message its callers already know.
pub(crate) fn query_route_error(
    vm_id: &str,
    ledger: &str,
    operation: &str,
    query_name: &str,
    db_path: &StdPath,
    error: &str,
) -> AppError {
    error!(
        vm_id,
        ledger,
        operation,
        query_name,
        db_path = %db_path.display(),
        error = %error,
        "session ledger route DB query failed"
    );
    AppError(
        StatusCode::INTERNAL_SERVER_ERROR,
        format!("{ledger} ledger query {query_name} failed for {vm_id}: {error}"),
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn query_route_db_json(
    vm_id: &str,
    ledger: &str,
    operation: &str,
    query_name: &str,
    db_path: &StdPath,
    db: &capsem_logger::DbHandle,
    sql: &str,
    params: &[serde_json::Value],
) -> Result<serde_json::Value, AppError> {
    let raw = db
        .query(sql, params)
        .await
        .map_err(|error| query_route_error(vm_id, ledger, operation, query_name, db_path, &error))?;
    parse_query_json(vm_id, ledger, query_name, db_path, &raw)
}

/// Parse one statement's `{"columns":[...],"rows":[...]}` result.
///
/// Split out of `query_route_db_json` because a batch read has every
/// statement's raw result in hand at once and must fail on a bad one with the
/// same message a single read would have given.
pub(crate) fn parse_query_json(
    vm_id: &str,
    ledger: &str,
    query_name: &str,
    db_path: &StdPath,
    raw: &str,
) -> Result<serde_json::Value, AppError> {
    serde_json::from_str(raw).map_err(|error| {
        error!(
            vm_id,
            ledger,
            operation = "parse query json",
            query_name,
            db_path = %db_path.display(),
            error = %error,
            "session ledger route DB query returned invalid JSON"
        );
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("{ledger} ledger query {query_name} returned invalid json for {vm_id}: {error}"),
        )
    })
}

pub(crate) fn query_json_to_objects(raw: serde_json::Value) -> Vec<serde_json::Value> {
    let columns: Vec<String> = raw
        .get("columns")
        .and_then(|value| value.as_array())
        .map(|columns| {
            columns
                .iter()
                .filter_map(|value| value.as_str().map(ToOwned::to_owned))
                .collect()
        })
        .unwrap_or_default();
    let rows = raw
        .get("rows")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();
    let mut objects = Vec::with_capacity(rows.len());
    for row in rows {
        let values = row.as_array().cloned().unwrap_or_default();
        let mut object = serde_json::Map::new();
        for (index, column) in columns.iter().enumerate() {
            object.insert(
                column.clone(),
                values.get(index).cloned().unwrap_or(serde_json::Value::Null),
            );
        }
        objects.push(serde_json::Value::Object(object));
    }
    objects
}

pub(crate) async fn query_route_objects(
    vm_id: &str,
    ledger: &str,
    query_name: &str,
    db_path: &StdPath,
    db: &capsem_logger::DbHandle,
    sql: &str,
    params: &[serde_json::Value],
) -> Result<Vec<serde_json::Value>, AppError> {
    let raw = query_route_db_json(vm_id, ledger, "query", query_name, db_path, db, sql, params).await?;
    Ok(query_json_to_objects(raw))
}

pub(crate) async fn query_route_typed_rows<T>(
    vm_id: &str,
    ledger: &str,
    query_name: &str,
    db_path: &StdPath,
    db: &capsem_logger::DbHandle,
    sql: &str,
    params: &[serde_json::Value],
) -> Result<Vec<T>, AppError>
where
    T: DeserializeOwned,
{
    let objects = query_route_objects(vm_id, ledger, query_name, db_path, db, sql, params).await?;
    decode_query_rows(vm_id, ledger, query_name, db_path, objects)
}

/// Map already-fetched result objects onto the ledger type they describe.
///
/// Split out of `query_route_typed_rows` because a batch read has its rows in
/// hand before any of them is decoded, and must not have to re-query to reach
/// the same mapping and the same failure message.
pub(crate) fn decode_query_rows<T>(
    vm_id: &str,
    ledger: &str,
    query_name: &str,
    db_path: &StdPath,
    objects: Vec<serde_json::Value>,
) -> Result<Vec<T>, AppError>
where
    T: DeserializeOwned,
{
    objects
        .into_iter()
        .map(|object| {
            serde_json::from_value::<T>(object).map_err(|error| {
                error!(
                    vm_id,
                    ledger,
                    operation = "decode query rows",
                    query_name,
                    db_path = %db_path.display(),
                    error = %error,
                    "session ledger route DB query mapping failed"
                );
                AppError(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("{ledger} ledger query {query_name} mapping failed for {vm_id}: {error}"),
                )
            })
        })
        .collect()
}

pub(crate) fn main_ledger_route_error(
    ledger: &str,
    operation: &str,
    db_path: &StdPath,
    error: impl std::fmt::Display,
) -> AppError {
    let error = error.to_string();
    error!(
        ledger,
        operation,
        db_path = %db_path.display(),
        error = %error,
        "main ledger route DB operation failed"
    );
    AppError(
        StatusCode::INTERNAL_SERVER_ERROR,
        format!("failed to {operation} {ledger} main ledger: {error}"),
    )
}
