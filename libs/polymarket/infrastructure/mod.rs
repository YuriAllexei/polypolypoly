//! Infrastructure Layer
//!
//! Contains implementations of external interfaces (database, API clients, etc.)
//! This layer depends on the domain layer but not on the application layer.

pub mod client;
pub mod config;
pub mod database;
pub mod heartbeat;
pub mod logging;
pub mod shutdown;

// Re-export commonly used types from client
pub use client::{
    clob::{
        build_ws_client, handle_client_event, Market, MarketTrackerConfig, OrderArgs, OrderBook,
        OrderType, Outcome, PriceLevel, RestClient, SharedOrderbooks, SharedPrecisions, Side,
        SniperHandler, SniperRoute, SniperRouter, TickSizeChangeEvent, WebSocketClient,
    },
    gamma::{GammaClient, GammaEvent, GammaFilters, GammaMarket, GammaTag},
    oracle::{
        spawn_oracle_trackers, OraclePriceManager, OracleType, PriceEntry, SharedOraclePrices,
    },
    sports::{
        spawn_sports_live_data_tracker, spawn_sports_tracker_with_state, FetchedGames,
        FullTimeEvent, IgnoredGames, MarketsByGame, NewGameEvent, SharedSportsLiveData,
        SportsLiveData, SportsLiveDataMessage, SportsRoute,
    },
    user::{
        spawn_user_order_tracker, OrderManager, OrderState, OrderStatus, SharedOrderManager,
        TradeState,
    },
    PolymarketAuth,
};

// Re-export database types
pub use database::{DatabaseError, MarketDatabase, Result};

// Re-export config types
pub use config::{BotConfig, EventsConfig, SniperConfig};

// Re-export infrastructure services
pub use heartbeat::Heartbeat;
pub use logging::{init_tracing, init_tracing_with_level};
pub use shutdown::ShutdownManager;
