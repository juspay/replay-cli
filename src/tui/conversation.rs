use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Frame;

use crate::models::{relative_time, Role};

use super::app::App;

/// Render the conversation preview view.
pub fn render(f: &mut Frame, app: &App) {
    if app.loaded_session.is_none() {
        return;
    }

    let area = f.area();

    let chunks = Layout::vertical([
        Constraint::Length(3), // header
        Constraint::Min(1),   // messages
        Constraint::Length(1), // status bar
    ])
    .split(area);

    render_header(f, app, chunks[0]);
    render_messages(f, app, chunks[1]);
    render_status_bar(f, app, chunks[2]);
}

fn render_header(f: &mut Frame, app: &App, area: Rect) {
    let loaded = match &app.loaded_session {
        Some(l) => l,
        None => return,
    };

    let session = &loaded.session;
    let path = session.project_path.as_deref().unwrap_or("");
    let branch = session.git_branch.as_deref().unwrap_or("");
    let time_str = relative_time(&session.timestamp);

    let line = Line::from(vec![
        Span::styled(
            session.agent.label(),
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(path, Style::default().add_modifier(Modifier::DIM)),
        Span::raw("  "),
        Span::styled(branch, Style::default().add_modifier(Modifier::DIM)),
        Span::raw("  "),
        Span::styled(time_str, Style::default().add_modifier(Modifier::DIM)),
    ]);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().add_modifier(Modifier::DIM));

    let header = Paragraph::new(line).block(block);
    f.render_widget(header, area);
}

fn render_messages(f: &mut Frame, app: &App, area: Rect) {
    let loaded = match &app.loaded_session {
        Some(l) => l,
        None => return,
    };

    let width = area.width.saturating_sub(2) as usize;
    let mut lines: Vec<Line> = Vec::new();

    for msg in &loaded.messages {
        // Separator line
        let (label, label_style) = match msg.role {
            Role::User => (
                "YOU",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Role::Assistant => (
                "ASSISTANT",
                Style::default().add_modifier(Modifier::DIM),
            ),
        };

        // Build rule line: ── LABEL ────────
        let prefix = format!("── {} ", label);
        let rule_len = width.saturating_sub(prefix.len());
        let rule = "─".repeat(rule_len);

        lines.push(Line::from(vec![])); // blank line
        lines.push(Line::from(vec![
            Span::styled(prefix, label_style),
            Span::styled(rule, Style::default().add_modifier(Modifier::DIM)),
        ]));

        // Message text — split into lines, respecting width
        let text_style = match msg.role {
            Role::User => Style::default(),
            Role::Assistant => Style::default(),
        };

        for text_line in msg.text.lines() {
            lines.push(Line::from(Span::styled(text_line, text_style)));
        }
    }

    // Trailing blank
    lines.push(Line::from(vec![]));

    let text = Text::from(lines);
    let paragraph = Paragraph::new(text)
        .wrap(Wrap { trim: false })
        .scroll((app.scroll_offset, 0));

    f.render_widget(paragraph, area);
}

fn render_status_bar(f: &mut Frame, app: &App, area: Rect) {
    let msg_count = app
        .loaded_session
        .as_ref()
        .map_or(0, |l| l.messages.len());

    let mut spans = vec![
        Span::styled(
            format!(" {} messages ", msg_count),
            Style::default().add_modifier(Modifier::DIM),
        ),
        Span::raw(" "),
        Span::styled("[U:upload]", Style::default().add_modifier(Modifier::DIM)),
        Span::raw(" "),
        Span::styled("[ESC:back]", Style::default().add_modifier(Modifier::DIM)),
        Span::raw(" "),
        Span::styled("[Q:quit]", Style::default().add_modifier(Modifier::DIM)),
    ];

    if let Some(ref status) = app.upload_status {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(
            status.clone(),
            Style::default().add_modifier(Modifier::BOLD),
        ));
    }

    let line = Line::from(spans);
    let bar = Paragraph::new(line);
    f.render_widget(bar, area);
}
