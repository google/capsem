//! Support bundle helpers (redactor + bundler).
//!
//! `redact` strips secrets from log lines and config files before they
//! land in `~/.capsem/support/<bundle>.tar.gz`. `bundle` walks the
//! `~/.capsem/` layout, gathers logs/configs/sessions, and emits a
//! manifest.json next to them.

pub mod redact;
