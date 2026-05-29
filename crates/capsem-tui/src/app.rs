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
    Confirm,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ControlAction {
    CreateEphemeral,
    Resume { name: String },
    Suspend { id: String },
    Stop { id: String },
    Delete { id: String },
}

impl ControlAction {
    pub const fn label(&self) -> &'static str {
        match self {
            Self::CreateEphemeral => "create",
            Self::Resume { .. } => "resume",
            Self::Suspend { .. } => "suspend",
            Self::Stop { .. } => "stop",
            Self::Delete { .. } => "delete",
        }
    }

    pub fn target(&self) -> &str {
        match self {
            Self::CreateEphemeral => "new ephemeral session",
            Self::Resume { name }
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
}

impl App {
    pub fn new(state: AppState) -> Self {
        let active_index = state
            .sessions
            .iter()
            .position(|session| session.id == state.active_session_id)
            .unwrap_or_default();
        Self {
            state,
            active_index,
            overlay: AppOverlay::None,
            pending_action: None,
        }
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
        if self.handle_overlay_key(key) {
            return AppAction::Consumed;
        }
        if let Some(action) = self.control_action_for_key(key) {
            self.pending_action = Some(action);
            self.overlay = AppOverlay::Confirm;
            return AppAction::Consumed;
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
            KeyCode::Char('o' | 'O') => AppOverlay::Home,
            _ => return false,
        };
        self.overlay = if self.overlay == next {
            AppOverlay::None
        } else {
            next
        };
        self.pending_action = None;
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
            KeyCode::Char('n' | 'N') => Some(ControlAction::CreateEphemeral),
            KeyCode::Char('r' | 'R') => self.active_resume_action(),
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

fn is_alt_key(modifiers: KeyModifiers) -> bool {
    modifiers.contains(KeyModifiers::ALT)
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
