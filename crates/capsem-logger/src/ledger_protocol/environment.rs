//! Named environment reads; SQLite and archive execution remain DB-owned.
use super::*;
use crate::db::{
    EnvironmentApplicationQuery, EnvironmentApplicationRecord, EnvironmentHistoryPayload, EnvironmentReferenceQuery,
    EnvironmentReferenceRecord, EnvironmentRequestHistory, EnvironmentRequestQuery, EnvironmentRequestRecord,
    EnvironmentRequestSnapshot, EnvironmentRevisionHistory, EnvironmentRevisionQuery, EnvironmentRevisionRecord,
    EnvironmentRevisionSnapshot, EnvironmentVariablePurgeQuery, EnvironmentVariablePurgeRecord,
};

mod correlation;
mod validation;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum LedgerEnvironmentQuery {
    Revisions {
        query: EnvironmentRevisionQuery,
    },
    RevisionSnapshot {
        environment_id: String,
        revision_id: String,
    },
    RevisionHistory {
        environment_id: String,
        revision_id: String,
    },
    Requests {
        query: EnvironmentRequestQuery,
    },
    RequestSnapshot {
        request_id: String,
    },
    RequestHistory {
        request_id: String,
    },
    References {
        query: EnvironmentReferenceQuery,
    },
    Applications {
        query: EnvironmentApplicationQuery,
    },
    Purges {
        query: EnvironmentVariablePurgeQuery,
    },
}
impl LedgerEnvironmentQuery {
    pub fn validate(&self) -> Result<(), LedgerProtocolError> {
        let valid = match self {
            Self::Revisions { query } => query.validate().is_ok(),
            Self::RevisionSnapshot {
                environment_id,
                revision_id,
            }
            | Self::RevisionHistory {
                environment_id,
                revision_id,
            } => validation::id(environment_id) && validation::id(revision_id),
            Self::Requests { query } => query.validate().is_ok(),
            Self::RequestSnapshot { request_id } | Self::RequestHistory { request_id } => validation::id(request_id),
            Self::References { query } => query.validate().is_ok(),
            Self::Applications { query } => query.validate().is_ok(),
            Self::Purges { query } => query.validate().is_ok(),
        };
        if valid {
            Ok(())
        } else {
            Err(LedgerProtocolError::InvalidOperation)
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum LedgerEnvironmentReply {
    Revisions {
        records: Vec<EnvironmentRevisionRecord>,
    },
    RevisionSnapshot {
        snapshot: Option<Box<EnvironmentRevisionSnapshot>>,
    },
    RevisionHistory {
        history: Option<Box<EnvironmentRevisionHistory>>,
    },
    Requests {
        records: Vec<EnvironmentRequestRecord>,
    },
    RequestSnapshot {
        snapshot: Option<Box<EnvironmentRequestSnapshot>>,
    },
    RequestHistory {
        history: Option<Box<EnvironmentRequestHistory>>,
    },
    References {
        records: Vec<EnvironmentReferenceRecord>,
    },
    Applications {
        records: Vec<EnvironmentApplicationRecord>,
    },
    Purges {
        records: Vec<EnvironmentVariablePurgeRecord>,
    },
}
impl LedgerEnvironmentReply {
    pub fn validate(&self) -> Result<(), LedgerProtocolError> {
        let valid = match self {
            Self::Revisions { records } => {
                records.len() <= MAX_LEDGER_QUERY_ROWS && records.iter().all(validation::revision)
            }
            Self::RevisionSnapshot { snapshot } => snapshot.as_ref().is_none_or(|s| {
                validation::revision(&s.record) && s.record.replayable && s.specification.validate().is_ok()
            }),
            Self::RevisionHistory { history } => history.as_ref().is_none_or(|h| {
                validation::revision(&h.record)
                    && validation::history(
                        h.record.replayable,
                        &h.payload,
                        |s| s.validate().is_ok(),
                        |s, key| !s.variables.contains_key(key),
                    )
            }),
            Self::Requests { records } => {
                records.len() <= MAX_LEDGER_QUERY_ROWS && records.iter().all(validation::request)
            }
            Self::RequestSnapshot { snapshot } => snapshot.as_ref().is_none_or(|s| {
                validation::request(&s.record)
                    && s.record.replayable
                    && validation::request_payload(&s.record, &s.payload)
            }),
            Self::RequestHistory { history } => history.as_ref().is_none_or(|h| {
                validation::request(&h.record)
                    && validation::history(
                        h.record.replayable,
                        &h.payload,
                        |p| validation::request_payload(&h.record, p),
                        validation::request_key_absent,
                    )
            }),
            Self::References { records } => validation::references(records),
            Self::Applications { records } => {
                records.len() <= MAX_LEDGER_QUERY_ROWS && records.iter().all(validation::application)
            }
            Self::Purges { records } => {
                records.len() <= MAX_LEDGER_QUERY_ROWS && records.iter().all(|r| r.validate().is_ok())
            }
        };
        if valid {
            Ok(())
        } else {
            Err(LedgerProtocolError::InvalidOperation)
        }
    }
    pub(crate) fn matches_query(&self, query: &LedgerEnvironmentQuery) -> bool {
        correlation::matches(self, query)
    }
}

#[cfg(test)]
mod tests;
