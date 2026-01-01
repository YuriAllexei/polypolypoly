//! UI widgets for the visualizer

pub mod orderbook;
pub mod sidebar;

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use super::App;

/// Draw the main UI layout
pub fn draw(frame: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),  // Header
            Constraint::Min(0),     // Main content
            Constraint::Length(3),  // Footer
        ])
        .split(frame.area());

    draw_header(frame, app, chunks[0]);
    draw_main(frame, app, chunks[1]);
    draw_footer(frame, app, chunks[2]);
}

fn draw_header(frame: &mut Frame, app: &App, area: Rect) {
    let connected = app.is_oms_connected();
    let status = if connected { "Connected" } else { "Connecting..." };
    let status_color = if connected { Color::Green } else { Color::Yellow };

    let market_count = app.markets.len();
    let order_count = app.get_total_order_count();

    let header_text = format!(
        " Status: {} | Markets: {} | Orders: {}",
        status, market_count, order_count
    );

    let header = Paragraph::new(header_text)
        .style(Style::default().fg(status_color))
        .block(Block::default().borders(Borders::ALL).title(" MM Visualizer "));

    frame.render_widget(header, area);
}

fn draw_main(frame: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(28),  // Sidebar (just market names)
            Constraint::Min(0),      // Orderbook area
        ])
        .split(area);

    sidebar::draw(frame, app, chunks[0]);

    // Draw orderbook for selected market
    if let Some(market) = app.get_selected_market() {
        orderbook::draw(frame, app, market, chunks[1]);
    } else {
        let empty = Paragraph::new(" No market selected. Use j/k to navigate.")
            .block(Block::default().borders(Borders::ALL).title(" Orderbook "));
        frame.render_widget(empty, chunks[1]);
    }
}

fn draw_footer(frame: &mut Frame, app: &App, area: Rect) {
    let status = app.status_message.as_deref().unwrap_or("");
    let position_summary = app.get_position_summary();

    let footer_text = if status.is_empty() {
        format!(" {} | q=quit j/k=nav r=refresh x=cancel d=dump", position_summary)
    } else {
        format!(" {} | {}", position_summary, status)
    };

    let footer = Paragraph::new(footer_text)
        .block(Block::default().borders(Borders::ALL));

    frame.render_widget(footer, area);
}
