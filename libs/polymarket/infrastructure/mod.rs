//! Infrastructure Layer
//!
//! Contains implementations of external interfaces (database, API clients, etc.)
//! This layer depends on the domain layer but not on the application layer.

pub mod balance_manager;
pub mod client;
pub mod config;
pub mod database;
pub mod heartbeat;
pub mod logging;
pub mod order_manager;
pub mod position_manager;
pub mod shutdown;

// Re-export commonly used types from client
pub use client::{
    clob::{
        build_ws_client, decimal_places, handle_client_event, Market, MarketTrackerConfig,
        OrderArgs, OrderBook, OrderType, Outcome, PriceLevel, RestClient, SharedOrderbooks,
        SharedPrecisions, Side, SniperHandler, SniperMessage, SniperRoute, SniperRouter,
        TickSizeChangeEvent, WebSocketClient,
    },
    gamma::{GammaClient, GammaEvent, GammaFilters, GammaMarket, GammaTag},
    oracle::{
        spawn_oracle_trackers, OraclePriceManager, OracleType, PriceEntry, SharedOraclePrices,
    },
    binance::{
        spawn_binance_tracker, BinanceAsset, BinancePriceEntry, BinancePriceManager,
        SharedBinancePrices,
    },
    sports::{
        spawn_sports_live_data_tracker, spawn_sports_tracker_with_state, FetchedGames,
        FullTimeEvent, IgnoredGames, MarketsByGame, NewGameEvent, SharedSportsLiveData,
        SportsLiveData, SportsLiveDataMessage, SportsRoute,
    },
    // Note: user module types are now in order_manager module
    PolymarketAuth,
    ctf::{
        CtfClient, CtfError, CtfOperation, CtfOperationResult,
        split_via_safe, merge_via_safe, approve_via_safe,
        split, merge,
        usdc_to_raw, usdc_from_raw,
        USDC_DECIMALS,
    },
};

// Re-export database types
pub use database::{DatabaseError, MarketDatabase, Result};

// Re-export config types
pub use config::{BotConfig, EventsConfig, SniperConfig};

// Re-export infrastructure services
pub use balance_manager::BalanceManager;
pub use heartbeat::Heartbeat;
pub use logging::{init_tracing, init_tracing_with_level};
pub use order_manager::{
    AssetOrderBook, Fill, MakerOrderInfo, Order, OrderManager, OrderStateStore, OrderStatus,
    SharedOrderState, Side as OrderSide, TradeStatus,
};
pub use position_manager::PositionManager;
pub use shutdown::ShutdownManager;
