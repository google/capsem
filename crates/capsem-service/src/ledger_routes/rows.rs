//! Ledger query results as route rows.
//!
//! The DB handle answers a query as `{"columns":[...],"rows":[[...]]}` JSON;
//! these turn that into the objects and typed rows routes return.
use super::*;

/// A ledger query's `{"columns":[...],"rows":[[...]]}` result, as parsed.
#[derive(Deserialize)]
struct QueryRows {
    columns: Vec<String>,
    rows: Vec<Vec<serde_json::Value>>,
}

/// One JSON object per row of a ledger query result, keyed by column.
///
/// Values are moved out of the parsed result, never cloned: a polled route's
/// rows used to be parsed into a tree, deep-copied out of it, then copied
/// again into objects.
pub(super) fn query_json_to_objects(raw: &str) -> serde_json::Result<Vec<serde_json::Value>> {
    let QueryRows { columns, rows } = serde_json::from_str(raw)?;
    Ok(rows
        .into_iter()
        .map(|values| {
            let mut values = values.into_iter();
            serde_json::Value::Object(
                columns
                    .iter()
                    .map(|column| (column.clone(), values.next().unwrap_or(serde_json::Value::Null)))
                    .collect(),
            )
        })
        .collect())
}

/// [`query_json_to_objects`], naming the route and query when the result is
/// not the shape the DB handle promises.
pub(super) fn route_query_objects(
    vm_id: &str,
    ledger: &str,
    query_name: &str,
    db_path: &StdPath,
    raw: &str,
) -> Result<Vec<serde_json::Value>, AppError> {
    query_json_to_objects(raw).map_err(|error| {
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

pub(super) async fn query_route_objects(
    vm_id: &str,
    ledger: &str,
    query_name: &str,
    db_path: &StdPath,
    db: &capsem_logger::DbHandle,
    sql: &str,
    params: &[serde_json::Value],
) -> Result<Vec<serde_json::Value>, AppError> {
    // Down the batch rail, which carries its own readiness check and answers
    // a repeat fetch of a ledger that has not moved from the handle's cache
    // instead of executing the statement again.
    let raw = db
        .query_many(vec![(sql.to_string(), params.to_vec())])
        .await
        .and_then(|results| {
            let [raw]: [String; 1] = results
                .try_into()
                .map_err(|results: Vec<String>| format!("query returned {} results, expected 1", results.len()))?;
            Ok(raw)
        });
    let raw = raw.map_err(|error| {
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
    route_query_objects(vm_id, ledger, query_name, db_path, &raw)
}

pub(super) async fn query_route_typed_rows<T>(
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
