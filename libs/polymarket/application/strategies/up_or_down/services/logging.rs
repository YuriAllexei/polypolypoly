//! Logging helpers for the Up or Down strategy.
//!
//! Provides formatted log output for important strategy events.

use crate::application::strategies::up_or_down::types::MarketTrackerContext;
use tracing::{error, info, warn};

/// Log when no asks are detected and timer starts
pub fn log_no_asks_started(ctx: &MarketTrackerContext, token_id: &str, outcome_name: &str) {
    info!(
        "════════════════════════════════════════════════════════════════\n\
         🎯 NO ASKS IN ORDERBOOK - STARTING TIMER\n\
         ════════════════════════════════════════════════════════════════\n\
           Market ID:    {}\n\
           Market:       {}\n\
           URL:          {}\n\
           Price to Beat:{}\n\
           Oracle:       {}\n\
           Asset:        {}\n\
           Timeframe:    {}\n\
           Outcome:      {}\n\
           Token ID:     {}\n\
         ════════════════════════════════════════════════════════════════",
        ctx.market_id,
        ctx.market_question,
        ctx.market_url,
        ctx.format_price_to_beat(),
        ctx.oracle_source,
        ctx.crypto_asset,
        ctx.timeframe,
        outcome_name,
        token_id
    );
}

/// Log when the no-asks threshold is exceeded
pub fn log_threshold_exceeded(
    ctx: &MarketTrackerContext,
    token_id: &str,
    outcome_name: &str,
    elapsed: f64,
    dynamic_threshold: f64,
) {
    info!(
        "════════════════════════════════════════════════════════════════\n\
         ⚡ NO-ASK TIME THRESHOLD EXCEEDED\n\
         ════════════════════════════════════════════════════════════════\n\
           Market ID:      {}\n\
           Market:         {}\n\
           URL:            {}\n\
           Price to Beat:  {}\n\
           Oracle:         {}\n\
           Asset:          {}\n\
           Timeframe:      {}\n\
           Outcome:        {}\n\
           Token ID:       {}\n\
           Elapsed Time:   {:.3} seconds\n\
           Threshold:      {:.3} seconds (dynamic)\n\
         ════════════════════════════════════════════════════════════════",
        ctx.market_id,
        ctx.market_question,
        ctx.market_url,
        ctx.format_price_to_beat(),
        ctx.oracle_source,
        ctx.crypto_asset,
        ctx.timeframe,
        outcome_name,
        token_id,
        elapsed,
        dynamic_threshold
    );
}

/// Log when placing a buy order
pub fn log_placing_order(
    ctx: &MarketTrackerContext,
    token_id: &str,
    outcome_name: &str,
    elapsed: f64,
    dynamic_threshold: f64,
) {
    info!(
        "════════════════════════════════════════════════════════════════\n\
         🚀 PLACING BUY ORDER\n\
         ════════════════════════════════════════════════════════════════\n\
           Market ID:      {}\n\
           Market:         {}\n\
           URL:            {}\n\
           Price to Beat:  {}\n\
           Oracle:         {}\n\
           Asset:          {}\n\
           Timeframe:      {}\n\
           Outcome:        {}\n\
           Token ID:       {}\n\
           Elapsed Time:   {:.3} seconds\n\
           Threshold:      {:.3} seconds (dynamic)\n\
         ════════════════════════════════════════════════════════════════",
        ctx.market_id,
        ctx.market_question,
        ctx.market_url,
        ctx.format_price_to_beat(),
        ctx.oracle_source,
        ctx.crypto_asset,
        ctx.timeframe,
        outcome_name,
        token_id,
        elapsed,
        dynamic_threshold
    );
}

/// Log successful order placement
pub fn log_order_success<T: std::fmt::Debug>(
    ctx: &MarketTrackerContext,
    token_id: &str,
    outcome_name: &str,
    order_id: &T,
) {
    info!(
        "════════════════════════════════════════════════════════════════\n\
         ✅ ORDER PLACED SUCCESSFULLY\n\
         ════════════════════════════════════════════════════════════════\n\
           Order ID:       {:?}\n\
           Market ID:      {}\n\
           Market:         {}\n\
           URL:            {}\n\
           Price to Beat:  {}\n\
           Outcome:        {}\n\
           Timeframe:      {}\n\
           Token ID:       {}\n\
         ════════════════════════════════════════════════════════════════",
        order_id,
        ctx.market_id,
        ctx.market_question,
        ctx.market_url,
        ctx.format_price_to_beat(),
        outcome_name,
        ctx.timeframe,
        token_id
    );
}

/// Log failed order placement
pub fn log_order_failed<E: std::fmt::Display>(
    ctx: &MarketTrackerContext,
    token_id: &str,
    outcome_name: &str,
    error: &E,
) {
    error!(
        "════════════════════════════════════════════════════════════════\n\
         ❌ ORDER PLACEMENT FAILED\n\
         ════════════════════════════════════════════════════════════════\n\
           Error:          {}\n\
           Market ID:      {}\n\
           Market:         {}\n\
           URL:            {}\n\
           Price to Beat:  {}\n\
           Outcome:        {}\n\
           Token ID:       {}\n\
         ════════════════════════════════════════════════════════════════",
        error,
        ctx.market_id,
        ctx.market_question,
        ctx.market_url,
        ctx.format_price_to_beat(),
        outcome_name,
        token_id
    );
}

/// Log when risk is detected (both signals active)
pub fn log_risk_detected(
    ctx: &MarketTrackerContext,
    token_id: &str,
    outcome_name: &str,
    avg_bid_price: f64,
    other_bids: &[f64],
    bps_diff: f64,
    oracle_price: f64,
) {
    warn!(
        "════════════════════════════════════════════════════════════════\n\
         ⚠️  RISK DETECTED - BOTH SIGNALS ACTIVE\n\
         ════════════════════════════════════════════════════════════════\n\
           Market ID:      {}\n\
           Market:         {}\n\
           URL:            {}\n\
           Price to Beat:  {}\n\
           Oracle Price:   ${:.4}\n\
           BPS Difference: {:.4} bps (threshold: {:.4})\n\
           Outcome:        {}\n\
           Token ID:       {}\n\
           Avg Bid (excl top): {:.4}\n\
           Other Bids:     {:?}\n\
         ════════════════════════════════════════════════════════════════",
        ctx.market_id,
        ctx.market_question,
        ctx.market_url,
        ctx.format_price_to_beat(),
        oracle_price,
        bps_diff,
        ctx.oracle_bps_price_threshold,
        outcome_name,
        token_id,
        avg_bid_price,
        other_bids
    );
}

/// Log when market has ended (all orderbooks empty)
pub fn log_market_ended(ctx: &MarketTrackerContext) {
    info!(
        "════════════════════════════════════════════════════════════════\n\
         🏁 MARKET ENDED - NO BIDS OR ASKS IN ANY ORDERBOOK\n\
         ════════════════════════════════════════════════════════════════\n\
           Market ID:    {}\n\
           Market:       {}\n\
           URL:          {}\n\
           Price to Beat:{}\n\
         ════════════════════════════════════════════════════════════════",
        ctx.market_id,
        ctx.market_question,
        ctx.market_url,
        ctx.format_price_to_beat()
    );
}
