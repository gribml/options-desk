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
