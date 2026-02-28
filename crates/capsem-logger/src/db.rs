use std::path::{Path, PathBuf};

use crate::reader::DbReader;
use crate::writer::DbWriter;

/// Convenience wrapper that owns the DB path and creates writer/reader instances.
pub struct SessionDb {
    path: PathBuf,
}

impl SessionDb {
    /// Create a new SessionDb pointing at the given path.
    /// Does not open any connections; call `writer()` or `reader()` as needed.
    pub fn new(path: &Path) -> Self {
        Self {
            path: path.to_path_buf(),
        }
    }

    /// The path to the database file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Open a writer (spawns a dedicated thread).
    pub fn writer(&self, capacity: usize) -> rusqlite::Result<DbWriter> {
        DbWriter::open(&self.path, capacity)
    }

    /// Open a read-only connection.
    pub fn reader(&self) -> rusqlite::Result<DbReader> {
        DbReader::open(&self.path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_db_path() {
        let db = SessionDb::new(Path::new("/tmp/test.db"));
        assert_eq!(db.path(), Path::new("/tmp/test.db"));
    }
}
