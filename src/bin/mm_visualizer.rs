//! MM Visualizer - Terminal UI for visualizing orderbooks and orders
//!
//! Uses the same real-time WebSocket components as the strategy:
//! - OMS (OrderStateStore) for our orders
//! - PositionTracker for positions
//! - Orderbooks via per-market WebSocket connections

use std::io;
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};

use polymarket::application::visualizer::{ui, App};

/// Interval for auto-refreshing markets (check for new orders/markets)
const MARKET_REFRESH_INTERVAL: Duration = Duration::from_secs(5);

fn main() -> Result<()> {
    // Load environment variables
    dotenv::dotenv().ok();

    // Note: Logging is disabled for TUI - it would corrupt the alternate screen display

    // Get database URL from environment (VISUALIZER_DATABASE_URL takes precedence for local dev)
    let database_url = std::env::var("VISUALIZER_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .map_err(|_| anyhow::anyhow!("VISUALIZER_DATABASE_URL or DATABASE_URL environment variable is required"))?;

    // Create tokio runtime
    let runtime = tokio::runtime::Runtime::new()?;

    // Initialize the app (connects to database, WebSockets, etc.)
    let mut app = runtime.block_on(async {
        App::initialize(runtime.handle().clone(), &database_url).await
    })?;

    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Run the main loop
    let result = run_app(&mut terminal, &mut app);

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    // Shutdown app
    app.shutdown();

    if let Err(e) = result {
        eprintln!("Error: {}", e);
    }

    Ok(())
}

fn run_app<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
) -> Result<()> {
    let mut last_market_refresh = Instant::now();

    loop {
        // Draw UI
        terminal.draw(|frame| ui::draw(frame, app))?;

        // Auto-refresh markets periodically (add new markets, remove inactive ones)
        if last_market_refresh.elapsed() >= MARKET_REFRESH_INTERVAL {
            app.refresh_markets();
            last_market_refresh = Instant::now();
        }

        // Handle input with 10ms timeout (for real-time updates)
        if event::poll(Duration::from_millis(10))? {
            if let Event::Key(key) = event::read()? {
                // Only handle key press events (not release)
                if key.kind == KeyEventKind::Press {
                    match key.code {
                        KeyCode::Char('q') => {
                            app.should_quit = true;
                        }
                        KeyCode::Char('j') | KeyCode::Down => {
                            app.next_market();
                        }
                        KeyCode::Char('k') | KeyCode::Up => {
                            app.prev_market();
                        }
                        KeyCode::Char('r') => {
                            // Manual refresh (in addition to auto-refresh)
                            app.refresh_markets();
                        }
                        KeyCode::Char('x') => {
                            // Cancel all open orders
                            app.cancel_all_orders();
                        }
                        KeyCode::Char('d') => {
                            // Dump all inventory for selected market
                            app.dump_inventory();
                        }
                        _ => {}
                    }
                }
            }
        }

        if app.should_quit {
            break;
        }
    }

    Ok(())
}
