use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct Quote {
    pub symbol: String,
    pub price: f64,
    pub change: f64,
    pub change_pct: f64,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct OptionQuote {
    pub price: f64,
    pub implied_vol: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct OptionMetaEntry {
    pub expiry: String,
    pub option_type: String,  // "call" | "put"
    pub strike: f64,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ForwardVolResult {
    pub forward_vol: f64,
    pub atm_vol_t1: f64,
    pub atm_vol_t2: f64,
    pub t1_years: f64,
    pub t2_years: f64,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct LatestBar {
    pub symbol: String,
    pub bar_time: String,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
    pub trade_count: i64,
    pub vwap: f64,
    pub cached: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct OptionChainEntry {
    pub symbol: String,
    pub underlying: String,
    pub expiry: String,
    #[serde(rename = "type")]
    pub option_type: String,
    pub strike: f64,
    pub bid: f64,
    pub ask: f64,
    pub mid: f64,
    pub last: Option<f64>,
    pub implied_vol: Option<f64>,
    pub delta: Option<f64>,
    pub gamma: Option<f64>,
    pub theta: Option<f64>,
    pub vega: Option<f64>,
    pub open_interest: Option<f64>,
    pub volume: Option<f64>,
}
