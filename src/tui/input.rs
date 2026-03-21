use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::app::{Msg, View};

/// Map a key event to a message, given the current view.
pub fn handle_key(key: KeyEvent, view: View, search_active: bool) -> Option<Msg> {
    // Ctrl-C always quits
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        return Some(Msg::Quit);
    }

    match view {
        View::Picker => handle_picker_key(key, search_active),
        View::Conversation => handle_conversation_key(key),
    }
}

fn handle_picker_key(key: KeyEvent, search_active: bool) -> Option<Msg> {
    match key.code {
        KeyCode::Up => Some(Msg::MoveUp),
        KeyCode::Down => Some(Msg::MoveDown),
        KeyCode::Enter => Some(Msg::Select),
        KeyCode::Tab => Some(Msg::CycleAgentFilter),
        KeyCode::Backspace => {
            if search_active {
                Some(Msg::SearchBackspace)
            } else {
                None
            }
        }
        KeyCode::Esc => Some(Msg::Back),
        KeyCode::PageUp => Some(Msg::PageUp),
        KeyCode::PageDown => Some(Msg::PageDown),
        KeyCode::Char(c) => {
            if !search_active {
                // When search is not active, j/k navigate, q quits
                match c {
                    'j' => Some(Msg::MoveDown),
                    'k' => Some(Msg::MoveUp),
                    'q' => Some(Msg::Quit),
                    _ => Some(Msg::SearchInput(c)),
                }
            } else {
                // When search is active, all chars go to search
                Some(Msg::SearchInput(c))
            }
        }
        _ => None,
    }
}

fn handle_conversation_key(key: KeyEvent) -> Option<Msg> {
    match key.code {
        KeyCode::Up | KeyCode::Char('k') => Some(Msg::ScrollUp),
        KeyCode::Down | KeyCode::Char('j') => Some(Msg::ScrollDown),
        KeyCode::PageUp => Some(Msg::PageUp),
        KeyCode::PageDown => Some(Msg::PageDown),
        KeyCode::Char('u') => Some(Msg::ConfirmUpload),
        KeyCode::Esc => Some(Msg::Back),
        KeyCode::Char('q') => Some(Msg::Quit),
        _ => None,
    }
}
