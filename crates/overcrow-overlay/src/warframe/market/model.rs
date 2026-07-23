use crate::warframe::{
    model::{ERROR_MAX_CHARS, STRING_MAX_CHARS, bound_chars},
    sanitize::{sanitize_item_name, sanitize_player_name},
};

pub const MARKET_RESULTS_MAX: usize = 12;
pub const MARKET_ORDERS_SHOWN: usize = 6;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TradeSide {
    /// You are selling the item (chat template / whisper to a buyer).
    Sell,
    /// You are buying the item (chat template / whisper to a seller).
    Buy,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TraderPresence {
    Online,
    Ingame,
    Offline,
    Unknown,
}

impl TraderPresence {
    pub fn label(self) -> &'static str {
        match self {
            Self::Online => "online",
            Self::Ingame => "in game",
            Self::Offline => "offline",
            Self::Unknown => "?",
        }
    }

    pub fn rank(self) -> u8 {
        match self {
            Self::Ingame => 0,
            Self::Online => 1,
            Self::Unknown => 2,
            Self::Offline => 3,
        }
    }
}

/// How often to re-fetch orders for the selected item while the market widget is live.
pub const MARKET_ORDERS_REFRESH_SECS: u64 = 30;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MarketSnapshot {
    pub query: String,
    pub results: Vec<MarketItemSummary>,
    pub selected: Option<MarketItemDetail>,
    pub status: Option<String>,
    pub error: Option<String>,
    /// Unix seconds of the last successful order fetch for `selected` (0 = never).
    pub selected_fetched_at_secs: u64,
    /// Earliest Unix second when another automatic order refresh may be queued.
    pub next_refresh_at_secs: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarketItemSummary {
    pub name: String,
    pub slug: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarketOrder {
    pub side: TradeSide,
    pub platinum: u32,
    pub trader: String,
    pub presence: TraderPresence,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarketItemDetail {
    pub name: String,
    pub slug: String,
    pub lowest_sell: Option<u32>,
    pub highest_buy: Option<u32>,
    pub order_count: u32,
    /// Cheapest visible sell orders (people selling — whisper them to buy).
    pub top_sells: Vec<MarketOrder>,
    /// Highest visible buy orders (people buying — whisper them to sell).
    pub top_buys: Vec<MarketOrder>,
}

/// Public trade-chat template (not addressed to a specific player).
pub fn format_trade_line(side: TradeSide, name: &str, price: u32) -> String {
    let prefix = match side {
        TradeSide::Sell => "WTS",
        TradeSide::Buy => "WTB",
    };
    let name = sanitize_item_name(name);
    format!("{prefix} {name} {price}p")
}

/// In-game whisper to a listed trader.
///
/// - Whisper a **seller** when you want to buy (`TradeSide::Buy`).
/// - Whisper a **buyer** when you want to sell (`TradeSide::Sell`).
pub fn format_whisper_line(side: TradeSide, trader: &str, item: &str, price: u32) -> String {
    let intent = match side {
        TradeSide::Buy => "WTB",
        TradeSide::Sell => "WTS",
    };
    let trader = sanitize_player_name(trader);
    let item = sanitize_item_name(item);
    let trader = if trader.is_empty() {
        "Tenno".to_owned()
    } else {
        trader
    };
    format!("/w {trader} Hi, {intent} {item} for {price}p")
}

pub fn bound_error(message: &str) -> String {
    bound_chars(message, ERROR_MAX_CHARS)
}

#[allow(dead_code)]
pub fn bound_query(query: &str) -> String {
    bound_chars(query.trim(), STRING_MAX_CHARS)
}
