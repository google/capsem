use capsem_sdk::models::VmAction;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::model::{AppState, ServiceStatus, SessionLifecycle};

mod create;
pub use create::{CreateDraft, CreateField, ImageCatalog};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AppAction {
    Consumed,
    Forward,
    Invoke(ControlAction),
    Exit,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum AppOverlay {
    #[default]
    None,
    Help,
    Stats,
    Home,
    Create,
    Fork,
    Confirm,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ControlAction {
    StartService,
    Update,
    CreateSession {
        name: Option<String>,
        image: Option<String>,
    },
    Fork {
        id: String,
        name: String,
    },
    Start {
        id: String,
        label: String,
    },
    Resume {
        id: String,
        label: String,
    },
    Checkpoint {
        id: String,
        label: String,
    },
    Suspend {
        id: String,
        label: String,
    },
    Stop {
        id: String,
        label: String,
    },
    Delete {
        id: String,
        label: String,
    },
    Purge {
        all: bool,
    },
}

impl ControlAction {
    pub const fn label(&self) -> &'static str {
        match self {
            Self::StartService => "start service",
            Self::Update => "update",
            Self::CreateSession { .. } => "create",
            Self::Fork { .. } => "fork",
            Self::Start { .. } => "start",
            Self::Resume { .. } => "resume",
            Self::Checkpoint { .. } => "checkpoint",
            Self::Suspend { .. } => "suspend",
            Self::Stop { .. } => "stop",
            Self::Delete { .. } => "delete",
            Self::Purge { .. } => "purge",
        }
    }

    pub const fn progress_label(&self) -> &'static str {
        match self {
            Self::StartService => "starting service",
            Self::Update => "updating",
            Self::CreateSession { .. } => "creating",
            Self::Fork { .. } => "forking",
            Self::Start { .. } => "starting",
            Self::Resume { .. } => "resuming",
            Self::Checkpoint { .. } => "checkpointing",
            Self::Suspend { .. } => "suspending",
            Self::Stop { .. } => "stopping",
            Self::Delete { .. } => "deleting",
            Self::Purge { .. } => "purging",
        }
    }

    pub fn target(&self) -> &str {
        match self {
            Self::StartService => "Capsem service",
            Self::Update => "complete verified release",
            Self::CreateSession { name: Some(name), .. } => name,
            Self::CreateSession { name: None, .. } => "new session",
            Self::Fork { name, .. } => name,
            Self::Start { label, .. }
            | Self::Resume { label, .. }
            | Self::Checkpoint { label, .. }
            | Self::Suspend { label, .. }
            | Self::Stop { label, .. }
            | Self::Delete { label, .. } => label,
            Self::Purge { all: true } => "all sessions",
            Self::Purge { all: false } => "temporary and broken sessions",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct App {
    state: AppState,
    active_index: usize,
    overlay: AppOverlay,
    pending_action: Option<ControlAction>,
    pending_focus_session: Option<String>,
    control_progress: Option<String>,
    create_draft: Option<CreateDraft>,
    pending_create: Option<CreateDraft>,
    catalog_generation: u64,
    catalog_request: Option<u64>,
    fork_draft: Option<ForkDraft>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ForkDraft {
    pub source_id: String,
    pub name: String,
}

impl App {
    pub fn new(state: AppState) -> Self {
        let active_index = state
            .sessions
            .iter()
            .position(|session| session.id == state.active_session_id)
            .unwrap_or_default();
        let mut app = Self {
            state,
            active_index,
            overlay: AppOverlay::None,
            pending_action: None,
            pending_focus_session: None,
            control_progress: None,
            create_draft: None,
            pending_create: None,
            catalog_generation: 0,
            catalog_request: None,
            fork_draft: None,
        };
        app.ensure_active_tab_visible();
        app.sync_empty_state_prompt();
        app
    }

    pub fn state(&self) -> &AppState {
        &self.state
    }

    pub fn overlay(&self) -> AppOverlay {
        self.overlay
    }

    pub fn pending_action(&self) -> Option<&ControlAction> {
        self.pending_action.as_ref()
    }

    pub fn control_progress(&self) -> Option<&str> {
        self.control_progress.as_deref()
    }

    pub fn create_draft(&self) -> Option<&CreateDraft> {
        self.create_draft.as_ref()
    }

    pub fn fork_draft(&self) -> Option<&ForkDraft> {
        self.fork_draft.as_ref()
    }

    pub fn replace_state(&mut self, mut state: AppState) {
        state.service.control_message = self.state.service.control_message.clone();
        let previous_active_id = self.state.active_session_id.clone();
        if let Some(index) = self.pending_focus_index(&state) {
            state.active_session_id = state.sessions[index].id.clone();
            self.pending_focus_session = None;
        } else if state.sessions.iter().any(|session| session.id == previous_active_id) {
            state.active_session_id = previous_active_id;
        }
        self.active_index = state
            .sessions
            .iter()
            .position(|session| session.id == state.active_session_id)
            .unwrap_or_default();
        self.state = state;
        self.ensure_active_tab_visible();
        self.sync_empty_state_prompt();
    }

    pub fn set_control_message(&mut self, message: impl Into<String>) {
        self.state.service.control_message = Some(message.into());
    }

    pub fn set_control_progress(&mut self, label: impl Into<String>) {
        self.control_progress = Some(label.into());
    }

    pub fn clear_control_progress(&mut self) {
        self.control_progress = None;
    }

    pub fn focus_session_when_available(&mut self, id: impl Into<String>) {
        let id = id.into();
        if self.select_session_by_id(&id) {
            return;
        }
        self.pending_focus_session = Some(id);
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> AppAction {
        if is_exit_key(key) {
            return AppAction::Exit;
        }
        if let Some(action) = self.handle_pending_action_key(key) {
            return action;
        }
        if self.overlay == AppOverlay::Create {
            return self.handle_create_key(key);
        }
        if self.overlay == AppOverlay::Fork {
            return self.handle_fork_key(key);
        }
        if self.handle_overlay_key(key) {
            return AppAction::Consumed;
        }
        if self.overlay != AppOverlay::None {
            if key.code == KeyCode::Esc {
                self.overlay = AppOverlay::None;
            }
            return AppAction::Consumed;
        }
        if is_new_key(key) {
            self.open_create();
            return AppAction::Consumed;
        }
        if is_fork_key(key) && self.open_fork() {
            return AppAction::Consumed;
        }
        if self.resume_key_is_blocked(key) {
            if let Some(reason) = self.active_resume_blocked_reason().map(str::to_string) {
                self.set_control_message(reason);
            }
            return AppAction::Consumed;
        }
        if let Some(action) = self.control_action_for_key(key) {
            self.pending_action = Some(action);
            self.overlay = AppOverlay::Confirm;
            return AppAction::Consumed;
        }
        if key.code == KeyCode::Enter && key.modifiers.is_empty() {
            if self.active_resume_blocked_reason().is_some() {
                self.open_create();
                return AppAction::Consumed;
            }
            if self.state.active_session().is_none() {
                self.open_create();
                return AppAction::Consumed;
            }
            if let Some(action) = self.active_resume_action() {
                return AppAction::Invoke(action);
            }
        }
        if is_previous_key(key) {
            self.previous_session();
            return AppAction::Consumed;
        }
        if is_next_key(key) {
            self.next_session();
            return AppAction::Consumed;
        }
        if let Some(index) = select_index(key) {
            self.select_session(index);
            return AppAction::Consumed;
        }
        AppAction::Forward
    }

    pub fn next_session(&mut self) {
        let visible = visible_session_indices(&self.state);
        if visible.is_empty() {
            return;
        }
        let position = visible
            .iter()
            .position(|index| *index == self.active_index)
            .unwrap_or_default();
        self.active_index = visible[(position + 1) % visible.len()];
        self.sync_active_session();
    }

    pub fn previous_session(&mut self) {
        let visible = visible_session_indices(&self.state);
        if visible.is_empty() {
            return;
        }
        let position = visible
            .iter()
            .position(|index| *index == self.active_index)
            .unwrap_or_default();
        self.active_index = if position == 0 {
            visible[visible.len() - 1]
        } else {
            visible[position - 1]
        };
        self.sync_active_session();
    }

    pub fn select_session(&mut self, index: usize) {
        let visible = visible_session_indices(&self.state);
        let Some(actual_index) = visible.get(index).copied() else {
            return;
        };
        self.active_index = actual_index;
        self.sync_active_session();
    }

    pub fn select_session_by_id(&mut self, id: &str) -> bool {
        let Some(index) = self
            .state
            .sessions
            .iter()
            .position(|session| session.id == id || session.title == id)
        else {
            return false;
        };
        self.active_index = index;
        self.sync_active_session();
        true
    }

    fn pending_focus_index(&self, state: &AppState) -> Option<usize> {
        let pending = self.pending_focus_session.as_deref()?;
        state
            .sessions
            .iter()
            .position(|session| session.id == pending || session.title == pending)
    }

    fn ensure_active_tab_visible(&mut self) {
        if self
            .state
            .sessions
            .get(self.active_index)
            .is_some_and(session_visible_in_tabs)
        {
            self.sync_active_session();
            return;
        }
        let Some(index) = self.state.sessions.iter().position(session_visible_in_tabs) else {
            self.sync_active_session();
            return;
        };
        self.active_index = index;
        self.sync_active_session();
    }

    fn sync_active_session(&mut self) {
        let Some(session) = self.state.sessions.get(self.active_index) else {
            return;
        };
        self.state.active_session_id.clone_from(&session.id);
    }

    fn sync_empty_state_prompt(&mut self) {
        if service_needs_start(self.state.service.status) {
            self.create_draft = None;
            self.fork_draft = None;
            self.pending_action = Some(ControlAction::StartService);
            self.overlay = AppOverlay::Confirm;
            return;
        }
        if matches!(self.pending_action, Some(ControlAction::StartService)) {
            self.pending_action = None;
            self.overlay = AppOverlay::None;
        }
        if self.state.sessions.is_empty() && self.overlay == AppOverlay::None {
            self.open_create();
        }
    }

    fn handle_overlay_key(&mut self, key: KeyEvent) -> bool {
        if !is_alt_key(key.modifiers) {
            return false;
        }
        let next = match key.code {
            KeyCode::Char('?' | '/') => AppOverlay::Help,
            KeyCode::Char('i' | 'I') => AppOverlay::Stats,
            KeyCode::Char('l' | 'L' | 'o' | 'O') => AppOverlay::Home,
            _ => return false,
        };
        self.overlay = if self.overlay == next { AppOverlay::None } else { next };
        self.pending_action = None;
        self.create_draft = None;
        self.fork_draft = None;
        true
    }

    fn handle_pending_action_key(&mut self, key: KeyEvent) -> Option<AppAction> {
        let pending = self.pending_action.clone()?;
        match key.code {
            KeyCode::Enter => {
                self.pending_action = None;
                self.overlay = AppOverlay::None;
                if self.action_available(&pending) {
                    Some(AppAction::Invoke(pending))
                } else {
                    self.set_control_message(format!(
                        "{} is no longer available for {}",
                        pending.label(),
                        pending.target()
                    ));
                    Some(AppAction::Consumed)
                }
            }
            KeyCode::Esc => {
                self.pending_action = None;
                self.overlay = AppOverlay::None;
                Some(AppAction::Consumed)
            }
            _ => Some(AppAction::Consumed),
        }
    }

    fn control_action_for_key(&self, key: KeyEvent) -> Option<ControlAction> {
        if !is_alt_key(key.modifiers) {
            return None;
        }
        match key.code {
            KeyCode::Char('r' | 'R') => self.active_resume_action(),
            KeyCode::Char('c' | 'C') => self.active_checkpoint_action(),
            KeyCode::Char('s' | 'S') => self.active_suspend_action(),
            KeyCode::Char('t' | 'T') => {
                self.active_session_action(VmAction::Stop, |id, label| ControlAction::Stop { id, label })
            }
            KeyCode::Char('d' | 'D') => {
                self.active_session_action(VmAction::Delete, |id, label| ControlAction::Delete { id, label })
            }
            KeyCode::Char('p' | 'P') => Some(ControlAction::Purge { all: false }),
            KeyCode::Char('u' | 'U') => Some(ControlAction::Update),
            _ => None,
        }
    }

    fn resume_key_is_blocked(&self, key: KeyEvent) -> bool {
        is_alt_key(key.modifiers)
            && matches!(key.code, KeyCode::Char('r' | 'R'))
            && self.active_resume_blocked_reason().is_some()
    }

    fn active_resume_action(&self) -> Option<ControlAction> {
        let session = self.state.active_session()?;
        if !matches!(
            session.lifecycle,
            SessionLifecycle::Idle | SessionLifecycle::Suspended | SessionLifecycle::Failed
        ) {
            return None;
        }
        if resume_blocked_reason(session).is_some() {
            return None;
        }
        let id = session.id.clone();
        let label = session.title.clone();
        if session.available_actions.contains(&VmAction::Start) {
            Some(ControlAction::Start { id, label })
        } else if session.available_actions.contains(&VmAction::Resume) {
            Some(ControlAction::Resume { id, label })
        } else {
            None
        }
    }

    fn active_resume_blocked_reason(&self) -> Option<&str> {
        self.state.active_session().and_then(resume_blocked_reason)
    }

    fn active_checkpoint_action(&self) -> Option<ControlAction> {
        let session = self.state.active_session()?;
        if !session.available_actions.contains(&VmAction::Pause) {
            return None;
        }
        Some(ControlAction::Checkpoint {
            id: session.id.clone(),
            label: session.title.clone(),
        })
    }

    fn active_suspend_action(&self) -> Option<ControlAction> {
        let session = self.state.active_session()?;
        if !session.available_actions.contains(&VmAction::Pause) {
            return None;
        }
        Some(ControlAction::Suspend {
            id: session.id.clone(),
            label: session.title.clone(),
        })
    }

    fn active_session_action(
        &self,
        required: VmAction,
        action: impl FnOnce(String, String) -> ControlAction,
    ) -> Option<ControlAction> {
        let session = self.state.active_session()?;
        if !session.available_actions.contains(&required) {
            return None;
        }
        Some(action(session.id.clone(), session.title.clone()))
    }

    fn action_available(&self, action: &ControlAction) -> bool {
        let (id, required) = match action {
            ControlAction::Start { id, .. } => (id, VmAction::Start),
            ControlAction::Resume { id, .. } => (id, VmAction::Resume),
            ControlAction::Checkpoint { id, .. } | ControlAction::Suspend { id, .. } => (id, VmAction::Pause),
            ControlAction::Stop { id, .. } => (id, VmAction::Stop),
            ControlAction::Delete { id, .. } => (id, VmAction::Delete),
            ControlAction::Fork { id, .. } => (id, VmAction::Fork),
            _ => return true,
        };
        self.state.sessions.iter().any(|session| {
            session.id == *id
                && session.available_actions.contains(&required)
                && (!matches!(required, VmAction::Start | VmAction::Resume) || session.can_resume)
        })
    }

    fn active_id(&self) -> Option<String> {
        self.state.active_session().map(|session| session.id.clone())
    }

    fn open_fork(&mut self) -> bool {
        if !self
            .state
            .active_session()
            .is_some_and(|session| session.available_actions.contains(&VmAction::Fork))
        {
            return false;
        }
        let Some(source_id) = self.active_id() else {
            return false;
        };
        self.pending_action = None;
        self.create_draft = None;
        self.fork_draft = Some(ForkDraft {
            name: next_fork_name(&self.state, &source_id),
            source_id,
        });
        self.overlay = AppOverlay::Fork;
        true
    }

    fn handle_fork_key(&mut self, key: KeyEvent) -> AppAction {
        match key.code {
            KeyCode::Esc => {
                self.fork_draft = None;
                self.overlay = AppOverlay::None;
                AppAction::Consumed
            }
            KeyCode::Enter => {
                let Some(draft) = self.fork_draft.clone() else {
                    self.overlay = AppOverlay::None;
                    return AppAction::Consumed;
                };
                let name = draft.name.trim().to_string();
                if name.is_empty() {
                    return AppAction::Consumed;
                }
                self.fork_draft = None;
                self.overlay = AppOverlay::None;
                let action = ControlAction::Fork {
                    id: draft.source_id,
                    name,
                };
                if self.action_available(&action) {
                    AppAction::Invoke(action)
                } else {
                    self.set_control_message("fork is no longer available for this session");
                    AppAction::Consumed
                }
            }
            KeyCode::Backspace => {
                if let Some(draft) = &mut self.fork_draft {
                    draft.name.pop();
                }
                AppAction::Consumed
            }
            KeyCode::Char(ch)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER) =>
            {
                if let Some(draft) = &mut self.fork_draft {
                    draft.name.push(ch);
                }
                AppAction::Consumed
            }
            _ => AppAction::Consumed,
        }
    }
}

fn is_exit_key(key: KeyEvent) -> bool {
    matches!(
        (key.code, key.modifiers),
        (KeyCode::Char('q' | 'Q'), modifiers) if is_alt_key(modifiers)
    )
}

fn is_previous_key(key: KeyEvent) -> bool {
    is_alt_key(key.modifiers) && matches!(key.code, KeyCode::Left)
}

fn is_next_key(key: KeyEvent) -> bool {
    is_alt_key(key.modifiers) && matches!(key.code, KeyCode::Right)
}

fn is_new_key(key: KeyEvent) -> bool {
    is_alt_key(key.modifiers) && matches!(key.code, KeyCode::Char('n' | 'N'))
}

fn is_fork_key(key: KeyEvent) -> bool {
    is_alt_key(key.modifiers) && matches!(key.code, KeyCode::Char('f' | 'F'))
}

fn is_alt_key(modifiers: KeyModifiers) -> bool {
    modifiers.contains(KeyModifiers::ALT)
}

fn service_needs_start(status: ServiceStatus) -> bool {
    matches!(
        status,
        ServiceStatus::Offline | ServiceStatus::Degraded | ServiceStatus::Failed
    )
}

pub fn resume_blocked_reason(session: &crate::model::SessionSummary) -> Option<&str> {
    if !matches!(
        session.lifecycle,
        crate::model::SessionLifecycle::Idle
            | crate::model::SessionLifecycle::Suspended
            | crate::model::SessionLifecycle::Failed
    ) {
        return None;
    }
    if session.can_resume
        && session
            .available_actions
            .iter()
            .any(|action| matches!(action, VmAction::Start | VmAction::Resume))
    {
        return None;
    }
    Some(
        session
            .resume_blocked_reason
            .as_deref()
            .unwrap_or("cannot resume: session state is not resumable"),
    )
}

pub fn session_visible_in_tabs(session: &crate::model::SessionSummary) -> bool {
    resume_blocked_reason(session).is_none()
}

fn visible_session_indices(state: &AppState) -> Vec<usize> {
    state
        .sessions
        .iter()
        .enumerate()
        .filter_map(|(index, session)| session_visible_in_tabs(session).then_some(index))
        .collect()
}

/// The first free `vm-N` session name, the service's own spelling.
fn next_session_name(state: &AppState) -> String {
    for index in 1..1000 {
        let candidate = format!("vm-{index}");
        if state.sessions.iter().all(|session| session.id != candidate) {
            return candidate;
        }
    }
    "vm-1000".to_string()
}

fn next_fork_name(state: &AppState, source_id: &str) -> String {
    let base = format!("{source_id}-fork");
    if state.sessions.iter().all(|session| session.id != base) {
        return base;
    }
    for index in 2..1000 {
        let candidate = format!("{base}-{index}");
        if state.sessions.iter().all(|session| session.id != candidate) {
            return candidate;
        }
    }
    base
}

fn select_index(key: KeyEvent) -> Option<usize> {
    if !is_alt_key(key.modifiers) {
        return None;
    }
    let KeyCode::Char(value) = key.code else {
        return None;
    };
    value.to_digit(10).map(|digit| digit.saturating_sub(1) as usize)
}
