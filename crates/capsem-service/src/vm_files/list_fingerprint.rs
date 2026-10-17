//! Fields that invalidate the VM inventory response.

use super::*;

fn append_fingerprint_field(out: &mut String, value: &str) {
    use std::fmt::Write as _;

    let _ = write!(out, "{}:", value.len());
    out.push_str(value);
    out.push('|');
}

fn append_labels_fingerprint(out: &mut String, labels: Option<&HashMap<String, String>>) {
    use std::fmt::Write as _;

    let mut pairs: Vec<_> = labels.into_iter().flatten().collect();
    pairs.sort_unstable();
    let _ = write!(out, "labels={};", pairs.len());
    for (k, v) in pairs {
        append_fingerprint_field(out, k);
        append_fingerprint_field(out, v);
    }
}

/// Cheap key over every field /vms/list returns; list_lifecycle reuses the cached response while it is unchanged.
pub(crate) fn list_response_fingerprint(state: &ServiceState) -> String {
    use std::fmt::Write as _;

    let mut fingerprint = String::new();
    {
        let instances = state.instances.lock().unwrap();
        let _ = write!(fingerprint, "running={};", instances.len());
        for i in instances.values() {
            append_fingerprint_field(&mut fingerprint, &i.id);
            append_fingerprint_field(&mut fingerprint, &i.name);
            let _ = write!(
                fingerprint,
                "{}:{}:{}:{}:{};",
                i.pid,
                i.persistent,
                i.ram_mb,
                i.cpus,
                i.start_time.elapsed().as_secs()
            );
            append_fingerprint_field(&mut fingerprint, &i.base_version);
            append_fingerprint_field(&mut fingerprint, i.forked_from.as_deref().unwrap_or(""));
            append_labels_fingerprint(&mut fingerprint, i.labels.as_ref());
        }
    }
    {
        let registry = state.persistent_registry.lock().unwrap();
        let instances = state.instances.lock().unwrap();
        let inactive_entries: Vec<&PersistentVmEntry> = registry
            .list()
            .filter(|entry| !instances.contains_key(&persistent_entry_vm_id(entry)))
            .collect();
        let _ = write!(fingerprint, "inactive={};", inactive_entries.len());
        for entry in inactive_entries {
            append_fingerprint_field(&mut fingerprint, &persistent_entry_vm_id(entry));
            append_fingerprint_field(&mut fingerprint, &entry.name);
            append_fingerprint_field(&mut fingerprint, entry.legacy_profile_id.as_deref().unwrap_or(""));
            append_fingerprint_field(&mut fingerprint, &entry.base_version);
            append_fingerprint_field(&mut fingerprint, entry.forked_from.as_deref().unwrap_or(""));
            append_labels_fingerprint(&mut fingerprint, entry.labels.as_ref());
            append_fingerprint_field(&mut fingerprint, entry.description.as_deref().unwrap_or(""));
            append_fingerprint_field(&mut fingerprint, entry.last_error.as_deref().unwrap_or(""));
            let _ = write!(
                fingerprint,
                "{}:{}:{}:{}:{}:{};",
                entry.ram_mb,
                entry.cpus,
                entry.suspended,
                entry.defunct,
                entry.session_dir.display(),
                persistent_resume_state_fingerprint(state, entry)
            );
        }
        drop(instances);
        drop(registry);
    }
    fingerprint
}
