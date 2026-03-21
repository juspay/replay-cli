use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

use crate::models::relative_time;

use super::app::App;

/// Render the picker view.
pub fn render(f: &mut Frame, app: &App) {
    let area = f.area();

    let chunks = Layout::vertical([
        Constraint::Length(3), // search bar
        Constraint::Min(1),   // session list
        Constraint::Length(1), // status bar
    ])
    .split(area);

    render_search_bar(f, app, chunks[0]);
    render_session_list(f, app, chunks[1]);
    render_status_bar(f, app, chunks[2]);
}

fn render_search_bar(f: &mut Frame, app: &App, area: Rect) {
    let filter_label = app.agent_filter.label();

    let cursor = if app.search_query.is_empty() {
        "_"
    } else {
        ""
    };

    let search_line = Line::from(vec![
        Span::styled("REPLAY", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" > "),
        Span::styled(
            format!("{}{}", app.search_query, cursor),
            Style::default().add_modifier(Modifier::DIM),
        ),
    ]);

    let filter_span = Span::styled(
        format!("[{}]", filter_label),
        Style::default().add_modifier(Modifier::DIM),
    );

    // Render search in a bordered block with filter on the right
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().add_modifier(Modifier::DIM))
        .title_bottom(Line::from(filter_span).right_aligned());

    let search = Paragraph::new(search_line).block(block);
    f.render_widget(search, area);
}

fn render_session_list(f: &mut Frame, app: &App, area: Rect) {
    let items: Vec<ListItem> = app
        .filtered_indices
        .iter()
        .map(|&idx| {
            let session = &app.all_sessions[idx];

            let agent_label = session.agent.label();
            let time_str = relative_time(&session.timestamp);
            let path = session
                .project_path
                .as_deref()
                .map(shorten_path)
                .unwrap_or_default();
            let branch = session.git_branch.as_deref().unwrap_or("");

            // Line 1: agent  time  path  branch
            let line1 = Line::from(vec![
                Span::styled(
                    format!("{:<9}", agent_label),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("{:<9}", time_str),
                    Style::default().add_modifier(Modifier::DIM),
                ),
                Span::styled(
                    format!("{:<30}", path),
                    Style::default().add_modifier(Modifier::DIM),
                ),
                Span::styled(branch, Style::default().add_modifier(Modifier::DIM)),
            ]);

            // Line 2: preview
            let preview = session
                .first_user_message
                .as_deref()
                .unwrap_or("(no message)")
                .replace('\n', " ");
            let preview: String = preview.chars().take(area.width as usize - 4).collect();

            let line2 = Line::from(vec![Span::styled(
                format!("  > {}", preview),
                Style::default(),
            )]);

            ListItem::new(vec![line1, line2])
        })
        .collect();

    let list = List::new(items)
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
        .highlight_spacing(ratatui::widgets::HighlightSpacing::Always);

    let mut state = ListState::default();
    if !app.filtered_indices.is_empty() {
        state.select(Some(app.selected_index));
    }

    f.render_stateful_widget(list, area, &mut state);
}

fn render_status_bar(f: &mut Frame, app: &App, area: Rect) {
    let count = app.filtered_indices.len();
    let total = app.all_sessions.len();

    let status = if count == total {
        format!("{} sessions", total)
    } else {
        format!("{}/{} sessions", count, total)
    };

    let line = Line::from(vec![
        Span::styled(
            format!(" {} ", status),
            Style::default().add_modifier(Modifier::DIM),
        ),
        Span::raw(" "),
        Span::styled(
            "[ENTER:view]",
            Style::default().add_modifier(Modifier::DIM),
        ),
        Span::raw(" "),
        Span::styled(
            "[TAB:filter]",
            Style::default().add_modifier(Modifier::DIM),
        ),
        Span::raw(" "),
        Span::styled("[Q:quit]", Style::default().add_modifier(Modifier::DIM)),
    ]);

    let bar = Paragraph::new(line);
    f.render_widget(bar, area);
}

/// Shorten a path like /Users/nishant/Workspace/foo to ~/Workspace/foo
fn shorten_path(path: &str) -> String {
    if let Some(home) = dirs::home_dir() {
        let home_str = home.to_string_lossy();
        if path.starts_with(home_str.as_ref()) {
            return format!("~{}", &path[home_str.len()..]);
        }
    }
    path.to_string()
}
