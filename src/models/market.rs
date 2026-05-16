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
