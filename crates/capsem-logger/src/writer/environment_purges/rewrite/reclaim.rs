use super::*;
use capsem_foundation::unix::contained::ContainedOpenOptions;
use capsem_foundation::unix::lock::{try_lock_descriptor, DescriptorLockAttempt, LockMode};
use std::ffi::OsStr;
use std::io::{Read, Seek, SeekFrom};

pub(super) fn finish(
    conn: &Connection,
    bodies: &BodyArchive,
    execution: &EnvironmentVariablePurgeExecution,
) -> Result<EnvironmentVariablePurgeRecord, String> {
    let record = status::record(conn, &execution.purge_id)?;
    let old = GenerationId::from_bytes(
        record
            .old_generation_id
            .ok_or("purge reclamation lacks old generation")?,
    )
    .map_err(|_| "invalid old purge generation")?;
    let new = GenerationId::from_bytes(
        record
            .new_generation_id
            .ok_or("purge reclamation lacks published generation")?,
    )
    .map_err(|_| "invalid published purge generation")?;
    let directory = bodies
        .generation_directory()
        .ok_or("purge archive directory is unavailable")?;
    let db_path = bodies.db_path().ok_or("purge ledger path is unavailable")?;
    let _publication = capsem_foundation::unix::lock::acquire_existing_until(
        &crate::writer::archive_lock_path_for_db(db_path),
        LockMode::Exclusive,
        std::time::Instant::now() + std::time::Duration::from_secs(5),
    )
    .map_err(|_| "cannot acquire purge reclamation lock")?;
    let current = crate::schema::archive_state(conn).map_err(|_| "cannot read purge reclamation root")?;
    let attempt = collect(directory, current);
    match attempt {
        Ok(true) => {
            status::durable_update(
                conn,
                execution,
                EnvironmentVariablePurgeState::Committed,
                Some(old),
                Some(new),
                None,
            )?;
        }
        Ok(false) => {}
        Err(_) => {
            status::durable_update(
                conn,
                execution,
                EnvironmentVariablePurgeState::ReclaimPending,
                Some(old),
                Some(new),
                Some("old archive generation reclamation is still pending"),
            )?;
        }
    }
    status::record(conn, &execution.purge_id)
}

fn collect(
    directory: &capsem_foundation::unix::contained::ContainedDir,
    current: crate::schema::ArchiveState,
) -> Result<bool, String> {
    let mut complete = true;
    directory
        .visit_entries(|entry| {
            let name = entry.name;
            let Some(name_text) = name.to_str() else {
                return Ok(true);
            };
            let Ok(generation) = GenerationId::from_file_name(name_text) else {
                return Ok(true);
            };
            if generation == current.header.generation_id {
                return Ok(true);
            }
            if entry.kind != capsem_foundation::unix::contained::EntryKind::File {
                return Err(std::io::Error::other("old archive generation is not a regular file"));
            }
            let file = directory.open_file(OsStr::new(name_text), ContainedOpenOptions::read_only())?;
            let mut held = match try_lock_descriptor(file, LockMode::Exclusive)? {
                DescriptorLockAttempt::Acquired(held) => held,
                DescriptorLockAttempt::Contended(_) => {
                    complete = false;
                    return Ok(true);
                }
            };
            held.seek(SeekFrom::Start(0))?;
            let mut header = [0u8; capsem_archive::FILE_HEADER_BYTES];
            held.read_exact(&mut header)?;
            capsem_archive::format::decode_file_header(&header)
                .and_then(|header| header.validate(current.header.archive_id, generation, name_text))
                .map_err(|_| std::io::Error::other("old archive generation identity is not owned by this ledger"))?;
            directory.remove_private_file(OsStr::new(name_text))?;
            drop(held);
            Ok(true)
        })
        .map_err(|_| "old archive generation collection remains pending")?;
    directory
        .sync()
        .map_err(|_| "old archive generation directory sync remains pending")?;
    Ok(complete)
}
