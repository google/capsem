use super::*;
fn ordered(mut cursors: impl Iterator<Item = i64>, after: i64) -> bool {
    let mut previous = after;
    cursors.all(|cursor| {
        let valid = cursor > previous;
        previous = cursor;
        valid
    })
}
pub(super) fn matches(reply: &LedgerEnvironmentReply, query: &LedgerEnvironmentQuery) -> bool {
    match (reply,query) {
        (LedgerEnvironmentReply::Revisions {records},LedgerEnvironmentQuery::Revisions {query})=>match query {
            EnvironmentRevisionQuery::Get {environment_id,revision_id}=>records.len()<=1&&records.iter().all(|r|r.environment_id==*environment_id&&r.revision_id==*revision_id),
            EnvironmentRevisionQuery::ForRequest {request_id}=>records.len()<=1&&records.iter().all(|r|r.request_id.as_ref()==Some(request_id)),
            EnvironmentRevisionQuery::Page {environment_id,state,after_id,limit}=>records.len()<=usize::from(*limit)&&ordered(records.iter().map(|r|r.cursor),*after_id)&&records.iter().all(|r|environment_id.as_ref().is_none_or(|id|r.environment_id==*id)&&state.is_none_or(|state|r.state==state)),
            EnvironmentRevisionQuery::History {environment_id,before_id,limit} => {
                let mut bound = *before_id;
                records.len() <= usize::from(*limit) && records.iter().all(|record| {
                    let valid = record.environment_id == *environment_id
                        && record.state == crate::events::EnvironmentRevisionState::Committed
                        && bound.is_none_or(|cursor| record.cursor < cursor);
                    bound = Some(record.cursor);
                    valid
                })
            }
        },
        (LedgerEnvironmentReply::RevisionSnapshot {snapshot},LedgerEnvironmentQuery::RevisionSnapshot {environment_id,revision_id})=>snapshot.as_ref().is_none_or(|s|s.record.environment_id==*environment_id&&s.record.revision_id==*revision_id),
        (LedgerEnvironmentReply::RevisionHistory {history},LedgerEnvironmentQuery::RevisionHistory {environment_id,revision_id})=>history.as_ref().is_none_or(|h|h.record.environment_id==*environment_id&&h.record.revision_id==*revision_id),
        (LedgerEnvironmentReply::Requests {records},LedgerEnvironmentQuery::Requests {query})=>match query {
            EnvironmentRequestQuery::Get {request_id}=>records.len()<=1&&records.iter().all(|r|r.request_id==*request_id),
            EnvironmentRequestQuery::Page {environment_id,requester,pending_only,after_id,limit}=>records.len()<=usize::from(*limit)&&ordered(records.iter().map(|r|r.cursor),*after_id)&&records.iter().all(|r|environment_id.as_ref().is_none_or(|id|r.environment_id==*id)&&requester.as_ref().is_none_or(|t|r.requester.as_ref()==Some(t))&&(!pending_only||r.state==crate::events::environment::request::EnvironmentRequestState::Pending)),
        },
        (LedgerEnvironmentReply::RequestSnapshot {snapshot},LedgerEnvironmentQuery::RequestSnapshot {request_id})=>snapshot.as_ref().is_none_or(|s|s.record.request_id==*request_id),
        (LedgerEnvironmentReply::RequestHistory {history},LedgerEnvironmentQuery::RequestHistory {request_id})=>history.as_ref().is_none_or(|h|h.record.request_id==*request_id),
        (LedgerEnvironmentReply::References {records},LedgerEnvironmentQuery::References {query})=>match query {
            EnvironmentReferenceQuery::CurrentEnvironment {environment_id}=>records.len()<=1&&records.iter().all(|r|matches!(r,EnvironmentReferenceRecord::CurrentEnvironment {revision} if revision.environment_id==*environment_id)),
            EnvironmentReferenceQuery::ArchivedEnvironment {environment_id}=>records.len()<=1&&records.iter().all(|r|matches!(r,EnvironmentReferenceRecord::ArchivedEnvironment {revision} if revision.environment_id==*environment_id)),
            EnvironmentReferenceQuery::Sandbox {..}=>records.len()<=3&&records.iter().all(|r|matches!(r,EnvironmentReferenceRecord::Sandbox {..})),
            EnvironmentReferenceQuery::CurrentEnvironments {after,limit} => records.len() <= usize::from(*limit) && validation::current_heads(records, after.as_deref()),
        },
        (LedgerEnvironmentReply::Applications {records},LedgerEnvironmentQuery::Applications {query})=>match query {
            EnvironmentApplicationQuery::Get {request_id,target}=>records.len()<=1&&records.iter().all(|r|r.request_id==*request_id&&r.target==*target),
            EnvironmentApplicationQuery::Page {request_id,target,after_id,limit}=>records.len()<=usize::from(*limit)&&ordered(records.iter().map(|r|r.cursor),*after_id)&&records.iter().all(|r|request_id.as_ref().is_none_or(|id|r.request_id==*id)&&target.as_ref().is_none_or(|t|r.target==*t)),
        },
        (LedgerEnvironmentReply::Purges {records},LedgerEnvironmentQuery::Purges {query})=>match query {
            EnvironmentVariablePurgeQuery::Get {purge_id}=>records.len()<=1&&records.iter().all(|r|r.intent.purge_id==*purge_id),
            EnvironmentVariablePurgeQuery::Page {environment_id,after_id,limit}=>records.len()<=usize::from(*limit)&&ordered(records.iter().map(|r|r.cursor),*after_id)&&records.iter().all(|r|environment_id.as_ref().is_none_or(|id|r.intent.environment_id==*id)),
        },
        _=>false,
    }
}
