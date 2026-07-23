mod client;
mod model;

#[cfg(test)]
pub use client::MarketBackend;
pub use client::{MarketClient, MarketCommand};
pub use model::{
    MARKET_ORDERS_REFRESH_SECS, MarketOrder, MarketSnapshot, TradeSide, format_trade_line,
    format_whisper_line,
};
#[cfg(test)]
pub use model::{MarketItemDetail, MarketItemSummary};

#[cfg(test)]
mod tests;
