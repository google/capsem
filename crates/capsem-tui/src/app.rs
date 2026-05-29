use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::model::AppState;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AppAction {
    Consumed,
    Forward,
    Exit,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum AppOverlay {
    #[default]
    None,
    Help,
    Stats,
    Home,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct App {
    state: AppState,
    active_index: usize,
    overlay: AppOverlay,
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
        }
    }

    pub fn state(&self) -> &AppState {
        &self.state
    }

    pub fn overlay(&self) -> AppOverlay {
        self.overlay
    }

    pub fn replace_state(&mut self, mut state: AppState) {
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

    pub fn handle_key(&mut self, key: KeyEvent) -> AppAction {
        if is_exit_key(key) {
            return AppAction::Exit;
        }
        if self.handle_overlay_key(key) {
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
        true
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

fn is_previous_key(key: KeyEvent) -> bool {
    is_control_key(key.modifiers) && matches!(key.code, KeyCode::Left)
}

fn is_next_key(key: KeyEvent) -> bool {
    is_control_key(key.modifiers) && matches!(key.code, KeyCode::Right)
}

fn is_control_key(modifiers: KeyModifiers) -> bool {
    modifiers.intersects(KeyModifiers::SUPER | KeyModifiers::CONTROL | KeyModifiers::ALT)
}

fn select_index(key: KeyEvent) -> Option<usize> {
    if !is_control_key(key.modifiers) {
        return None;
    }
    let KeyCode::Char(value) = key.code else {
        return None;
    };
    value
        .to_digit(10)
        .map(|digit| digit.saturating_sub(1) as usize)
}
