//! How a file event becomes the CEL facts a rule reads.
//!
//! Both file rails land here: the audit rail (`fs_monitor` observing the
//! workspace) and the explicit boundary rail (import, export, read, restore
//! through the file tools). One router means they cannot drift into different
//! notions of which action writes which facts.

use capsem_logger::{FileAction, FileEvent};

use super::{
    file_ext, file_name, runtime_file_event_type, ExplicitFileSecurityEvent, FileSecurityEvent, SecurityEvent,
};

/// The facts a single file action carries, before they are routed to that
/// action's slot group.
#[derive(Default)]
pub(crate) struct FileActionFacts {
    pub path: Option<String>,
    pub name: Option<String>,
    pub ext: Option<String>,
    pub mime_type: Option<String>,
    pub content: Option<String>,
}

impl FileSecurityEvent {
    /// Route the facts into the slots the action's rule namespace reads:
    /// `file.create.path`, `file.write.path`, `file.delete.path`, and so on.
    ///
    /// One router rather than a match per builder: the audit rail and the
    /// explicit boundary rail must not drift into different notions of which
    /// action writes which facts.
    pub(crate) fn set_action_facts(&mut self, action: FileAction, facts: FileActionFacts) {
        // An overflow marker names no path, so it fills no slot group: a rule
        // written about a path must not match the row that says a window of
        // paths went unrecorded.
        if action == FileAction::Overflow {
            return;
        }
        let (path, name, ext, mime_type, content) = match action {
            FileAction::Created => (
                &mut self.create_path,
                &mut self.create_name,
                &mut self.create_ext,
                &mut self.create_mime_type,
                &mut self.create_content,
            ),
            FileAction::Modified | FileAction::Restored => (
                &mut self.write_path,
                &mut self.write_name,
                &mut self.write_ext,
                &mut self.write_mime_type,
                &mut self.write_content,
            ),
            FileAction::Deleted => (
                &mut self.delete_path,
                &mut self.delete_name,
                &mut self.delete_ext,
                &mut self.delete_mime_type,
                &mut self.delete_content,
            ),
            FileAction::Read => (
                &mut self.read_path,
                &mut self.read_name,
                &mut self.read_ext,
                &mut self.read_mime_type,
                &mut self.read_content,
            ),
            FileAction::Imported => (
                &mut self.import_path,
                &mut self.import_name,
                &mut self.import_ext,
                &mut self.import_mime_type,
                &mut self.import_content,
            ),
            FileAction::Exported => (
                &mut self.export_path,
                &mut self.export_name,
                &mut self.export_ext,
                &mut self.export_mime_type,
                &mut self.export_content,
            ),
            // Returned above, before any slot was chosen.
            FileAction::Overflow => return,
        };
        *path = facts.path;
        *name = facts.name;
        *ext = facts.ext;
        *mime_type = facts.mime_type;
        *content = facts.content;
    }
}

/// The ledger row behind an explicit boundary event.
///
/// `kind` is `File` without a stat because these boundaries never resolve a
/// path themselves: import, export, read and restore each name a regular file
/// the caller already opened, and the bytes arrive with the request rather
/// than being read back off disk here. Nothing on this path dereferences a
/// guest-controlled link.
pub(super) fn explicit_primary_file_event(event: &ExplicitFileSecurityEvent) -> FileEvent {
    FileEvent {
        event_id: None,
        timestamp: std::time::SystemTime::now(),
        action: event.action,
        path: event.path.clone(),
        size: event.size,
        kind: capsem_logger::FileKind::File,
        trace_id: event.trace_id.clone(),
        credential_ref: event.credential_ref.clone(),
    }
}

pub fn security_event_from_file_event(event: &FileEvent) -> SecurityEvent {
    let mut file = FileSecurityEvent {
        kind: Some(event.kind.as_str().to_string()),
        ..FileSecurityEvent::default()
    };
    file.set_action_facts(
        event.action,
        FileActionFacts {
            path: Some(event.path.clone()),
            name: file_name(&event.path),
            ext: file_ext(&event.path),
            ..FileActionFacts::default()
        },
    );
    let mut security_event = SecurityEvent::new(runtime_file_event_type(event.action)).with_file(file);
    if let Some(trace_id) = event.trace_id.clone() {
        security_event = security_event.with_trace_id(trace_id);
    }
    if let Some(credential_ref) = event.credential_ref.clone() {
        security_event = security_event.with_credential_ref(credential_ref);
    }
    security_event
}

pub fn security_event_from_explicit_file_event(event: &ExplicitFileSecurityEvent) -> SecurityEvent {
    let mut file = FileSecurityEvent {
        content: event.content.clone(),
        kind: Some(capsem_logger::FileKind::File.as_str().to_string()),
        ..FileSecurityEvent::default()
    };
    file.set_action_facts(
        event.action,
        FileActionFacts {
            path: Some(event.path.clone()),
            name: file_name(&event.path),
            ext: file_ext(&event.path),
            mime_type: event.mime_type.clone(),
            content: event.content.clone(),
        },
    );
    let mut security_event = SecurityEvent::new(runtime_file_event_type(event.action)).with_file(file);
    if let Some(trace_id) = event.trace_id.clone() {
        security_event = security_event.with_trace_id(trace_id);
    }
    if let Some(credential_ref) = event.credential_ref.clone() {
        security_event = security_event.with_credential_ref(credential_ref);
    }
    security_event
}
