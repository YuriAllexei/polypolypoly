//! Orderbook ladder widget - displays vertical orderbook with highlighting

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::application::visualizer::{App, MarketInfo};

const MAX_LEVELS: usize = 10;
const PRICE_EPSILON: f64 = 0.0001;

/// Draw the orderbook for a selected market
pub fn draw(frame: &mut Frame, app: &App, market: &MarketInfo, area: Rect) {
    let title = format!(" {} ", market.question_short());

    let block = Block::default().borders(Borders::ALL).title(title);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Split into two columns for UP and DOWN tokens
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(inner);

    draw_token_ladder(frame, app, market, true, columns[0]);
    draw_token_ladder(frame, app, market, false, columns[1]);
}

fn draw_token_ladder(frame: &mut Frame, app: &App, market: &MarketInfo, is_up: bool, area: Rect) {
    let token_id = if is_up { &market.up_token_id } else { &market.down_token_id };
    let outcome_name = if is_up { &market.up_outcome } else { &market.down_outcome };

    // Get order count for this token
    let order_count = count_token_orders(app, token_id);

    // Get position for this token
    let (size, avg_price) = get_token_position(app, token_id);

    // Build title with outcome name, order count, and position
    let pos_str = if size.abs() > 0.01 {
        format!("{:.0}@{:.2}", size, avg_price)
    } else {
        "0".to_string()
    };

    let title = format!(" {} ({} ord) | {} ", outcome_name, order_count, pos_str);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(title);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Get orderbook data
    let (asks, bids, spread) = app.get_orderbook_levels(token_id);

    // Get our orders at each price level
    let our_orders = app.get_our_orders_for_token(token_id);

    // Calculate layout: asks (top), spread (middle), bids (bottom)
    let ask_height = asks.len().min(MAX_LEVELS) as u16;
    let bid_height = bids.len().min(MAX_LEVELS) as u16;

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(ask_height.max(1)),
            Constraint::Length(1), // Spread line
            Constraint::Length(bid_height.max(1)),
            Constraint::Min(0), // Remaining space
        ])
        .split(inner);

    // Draw asks (lowest price closest to spread - take best asks and reverse for display)
    // Asks come sorted ascending (lowest first), we want lowest at bottom near spread
    let ask_lines: Vec<Line> = asks
        .iter()
        .take(MAX_LEVELS)
        .rev()  // Reverse so lowest price is at bottom (closest to spread)
        .map(|(price, size)| {
            let our_size = get_our_size_at_price(&our_orders, *price);
            format_level(*price, *size, our_size, false)
        })
        .collect();

    let asks_widget = Paragraph::new(ask_lines);
    frame.render_widget(asks_widget, chunks[0]);

    // Draw spread
    let spread_text = format!("──── Spread: {:.4} ────", spread.unwrap_or(0.0));
    let spread_widget = Paragraph::new(spread_text)
        .style(Style::default().fg(Color::DarkGray));
    frame.render_widget(spread_widget, chunks[1]);

    // Draw bids (highest price closest to spread)
    let bid_lines: Vec<Line> = bids
        .iter()
        .take(MAX_LEVELS)
        .map(|(price, size)| {
            let our_size = get_our_size_at_price(&our_orders, *price);
            format_level(*price, *size, our_size, true)
        })
        .collect();

    let bids_widget = Paragraph::new(bid_lines);
    frame.render_widget(bids_widget, chunks[2]);
}

/// Count open orders for a specific token
fn count_token_orders(app: &App, token_id: &str) -> usize {
    let oms = app.order_state.read();
    let mut count = 0;
    for order in oms.get_bids(token_id) {
        if order.is_open() {
            count += 1;
        }
    }
    for order in oms.get_asks(token_id) {
        if order.is_open() {
            count += 1;
        }
    }
    count
}

/// Get position size and avg entry price for a token
fn get_token_position(app: &App, token_id: &str) -> (f64, f64) {
    app.position_tracker
        .read()
        .get_position(token_id)
        .map(|p| (p.size, p.avg_entry_price))
        .unwrap_or((0.0, 0.0))
}

/// Format a single price level line
fn format_level(price: f64, total_size: f64, our_size: Option<f64>, is_bid: bool) -> Line<'static> {
    let side_label = if is_bid { "BID" } else { "ASK" };
    let base_color = if is_bid { Color::Green } else { Color::Red };

    // Check if we have orders at this level
    if let Some(ours) = our_size {
        // We have orders here - highlight in blue
        let text = format!(
            " {} {:.4} {:>8.1}/{:.1}",
            side_label, price, total_size, ours
        );
        Line::from(Span::styled(
            text,
            Style::default()
                .fg(Color::White)
                .bg(Color::Blue)
                .add_modifier(Modifier::BOLD),
        ))
    } else {
        // No orders here - normal coloring
        let text = format!(" {} {:.4} {:>8.1}", side_label, price, total_size);
        Line::from(Span::styled(text, Style::default().fg(base_color)))
    }
}

/// Get our aggregated size at a specific price level (sum of all orders at this price)
fn get_our_size_at_price(our_orders: &[(f64, f64)], price: f64) -> Option<f64> {
    let total: f64 = our_orders
        .iter()
        .filter(|(p, _)| (*p - price).abs() < PRICE_EPSILON)
        .map(|(_, size)| *size)
        .sum();

    if total > 0.0 {
        Some(total)
    } else {
        None
    }
}
