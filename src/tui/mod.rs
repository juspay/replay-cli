pub mod app;
pub mod conversation;
pub mod input;
pub mod picker;

use std::io;
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self, Event};
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::execute;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use app::{App, View};
use input::handle_key;

/// Run the TUI event loop.
pub fn run_tui(mut app: App) -> Result<()> {
    // Setup terminal
    terminal::enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    // Event loop
    let result = event_loop(&mut terminal, &mut app);

    // Restore terminal
    terminal::disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
) -> Result<()> {
    loop {
        // Render
        terminal.draw(|f| {
            match app.view {
                View::Picker => picker::render(f, app),
                View::Conversation => conversation::render(f, app),
            }
        })?;

        if app.should_quit {
            break;
        }

        // Poll for events
        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                let search_active = !app.search_query.is_empty();
                if let Some(msg) = handle_key(key, app.view, search_active) {
                    app.update(msg);
                }
            }
        }
    }

    Ok(())
}
