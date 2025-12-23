//! Market tracker for the Market Merger strategy
//!
//! Main accumulation loop for a single market.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::RwLock;
use tokio::time::sleep;
use tracing::{debug, info, warn};

use crate::application::strategies::market_merger::config::MarketMergerConfig;
use crate::application::strategies::market_merger::services::{
    MergeChecker, OpportunityScanner, QuoteCalculator, SizeCalculator,
};
use crate::application::strategies::market_merger::tracker::QuoteManager;
use crate::application::strategies::market_merger::types::{MarketContext, MarketState};
use crate::domain::orderbook::Orderbook;
use crate::domain::DbMarket;
use crate::infrastructure::client::clob::TradingClient;
use crate::infrastructure::{BalanceManager, OrderStateStore, OrderType, Side};

/// Context for running the accumulator
pub struct AccumulatorContext {
    pub shutdown_flag: Arc<AtomicBool>,
    pub trading: Arc<TradingClient>,
    pub balance_manager: Arc<RwLock<BalanceManager>>,
    pub order_state: Arc<tokio::sync::RwLock<OrderStateStore>>,
}

/// Run the accumulator loop for a single market
pub async fn run_accumulator(
    market: DbMarket,
    config: MarketMergerConfig,
    ctx: AccumulatorContext,
) -> anyhow::Result<()> {
    // Initialize market context
    let market_ctx = MarketContext::from_market(&market)?;
    let mut state = MarketState::new();

    info!(
        "Starting accumulator for {} ({} {})",
        market_ctx.market_id, market_ctx.crypto_asset, market_ctx.timeframe
    );

    // Initialize services
    let quote_calculator = QuoteCalculator::new(&config);
    let size_calculator = SizeCalculator::new(&config);
    let opportunity_scanner = OpportunityScanner::new(&config);
    let merge_checker = MergeChecker::new(&config);
    let quote_manager = QuoteManager::new(ctx.trading.clone(), ctx.order_state.clone());

    // TODO: Hydrate positions from Polymarket Data API (for restart recovery)
    // hydrate_positions_from_api(&mut state, &market_ctx).await?;

    // TODO: Connect orderbook WebSocket
    // let (orderbooks, _ws_client) = connect_orderbook_ws(&market_ctx).await?;

    let refresh_interval = Duration::from_millis(config.quote_refresh_ms);
    let mut last_refresh = Instant::now();

    // Main loop
    loop {
        // 1. Check shutdown
        if !ctx.shutdown_flag.load(Ordering::Acquire) {
            info!("Shutdown requested, stopping accumulator");
            break;
        }

        // 2. Check trading halt
        let is_halted = ctx.balance_manager.read().is_halted();
        if is_halted {
            warn!("Trading halted, canceling all bids");
            if let Err(e) = quote_manager.cancel_all(&mut state).await {
                warn!("Failed to cancel bids on halt: {}", e);
            }
            sleep(Duration::from_secs(1)).await;
            continue;
        }

        // 3. Get orderbooks (placeholder - needs WebSocket integration)
        let (up_ob, down_ob) = match get_orderbooks(&market_ctx).await {
            Ok(obs) => obs,
            Err(e) => {
                warn!("Failed to get orderbooks: {}", e);
                sleep(Duration::from_millis(500)).await;
                continue;
            }
        };

        // 4. Sync positions from fills (WebSocket updates via OrderStateStore)
        sync_positions_from_fills(&mut state, &ctx.order_state, &market_ctx).await;

        // 5. Update sizing phase based on position value
        size_calculator.update_phase(&mut state);

        // 6. Check if we should merge (continuous merging)
        let merge_decision = merge_checker.should_merge(&state);
        if merge_decision.should_merge {
            info!(
                "Merge triggered: {} pairs, expected profit ${:.2}",
                merge_decision.pairs, merge_decision.expected_profit
            );
            // TODO: User implements merge execution
            // execute_merge(&market_ctx, merge_decision.pairs).await?;
        }

        // 7. Scan for taker opportunities (opportunity-based, not threshold-based)
        if let Some(opp) = opportunity_scanner.scan(&market_ctx, &state, &up_ob, &down_ob) {
            info!(
                "Taker opportunity: {} (score: {:.1})",
                opp, opp.score
            );
            // Execute taker order
            if let Err(e) = execute_taker(&ctx.trading, &opp).await {
                warn!("Failed to execute taker: {}", e);
            }
        }

        // 8. Refresh quote ladder periodically
        if last_refresh.elapsed() >= refresh_interval {
            let balance = ctx.balance_manager.read().current_balance();

            // Calculate bid prices
            let mut ladder = quote_calculator.calculate_bids(&market_ctx, &state, &up_ob, &down_ob);

            // Calculate sizes
            size_calculator.calculate_sizes(&state, balance, &mut ladder);

            // Update bids
            if let Err(e) = quote_manager.update_bids(&market_ctx, &mut state, &ladder).await {
                warn!("Failed to update bids: {}", e);
            }

            last_refresh = Instant::now();

            // Log state periodically
            debug!(
                "State: Up={:.1}@${:.3}, Down={:.1}@${:.3}, Combined=${:.3}, Phase={}",
                state.up_size,
                state.up_avg_cost,
                state.down_size,
                state.down_avg_cost,
                state.combined_cost(),
                state.phase
            );
        }

        sleep(Duration::from_millis(50)).await;
    }

    // Cleanup: cancel all bids
    info!("Cleaning up, canceling all bids");
    if let Err(e) = quote_manager.cancel_all(&mut state).await {
        warn!("Failed to cancel bids on cleanup: {}", e);
    }

    Ok(())
}

/// Get orderbooks for Up and Down tokens
/// TODO: Integrate with WebSocket orderbook manager
async fn get_orderbooks(ctx: &MarketContext) -> anyhow::Result<(Orderbook, Orderbook)> {
    // Placeholder - needs real WebSocket integration
    // This should connect to the orderbook WebSocket and get live data
    Ok((
        Orderbook::new(ctx.up_token_id.clone()),
        Orderbook::new(ctx.down_token_id.clone()),
    ))
}

/// Sync positions from fill events
async fn sync_positions_from_fills(
    state: &mut MarketState,
    order_state: &Arc<tokio::sync::RwLock<OrderStateStore>>,
    ctx: &MarketContext,
) {
    // Get fills from order state store
    let fills = {
        let store = order_state.read().await;
        let mut all_fills = store.get_fills(&ctx.up_token_id);
        all_fills.extend(store.get_fills(&ctx.down_token_id));
        all_fills
    };

    // Apply each fill to state
    for fill in fills {
        let is_up = fill.asset_id == ctx.up_token_id;
        state.apply_fill(&fill.asset_id, is_up, fill.price, fill.size);

        debug!(
            "Applied fill: {} {} @ ${:.3} x {:.1}",
            if is_up { "Up" } else { "Down" },
            fill.asset_id,
            fill.price,
            fill.size
        );
    }
}

/// Execute a taker order
async fn execute_taker(
    trading: &TradingClient,
    opp: &crate::application::strategies::market_merger::types::TakerOpportunity,
) -> anyhow::Result<()> {
    info!(
        "Executing taker: {} @ ${:.3} x {:.1} (score: {:.1})",
        if opp.is_up { "Up" } else { "Down" },
        opp.price,
        opp.size,
        opp.score
    );

    // Place FOK (fill or kill) order for taker
    trading
        .place_order(
            &opp.token_id,
            opp.price,
            opp.size,
            Side::Buy,
            OrderType::FOK,
        )
        .await?;

    Ok(())
}
