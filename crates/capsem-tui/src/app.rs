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
    prefix_pending: bool,
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
            prefix_pending: false,
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

    pub fn prefix_pending(&self) -> bool {
        self.prefix_pending
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
        if self.prefix_pending {
            self.prefix_pending = false;
            return self.handle_prefix_key(key);
        }
        if is_prefix_key(key) {
            self.prefix_pending = true;
            self.pending_action = None;
            self.overlay = AppOverlay::None;
            return AppAction::Consumed;
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

    fn handle_prefix_key(&mut self, key: KeyEvent) -> AppAction {
        match key.code {
            KeyCode::Char('h') | KeyCode::Char('p') | KeyCode::Left => {
                self.previous_session();
                AppAction::Consumed
            }
            KeyCode::Char('l') | KeyCode::Char('n') | KeyCode::Right => {
                self.next_session();
                AppAction::Consumed
            }
            KeyCode::Char(ch) if ch.is_ascii_digit() => {
                let index = ch.to_digit(10).unwrap_or_default();
                if index > 0 {
                    self.select_session(index as usize - 1);
                }
                AppAction::Consumed
            }
            KeyCode::Esc => AppAction::Consumed,
            _ if is_prefix_key(key) => AppAction::Forward,
            _ => AppAction::Consumed,
        }
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
        let next = match key.code {
            KeyCode::F(1) => AppOverlay::Help,
            KeyCode::F(2) => AppOverlay::Stats,
            KeyCode::F(3) => AppOverlay::Home,
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
        match key.code {
            KeyCode::F(4) => Some(ControlAction::CreateEphemeral),
            KeyCode::F(5) => self.active_resume_action(),
            KeyCode::F(6) => self.active_suspend_action(),
            KeyCode::F(7) => self.active_id().map(|id| ControlAction::Stop { id }),
            KeyCode::F(8) => self.active_id().map(|id| ControlAction::Delete { id }),
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
    let modifiers = key.modifiers;
    matches!(
        (key.code, modifiers),
        (KeyCode::Char('q'), KeyModifiers::SUPER)
            | (KeyCode::Esc, KeyModifiers::CONTROL)
            | (KeyCode::F(10), KeyModifiers::NONE)
    )
}

fn is_prefix_key(key: KeyEvent) -> bool {
    matches!(
        (key.code, key.modifiers),
        (KeyCode::Char('b'), KeyModifiers::CONTROL)
    )
}

fn is_previous_key(key: KeyEvent) -> bool {
    is_alt_key(key.modifiers)
        && matches!(
            key.code,
            KeyCode::Left | KeyCode::Char('h') | KeyCode::Char('p')
        )
}

fn is_next_key(key: KeyEvent) -> bool {
    is_alt_key(key.modifiers)
        && matches!(
            key.code,
            KeyCode::Right | KeyCode::Char('l') | KeyCode::Char('n')
        )
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
