// SPDX-License-Identifier: GPL-2.0-only

//! Atomic publication of city saves and reusable authored-world SQLite files.

use rusqlite::{Connection, Transaction};
use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

// Temporary names affect filesystem ownership only, never simulation state or saved contents.
static NEXT_TEMP_FILE: AtomicU64 = AtomicU64::new(0);

struct PendingSqliteFile(PathBuf);

impl PendingSqliteFile {
    fn create(destination: &Path) -> io::Result<Self> {
        let filename = destination.file_name().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "snapshot path has no filename")
        })?;
        if let Some(parent) = destination.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent)?;
        }
        loop {
            let sequence = NEXT_TEMP_FILE.fetch_add(1, Ordering::Relaxed);
            let mut temporary_name = OsString::from(".");
            temporary_name.push(filename);
            temporary_name.push(format!(".{}.{sequence}.tmp", std::process::id()));
            let path = destination.with_file_name(temporary_name);
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(_) => return Ok(Self(path)),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
    }
}

impl Drop for PendingSqliteFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
        // A failed rollback may leave the temporary database's DELETE-mode journal.
        let mut journal = self.0.as_os_str().to_os_string();
        journal.push("-journal");
        let _ = fs::remove_file(Path::new(&journal));
    }
}

/// Commits and closes a fresh SQLite file before replacing the destination in one rename.
///
/// The adjacent temporary file stays on the destination filesystem. Any schema, callback,
/// commit, close or rename error leaves the previous destination intact and removes the temp file.
pub(crate) fn write_sqlite_snapshot<E>(
    path: &Path,
    schema: &str,
    write: impl FnOnce(&Transaction<'_>) -> Result<(), E>,
) -> Result<(), E>
where
    E: From<rusqlite::Error> + From<io::Error>,
{
    let pending = PendingSqliteFile::create(path)?;
    let mut connection = Connection::open(&pending.0)?;
    connection.execute_batch(schema)?;
    let transaction = connection.transaction()?;
    write(&transaction)?;
    transaction.commit()?;
    connection.close().map_err(|(_, error)| error)?;
    fs::rename(&pending.0, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::save::SaveLoadError;

    #[test]
    fn snapshot_publication_replaces_successes_and_preserves_failures() {
        let destination =
            PendingSqliteFile::create(&std::env::temp_dir().join("metrum_atomic_snapshot.sqlite"))
                .unwrap();
        let schema = "CREATE TABLE snapshot(value INTEGER NOT NULL);";
        let save = |value: Option<i64>| {
            write_sqlite_snapshot::<SaveLoadError>(&destination.0, schema, |transaction| {
                transaction.execute("INSERT INTO snapshot(value) VALUES (?1)", [value])?;
                Ok(())
            })
        };
        save(Some(7)).unwrap();
        let original = fs::read(&destination.0).unwrap();
        assert!(save(None).is_err());
        assert!(fs::read(&destination.0).unwrap() == original);
        save(Some(9)).unwrap();
        let connection = Connection::open(&destination.0).unwrap();
        let values: (i64, i64) = connection
            .query_row("SELECT COUNT(*), MIN(value) FROM snapshot", [], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .unwrap();
        assert_eq!(values, (1, 9));
    }
}
