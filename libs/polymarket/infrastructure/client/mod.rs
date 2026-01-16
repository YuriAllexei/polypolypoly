//! Polymarket API clients
//!
//! Provides clients for both the Gamma API (market data) and CLOB API (trading).

pub mod auth;
pub mod binance;
pub mod clob;
pub mod ctf;
pub mod data;
pub mod gamma;
pub mod oracle;
pub mod redeem;
pub mod sports;
pub mod user;

pub use auth::PolymarketAuth;
pub use binance::{
    spawn_binance_tracker, BinanceAsset, BinancePriceEntry, BinancePriceManager,
    SharedBinancePrices,
};
pub use clob::{RestClient, WebSocketClient, Market, Outcome, OrderBook, PriceLevel, Side, OrderType, OrderArgs, TradingClient, TradingError};
pub use data::{DataApiClient, Position, PositionFilters, PositionSortBy, SortDirection};
pub use gamma::{GammaClient, GammaEvent, GammaMarket, GammaTag, GammaFilters};
pub use oracle::{spawn_oracle_trackers, OraclePriceManager, SharedOraclePrices, OracleType, PriceEntry, CandlestickApiClient};
pub use sports::{spawn_sports_live_data_tracker, SportsLiveData, SportsLiveDataMessage, SportsRoute};
// Note: OrderManager and related types moved to infrastructure::order_manager
pub use redeem::{
    RedeemClient, RedeemError, RedemptionResult,
    create_signer_provider, fetch_redeemable_positions,
    redeem_all_positions, redeem_single, redeem_all,
    redeem_via_safe,
    POLYGON_RPC_URL, POLYGON_CHAIN_ID,
};
pub use ctf::{
    CtfClient, CtfError, CtfOperation, CtfOperationResult,
    split_via_safe, merge_via_safe, approve_via_safe,
    split, merge,
    usdc_to_raw, usdc_from_raw,
    USDC_DECIMALS, CTF_CONTRACT, NEG_RISK_CTF_CONTRACT, USDC_ADDRESS,
};
