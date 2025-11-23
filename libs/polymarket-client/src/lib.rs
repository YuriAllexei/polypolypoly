pub mod auth;
pub mod types;
pub mod rest;
pub mod websocket;
pub mod gamma;
pub mod gamma_types;

pub use auth::{PolymarketAuth, AuthError};
pub use types::*;
pub use rest::RestClient;
pub use websocket::*;
pub use gamma::{GammaClient, GammaError};
pub use gamma_types::{GammaMarket, GammaEvent, GammaFilters, GammaTag};
