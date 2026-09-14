//! Rate calculation shared by host records and raw guest protocol collectors.
pub(crate) fn per_second(count: usize, elapsed: std::time::Duration) -> f64 {
    count as f64 / elapsed.as_secs_f64().max(1e-9)
}
