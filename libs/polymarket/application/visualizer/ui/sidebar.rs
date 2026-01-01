//! Sidebar widget - displays list of markets

use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState},
    Frame,
};

use crate::application::visualizer::App;

/// Draw the sidebar with market list
pub fn draw(frame: &mut Frame, app: &App, area: Rect) {
    let items: Vec<ListItem> = app
        .markets
        .iter()
        .enumerate()
        .map(|(i, market)| {
            let is_selected = i == app.selected_index;

            let style = if is_selected {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };

            let prefix = if is_selected { "> " } else { "  " };

            // Truncate market name to fit sidebar
            let name = if market.display_name.len() > 24 {
                format!("{}...", &market.display_name[..21])
            } else {
                market.display_name.clone()
            };

            let content = Line::from(vec![
                Span::styled(prefix, style),
                Span::styled(name, style),
            ]);

            ListItem::new(content)
        })
        .collect();

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(" Markets "))
        .highlight_style(Style::default().add_modifier(Modifier::BOLD));

    // Create a list state for the selected item
    let mut state = ListState::default();
    state.select(Some(app.selected_index));

    frame.render_stateful_widget(list, area, &mut state);
}
