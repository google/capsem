use super::*;
use capsem_sdk::models::{SecurityAskDecision, SecurityAskListResponse, SecurityAskStatus};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApprovalView {
    pub(crate) id: u64,
    pub vm_id: String,
    pub list: Option<SecurityAskListResponse>,
    pub selected: Option<usize>,
    pub loading: bool,
    pub resolving: bool,
    pub error: Option<String>,
}

impl App {
    pub fn approval_view(&self) -> Option<&ApprovalView> {
        self.approval_view.as_ref()
    }

    pub(super) fn open_approvals(&mut self) {
        let Some(session) = self.state.active_session() else {
            return;
        };
        self.create_draft = None;
        self.catalog_request = None;
        self.fork_draft = None;
        self.pending_action = None;
        self.approval_view_id = self.approval_view_id.wrapping_add(1);
        self.approval_view = Some(ApprovalView {
            id: self.approval_view_id,
            vm_id: session.id.clone(),
            list: None,
            selected: None,
            loading: false,
            resolving: false,
            error: None,
        });
        self.overlay = AppOverlay::Approvals;
        self.refresh_approvals();
    }

    pub fn take_approval_request(&mut self) -> Option<ControlAction> {
        self.approval_request.take()
    }

    pub fn refresh_approvals(&mut self) {
        let Some(view) = &mut self.approval_view else { return };
        if self.overlay != AppOverlay::Approvals || view.loading || view.resolving {
            return;
        }
        view.loading = true;
        self.approval_request = Some(ControlAction::ListApprovals {
            id: view.vm_id.clone(),
            view_id: view.id,
        });
    }

    pub fn apply_approval_list(&mut self, id: &str, view_id: u64, result: Result<SecurityAskListResponse, String>) {
        let Some(view) = &mut self.approval_view else { return };
        if view.vm_id != id || view.id != view_id || self.overlay != AppOverlay::Approvals || !view.loading {
            return;
        }
        view.loading = false;
        match result {
            Ok(list) if list.vm_id == id => {
                view.selected = match &view.list {
                    Some(previous) if previous.generation == list.generation => view
                        .selected
                        .and_then(|index| previous.requests.get(index))
                        .and_then(|selected| {
                            list.requests
                                .iter()
                                .position(|request| request.ask_id == selected.ask_id)
                        }),
                    None if view.error.is_none() && !list.requests.is_empty() => Some(0),
                    _ => None,
                };
                view.list = Some(list);
                view.error = None;
            }
            _ => {
                view.list = None;
                view.selected = None;
                view.error = Some("Approval requests unavailable; R retries the current state".into());
            }
        }
    }

    pub fn complete_approval_decision(&mut self, id: &str, view_id: u64, success: bool) {
        let Some(view) = &mut self.approval_view else { return };
        if view.vm_id != id || view.id != view_id || !view.resolving {
            return;
        }
        view.resolving = false;
        view.error = (!success).then(|| "Decision not confirmed; check the current state before deciding again".into());
        self.refresh_approvals();
    }

    pub(super) fn handle_approval_key(&mut self, key: KeyEvent) -> AppAction {
        if key.code == KeyCode::Esc {
            self.overlay = AppOverlay::None;
            self.approval_view = None;
            self.approval_request = None;
            return AppAction::Consumed;
        }
        if matches!(key.code, KeyCode::Char('r' | 'R')) {
            self.refresh_approvals();
            return AppAction::Consumed;
        }
        let Some(view) = &mut self.approval_view else {
            return AppAction::Consumed;
        };
        let Some(list) = &view.list else {
            return AppAction::Consumed;
        };
        if matches!(key.code, KeyCode::Up | KeyCode::Down) && !list.requests.is_empty() {
            let count = list.requests.len();
            view.selected = if key.code == KeyCode::Down {
                Some(view.selected.map_or(0, |index| (index + 1) % count))
            } else {
                Some(view.selected.map_or(count - 1, |index| (index + count - 1) % count))
            };
        }
        let decision = match key.code {
            KeyCode::Char('a' | 'A') if key.modifiers == KeyModifiers::NONE => SecurityAskDecision::Approve,
            KeyCode::Char('d' | 'D') if key.modifiers == KeyModifiers::NONE => SecurityAskDecision::Deny,
            _ => return AppAction::Consumed,
        };
        let Some(request) = view.selected.and_then(|index| list.requests.get(index)) else {
            return AppAction::Consumed;
        };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        if view.loading
            || view.resolving
            || request.status != SecurityAskStatus::Pending
            || i128::from(request.expires_at_unix_ms) <= i128::try_from(now).unwrap_or(i128::MAX)
        {
            return AppAction::Consumed;
        }
        view.resolving = true;
        AppAction::Invoke(ControlAction::ResolveApproval {
            id: view.vm_id.clone(),
            view_id: view.id,
            generation: list.generation.clone(),
            ask_id: request.ask_id.clone(),
            decision,
        })
    }
}
