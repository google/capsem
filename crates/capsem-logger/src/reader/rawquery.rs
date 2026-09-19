//! Caller-owned SQL, DB-owned execution: the raw read path behind `query`.
//!
//! The route names its intent as a SELECT; this module owns validation,
//! binding, row shaping and the 5 s interrupt.
use std::time::{Duration, Instant};

use serde_json::Value;

use super::{validate_select_only, DbReader};

impl DbReader {
    fn with_query_timeout<F>(&self, run_query: F) -> Result<String, String>
    where
        F: FnOnce() -> Result<String, String>,
    {
        const TIMEOUT_MS: u64 = 5_000;
        const PROGRESS_OPS: i32 = 10_000;

        self.record_query_executed();
        let deadline = Instant::now() + Duration::from_millis(TIMEOUT_MS);
        self.conn
            .progress_handler(PROGRESS_OPS, Some(move || Instant::now() >= deadline));
        let result = run_query();
        self.conn.progress_handler(0, None::<fn() -> bool>);

        result.map_err(|e| {
            if e.contains("interrupted") {
                "query timed out after 5 seconds".to_string()
            } else {
                e
            }
        })
    }

    /// Execute an arbitrary read-only SQL query and return JSON.
    ///
    /// Returns `{"columns":[...],"rows":[[...], ...]}`.
    /// Caps output at 10,000 rows. Interrupts queries that run longer than
    /// 5 seconds via `sqlite3_interrupt`.
    pub fn query_raw(&self, sql: &str) -> Result<String, String> {
        // Defense-in-depth: reject non-SELECT SQL up front. The production
        // connection is opened read-only (SQLITE_OPEN_READ_ONLY) so writes
        // would fail at execution with a cryptic SQLite error -- validating
        // here gives a clear, consistent error and also guards open_in_memory().
        validate_select_only(sql)?;

        const MAX_ROWS: usize = 10_000;
        self.with_query_timeout(|| self.query_raw_inner(sql, MAX_ROWS))
    }

    /// Execute an arbitrary read-only SQL query with bind parameters and return JSON.
    ///
    /// Same format as `query_raw`: `{"columns":[...],"rows":[[...], ...]}`.
    /// Parameters use `?` positional placeholders (rusqlite native syntax).
    /// Supported param types: null, i64, f64, string (from serde_json::Value).
    pub fn query_raw_with_params(&self, sql: &str, params: &[Value]) -> Result<String, String> {
        // Defense-in-depth: same rationale as query_raw.
        validate_select_only(sql)?;

        const MAX_ROWS: usize = 10_000;
        self.with_query_timeout(|| self.query_raw_params_inner(sql, params, MAX_ROWS))
    }

    fn query_raw_inner(&self, sql: &str, max_rows: usize) -> Result<String, String> {
        self.query_raw_params_inner(sql, &[], max_rows)
    }

    fn query_raw_params_inner(&self, sql: &str, params: &[Value], max_rows: usize) -> Result<String, String> {
        let result = self.query_rows(sql, params, max_rows).map_err(|e| e.to_string())?;
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    fn query_rows(&self, sql: &str, params: &[Value], max_rows: usize) -> rusqlite::Result<Value> {
        let mut stmt = self.conn.prepare(sql)?;

        let columns: Vec<String> = stmt.column_names().iter().map(|s| s.to_string()).collect();
        let col_count = columns.len();

        // Convert serde_json::Value params to rusqlite dynamic params.
        let rusqlite_params: Vec<Box<dyn rusqlite::types::ToSql>> = params
            .iter()
            .map(|v| {
                let boxed: Box<dyn rusqlite::types::ToSql> = match v {
                    Value::Null => Box::new(rusqlite::types::Null),
                    Value::Bool(b) => Box::new(i64::from(*b)),
                    Value::Number(n) => {
                        if let Some(i) = n.as_i64() {
                            Box::new(i)
                        } else if let Some(f) = n.as_f64() {
                            Box::new(f)
                        } else {
                            Box::new(rusqlite::types::Null)
                        }
                    }
                    Value::String(s) => Box::new(s.clone()),
                    _ => Box::new(rusqlite::types::Null),
                };
                boxed
            })
            .collect();
        let param_refs: Vec<&dyn rusqlite::types::ToSql> = rusqlite_params.iter().map(|b| b.as_ref()).collect();

        let mut rows: Vec<Vec<Value>> = Vec::new();
        let mut raw_rows = stmt.query(param_refs.as_slice())?;

        while let Some(row) = raw_rows.next()? {
            if rows.len() >= max_rows {
                break;
            }
            let mut values = Vec::with_capacity(col_count);
            for i in 0..col_count {
                let val = row.get_ref(i)?;
                let json_val = match val {
                    rusqlite::types::ValueRef::Null => Value::Null,
                    rusqlite::types::ValueRef::Integer(n) => Value::Number(serde_json::Number::from(n)),
                    rusqlite::types::ValueRef::Real(f) => {
                        if f.is_finite() {
                            serde_json::Number::from_f64(f)
                                .map(Value::Number)
                                .unwrap_or(Value::Null)
                        } else {
                            Value::Null
                        }
                    }
                    rusqlite::types::ValueRef::Text(t) => {
                        let s = std::str::from_utf8(t).unwrap_or("<invalid utf8>");
                        Value::String(s.to_string())
                    }
                    rusqlite::types::ValueRef::Blob(b) => Value::String(format!("<blob {} bytes>", b.len())),
                };
                values.push(json_val);
            }
            rows.push(values);
        }

        Ok(serde_json::json!({
            "columns": columns,
            "rows": rows,
        }))
    }
}
