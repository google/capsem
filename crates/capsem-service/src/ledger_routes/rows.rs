//! Typed ledger-worker results mapped into route JSON objects.
use super::*;

pub(super) fn ledger_rows_to_objects(rows: capsem_logger::ledger_protocol::LedgerRows) -> Vec<serde_json::Value> {
    rows.rows
        .into_iter()
        .map(|values| {
            let mut values = values.into_iter();
            serde_json::Value::Object(
                rows.columns
                    .iter()
                    .map(|column| {
                        let value = values.next().map_or(serde_json::Value::Null, ledger_value_to_json);
                        (column.clone(), value)
                    })
                    .collect(),
            )
        })
        .collect()
}

fn ledger_value_to_json(value: capsem_logger::ledger_protocol::LedgerValue) -> serde_json::Value {
    match value {
        capsem_logger::ledger_protocol::LedgerValue::Null => serde_json::Value::Null,
        capsem_logger::ledger_protocol::LedgerValue::Integer(value) => value.into(),
        capsem_logger::ledger_protocol::LedgerValue::Real(value) => serde_json::json!(value),
        capsem_logger::ledger_protocol::LedgerValue::Text(value) => value.into(),
        capsem_logger::ledger_protocol::LedgerValue::Blob(value) => {
            serde_json::Value::Array(value.into_iter().map(serde_json::Value::from).collect())
        }
    }
}

pub(super) async fn query_route_objects(
    vm_id: &str,
    ledger: &str,
    query_name: &str,
    db_path: &StdPath,
    db: &session_db_handles::SessionLedger,
    query: capsem_logger::ledger_protocol::LedgerQuery,
    set: usize,
) -> Result<Vec<serde_json::Value>, AppError> {
    let mut sets = db.query(query).await.map_err(|error| {
        error!(
            vm_id,
            ledger,
            operation = "query",
            query_name,
            db_path = %db_path.display(),
            error = %error,
            "session ledger route DB query failed"
        );
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("{ledger} ledger query {query_name} failed for {vm_id}: {error}"),
        )
    })?;
    if set >= sets.len() {
        return Err(ledger_route_error(
            vm_id,
            ledger,
            query_name,
            db_path,
            format!("query returned {} result sets, missing set {set}", sets.len()),
        ));
    }
    Ok(ledger_rows_to_objects(sets.swap_remove(set)))
}

pub(super) async fn query_route_typed_rows<T>(
    vm_id: &str,
    ledger: &str,
    query_name: &str,
    db_path: &StdPath,
    db: &session_db_handles::SessionLedger,
    query: capsem_logger::ledger_protocol::LedgerQuery,
    set: usize,
) -> Result<Vec<T>, AppError>
where
    T: DeserializeOwned,
{
    let objects = query_route_objects(vm_id, ledger, query_name, db_path, db, query, set).await?;
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
