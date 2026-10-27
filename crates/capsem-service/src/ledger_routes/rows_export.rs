//! `GET /vms/{id}/export/rows.ndjson.gz`: streamed rows-only session ledger dump.
//!
//! Exports the session ledger's SQLite rows as gzipped NDJSON (`capsem.rows.v1`),
//! complementing `GET /vms/{id}/bodies/export.warc.gz` (which carries archived
//! payloads). Line 1 is a JSON header naming the format, version, VM id,
//! export timestamp, and ordered table list; subsequent lines are
//! `{"table":"<name>","row":{...}}` ordered by `id ASC` per table.

use std::collections::BTreeMap;
use std::time::Duration;

use capsem_api::ExportRowsQuery;
use capsem_logger::ledger_protocol::{
    LedgerQuery, DEFAULT_EXPORT_LEDGER_TABLES, EXPORTABLE_LEDGER_TABLES, MAX_LEDGER_QUERY_ROWS,
};
use capsem_logger::{now_iso, GzipChunkEncoder};

use super::rows::query_route_objects;
use super::*;

const EXPORT_CHANNEL_CHUNKS: usize = 4;
const EXPORT_BLOCKED_WRITE: Duration = Duration::from_secs(30);
const MAX_EXPORT_PAGE_BUF_BYTES: usize = 64 * 1024;

pub(crate) fn parse_export_tables(raw: Option<&str>) -> Result<Vec<String>, AppError> {
    let Some(raw) = raw else {
        return Ok(DEFAULT_EXPORT_LEDGER_TABLES
            .iter()
            .map(|table| (*table).to_string())
            .collect());
    };
    let mut tables = Vec::new();
    for segment in raw.split(',') {
        let table = segment.trim();
        if table.is_empty() || !EXPORTABLE_LEDGER_TABLES.contains(&table) {
            return Err(AppError::new(
                StatusCode::BAD_REQUEST,
                format!("unknown or disallowed export table: {table:?}"),
            ));
        }
        if !tables.iter().any(|existing| existing == table) {
            tables.push(table.to_string());
        }
    }
    if tables.is_empty() {
        return Err(AppError::new(
            StatusCode::BAD_REQUEST,
            "tables parameter must name at least one exportable ledger table".to_string(),
        ));
    }
    Ok(tables)
}

async fn send_export_chunk(
    chunks: &tokio::sync::mpsc::Sender<Result<Bytes, std::io::Error>>,
    vm_id: &str,
    chunk: Vec<u8>,
) -> bool {
    if chunk.is_empty() {
        return true;
    }
    match tokio::time::timeout(EXPORT_BLOCKED_WRITE, chunks.send(Ok(Bytes::from(chunk)))).await {
        Ok(Ok(())) => true,
        Ok(Err(_)) => false,
        Err(_) => {
            error!(
                route = "/vms/{id}/export/rows.ndjson.gz",
                vm_id,
                "rows export response was blocked for 30 seconds"
            );
            false
        }
    }
}

async fn flush_page_buf(
    encoder: &mut GzipChunkEncoder,
    page_buf: &mut Vec<u8>,
    chunks: &tokio::sync::mpsc::Sender<Result<Bytes, std::io::Error>>,
    vm_id: &str,
    bytes_written: &mut u64,
) -> Result<bool, String> {
    if page_buf.is_empty() {
        return Ok(true);
    }
    encoder.write_all(page_buf).map_err(|error| error.to_string())?;
    page_buf.clear();
    let chunk = encoder.flush_chunk().map_err(|error| error.to_string())?;
    *bytes_written += chunk.len() as u64;
    Ok(send_export_chunk(chunks, vm_id, chunk).await)
}

async fn snapshot_table_watermarks(
    vm_id: &str,
    db_path: &StdPath,
    db: &session_db_handles::SessionLedger,
    tables: &[String],
) -> Result<BTreeMap<String, i64>, String> {
    let rows = query_route_objects(
        vm_id,
        "rows_export",
        "table_watermarks",
        db_path,
        db,
        LedgerQuery::TableWatermarks {
            tables: tables.to_vec(),
        },
        0,
    )
    .await
    .map_err(AppError::into_message)?;
    let mut watermarks = BTreeMap::new();
    for row in rows {
        let table_name = row
            .get("table_name")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| "table watermark row is missing string table_name".to_string())?;
        let max_id = row
            .get("max_id")
            .and_then(serde_json::Value::as_i64)
            .ok_or_else(|| format!("table watermark row for {table_name} is missing integer max_id"))?;
        watermarks.insert(table_name.to_string(), max_id);
    }
    Ok(watermarks)
}

async fn stream_rows_export(
    vm_id: String,
    db_path: PathBuf,
    db: Arc<session_db_handles::SessionLedger>,
    tables: Vec<String>,
    chunks: tokio::sync::mpsc::Sender<Result<Bytes, std::io::Error>>,
) -> Result<(u64, u64), String> {
    let watermarks = snapshot_table_watermarks(&vm_id, &db_path, &db, &tables).await?;
    let mut encoder = GzipChunkEncoder::new();
    let mut rows_written = 0_u64;
    let mut bytes_written = 0_u64;

    let header = json!({
        "type": "header",
        "format": "capsem.rows.v1",
        "format_version": 1,
        "vm_id": vm_id,
        "exported_at": now_iso(),
        "tables": tables,
    });
    let mut line = serde_json::to_vec(&header).map_err(|error| error.to_string())?;
    line.push(b'\n');
    encoder.write_all(&line).map_err(|error| error.to_string())?;
    let header_chunk = encoder.flush_chunk().map_err(|error| error.to_string())?;
    bytes_written += header_chunk.len() as u64;
    if !send_export_chunk(&chunks, &vm_id, header_chunk).await {
        return Ok((rows_written, bytes_written));
    }

    let page_limit = u16::try_from(MAX_LEDGER_QUERY_ROWS).unwrap_or(2_000);
    let mut page_buf = Vec::with_capacity(MAX_EXPORT_PAGE_BUF_BYTES);
    for table in &tables {
        let max_id = watermarks.get(table).copied().unwrap_or(0);
        if max_id <= 0 {
            continue;
        }
        let mut after_id = 0_i64;
        while after_id < max_id {
            let rows = query_route_objects(
                &vm_id,
                "rows_export",
                table,
                &db_path,
                &db,
                LedgerQuery::TableRows {
                    table: table.clone(),
                    after_id,
                    max_id: Some(max_id),
                    limit: page_limit,
                },
                0,
            )
            .await
            .map_err(AppError::into_message)?;
            if rows.is_empty() {
                break;
            }
            let page_len = rows.len();
            for row in rows {
                let row_id = row
                    .get("id")
                    .and_then(serde_json::Value::as_i64)
                    .ok_or_else(|| format!("exported row in table {table} is missing integer id"))?;
                after_id = row_id;
                let record = json!({
                    "table": table,
                    "row": row,
                });
                serde_json::to_writer(&mut page_buf, &record).map_err(|error| error.to_string())?;
                page_buf.push(b'\n');
                rows_written += 1;
                if page_buf.len() >= MAX_EXPORT_PAGE_BUF_BYTES
                    && !flush_page_buf(&mut encoder, &mut page_buf, &chunks, &vm_id, &mut bytes_written).await?
                {
                    return Ok((rows_written, bytes_written));
                }
            }
            if !flush_page_buf(&mut encoder, &mut page_buf, &chunks, &vm_id, &mut bytes_written).await? {
                return Ok((rows_written, bytes_written));
            }
            if page_len < usize::from(page_limit) {
                break;
            }
        }
    }

    let trailer = encoder.finish().map_err(|error| error.to_string())?;
    bytes_written += trailer.len() as u64;
    let _ = send_export_chunk(&chunks, &vm_id, trailer).await;
    Ok((rows_written, bytes_written))
}

/// `GET /vms/{id}/export/rows.ndjson.gz` -- stream session ledger rows as
/// gzipped NDJSON (`capsem.rows.v1`).
pub(crate) async fn handle_rows_ndjson_export(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
    Query(query): Query<ExportRowsQuery>,
) -> Result<axum::response::Response, AppError> {
    let session_dir = resolve_session_dir(&state, &id)?;
    let tables = parse_export_tables(query.tables.as_deref())?;
    let db_path = session_dir.join("session.db");
    let db = open_ready_session_db(&state, &id, "rows_ndjson_export", &db_path).await?;

    let (chunks, receiver) = tokio::sync::mpsc::channel(EXPORT_CHANNEL_CHUNKS);
    let vm_id = id.clone();
    let db_path_for_task = db_path.clone();
    tokio::spawn(async move {
        match stream_rows_export(vm_id.clone(), db_path_for_task, db, tables, chunks.clone()).await {
            Ok((rows_written, bytes_written)) => info!(
                route = "/vms/{id}/export/rows.ndjson.gz",
                vm_id = vm_id.as_str(),
                rows = rows_written,
                bytes_written,
                "rows_ndjson_export"
            ),
            Err(error) => {
                error!(
                    route = "/vms/{id}/export/rows.ndjson.gz",
                    vm_id = vm_id.as_str(),
                    error = %error,
                    "session rows NDJSON export failed after the response began; the client has a truncated gzip stream"
                );
                let _ = chunks.send(Err(std::io::Error::other(error))).await;
            }
        }
    });

    let stream = tokio_stream::wrappers::ReceiverStream::new(receiver);
    axum::response::Response::builder()
        .header(axum::http::header::CONTENT_TYPE, "application/gzip")
        .header(
            axum::http::header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"capsem-session-{id}.rows.ndjson.gz\""),
        )
        .body(axum::body::Body::from_stream(stream))
        .map_err(|error| ledger_route_error(&id, "rows_export", "build the export response", &db_path, error))
}
