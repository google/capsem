use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::model::{AppState, SessionLifecycle};

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
    CreateSession { name: String, profile_id: String },
    Fork { id: String, name: String },
    Resume { name: String },
    Checkpoint { id: String },
    Suspend { id: String },
    Stop { id: String },
    Delete { id: String },
}

impl ControlAction {
    pub const fn label(&self) -> &'static str {
        match self {
            Self::CreateSession { .. } => "create",
            Self::Fork { .. } => "fork",
            Self::Resume { .. } => "resume",
            Self::Checkpoint { .. } => "checkpoint",
            Self::Suspend { .. } => "suspend",
            Self::Stop { .. } => "stop",
            Self::Delete { .. } => "delete",
        }
    }

    pub fn target(&self) -> &str {
        match self {
            Self::CreateSession { name, .. } => name,
            Self::Fork { name, .. } => name,
            Self::Resume { name }
            | Self::Checkpoint { id: name }
            | Self::Suspend { id: name }
            | Self::Stop { id: name }
            | Self::Delete { id: name } => name,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct App {
    state: AppState,
    active_index: usize,
    overlay: AppOverlay,
    pending_action: Option<ControlAction>,
    create_draft: Option<CreateDraft>,
    fork_draft: Option<ForkDraft>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreateDraft {
    pub name: String,
    pub selected_profile: usize,
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
            create_draft: None,
            fork_draft: None,
        };
        if app.state.sessions.is_empty() {
            app.open_create();
        }
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

    pub fn create_draft(&self) -> Option<&CreateDraft> {
        self.create_draft.as_ref()
    }

    pub fn fork_draft(&self) -> Option<&ForkDraft> {
        self.fork_draft.as_ref()
    }

    pub fn replace_state(&mut self, mut state: AppState) {
        state.service.control_message = self.state.service.control_message.clone();
        let previous_active_id = self.state.active_session_id.clone();
        if state
            .sessions
            .iter()
            .any(|session| session.id == previous_active_id)
        {
            state.active_session_id = previous_active_id;
        }
        self.active_index = state
            .sessions
            .iter()
            .position(|session| session.id == state.active_session_id)
            .unwrap_or_default();
        self.state = state;
        self.sync_active_session();
    }

    pub fn set_control_message(&mut self, message: impl Into<String>) {
        self.state.service.control_message = Some(message.into());
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
        if is_fork_key(key) {
            if self.open_fork() {
                return AppAction::Consumed;
            }
        }
        if let Some(action) = self.control_action_for_key(key) {
            self.pending_action = Some(action);
            self.overlay = AppOverlay::Confirm;
            return AppAction::Consumed;
        }
        if key.code == KeyCode::Enter && key.modifiers.is_empty() {
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
        if self.state.sessions.is_empty() {
            return;
        }
        self.active_index = (self.active_index + 1) % self.state.sessions.len();
        self.sync_active_session();
    }

    pub fn previous_session(&mut self) {
        if self.state.sessions.is_empty() {
            return;
        }
        self.active_index = if self.active_index == 0 {
            self.state.sessions.len() - 1
        } else {
            self.active_index - 1
        };
        self.sync_active_session();
    }

    pub fn select_session(&mut self, index: usize) {
        if index >= self.state.sessions.len() {
            return;
        }
        self.active_index = index;
        self.sync_active_session();
    }

    fn sync_active_session(&mut self) {
        let Some(session) = self.state.sessions.get(self.active_index) else {
            return;
        };
        self.state.active_session_id.clone_from(&session.id);
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
        self.overlay = if self.overlay == next {
            AppOverlay::None
        } else {
            next
        };
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
                Some(AppAction::Invoke(pending))
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
            KeyCode::Char('t' | 'T') => self.active_id().map(|id| ControlAction::Stop { id }),
            KeyCode::Char('d' | 'D') => self.active_id().map(|id| ControlAction::Delete { id }),
            _ => None,
        }
    }

    fn active_resume_action(&self) -> Option<ControlAction> {
        let session = self.state.active_session()?;
        if !matches!(
            session.lifecycle,
            SessionLifecycle::Idle | SessionLifecycle::Suspended | SessionLifecycle::Failed
        ) {
            return None;
        }
        Some(ControlAction::Resume {
            name: session.id.clone(),
        })
    }

    fn active_checkpoint_action(&self) -> Option<ControlAction> {
        let session = self.state.active_session()?;
        if !session.persistent || !matches!(session.lifecycle, SessionLifecycle::Working) {
            return None;
        }
        Some(ControlAction::Checkpoint {
            id: session.id.clone(),
        })
    }

    fn active_suspend_action(&self) -> Option<ControlAction> {
        let session = self.state.active_session()?;
        if !session.persistent || !matches!(session.lifecycle, SessionLifecycle::Working) {
            return None;
        }
        Some(ControlAction::Suspend {
            id: session.id.clone(),
        })
    }

    fn active_id(&self) -> Option<String> {
        self.state
            .active_session()
            .map(|session| session.id.clone())
    }

    fn open_create(&mut self) {
        self.pending_action = None;
        self.fork_draft = None;
        self.create_draft = Some(CreateDraft {
            name: next_tmp_name(&self.state),
            selected_profile: default_profile_index(&self.state),
        });
        self.overlay = AppOverlay::Create;
    }

    fn open_fork(&mut self) -> bool {
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

    fn handle_create_key(&mut self, key: KeyEvent) -> AppAction {
        match key.code {
            KeyCode::Esc => {
                self.create_draft = None;
                self.overlay = AppOverlay::None;
                AppAction::Consumed
            }
            KeyCode::Enter => {
                let Some(draft) = self.create_draft.clone() else {
                    self.overlay = AppOverlay::None;
                    return AppAction::Consumed;
                };
                let name = draft.name.trim().to_string();
                if name.is_empty() {
                    return AppAction::Consumed;
                }
                let profile_id = selected_profile_id(&self.state, draft.selected_profile);
                self.create_draft = None;
                self.overlay = AppOverlay::None;
                AppAction::Invoke(ControlAction::CreateSession { name, profile_id })
            }
            KeyCode::Up => {
                if let Some(draft) = &mut self.create_draft {
                    draft.selected_profile = draft.selected_profile.saturating_sub(1);
                }
                AppAction::Consumed
            }
            KeyCode::Down => {
                let max_index = self.state.profiles.len().saturating_sub(1);
                if let Some(draft) = &mut self.create_draft {
                    draft.selected_profile =
                        draft.selected_profile.saturating_add(1).min(max_index);
                }
                AppAction::Consumed
            }
            KeyCode::Backspace => {
                if let Some(draft) = &mut self.create_draft {
                    draft.name.pop();
                }
                AppAction::Consumed
            }
            KeyCode::Char(ch)
                if !key.modifiers.intersects(
                    KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER,
                ) =>
            {
                if let Some(draft) = &mut self.create_draft {
                    draft.name.push(ch);
                }
                AppAction::Consumed
            }
            _ => AppAction::Consumed,
        }
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
                AppAction::Invoke(ControlAction::Fork {
                    id: draft.source_id,
                    name,
                })
            }
            KeyCode::Backspace => {
                if let Some(draft) = &mut self.fork_draft {
                    draft.name.pop();
                }
                AppAction::Consumed
            }
            KeyCode::Char(ch)
                if !key.modifiers.intersects(
                    KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER,
                ) =>
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

fn default_profile_index(state: &AppState) -> usize {
    state
        .profiles
        .iter()
        .position(|profile| profile.is_default)
        .unwrap_or_default()
}

fn selected_profile_id(state: &AppState, index: usize) -> String {
    state
        .profiles
        .get(index)
        .or_else(|| state.profiles.first())
        .map(|profile| profile.id.clone())
        .unwrap_or_else(|| "default".to_string())
}

fn next_tmp_name(state: &AppState) -> String {
    for index in 1..1000 {
        let candidate = format!("tmp-{index}");
        if state.sessions.iter().all(|session| session.id != candidate) {
            return candidate;
        }
    }
    "tmp".to_string()
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
    value
        .to_digit(10)
        .map(|digit| digit.saturating_sub(1) as usize)
}
