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

/// Distinct expiries in `meta` that haven't passed, sorted ascending.
///
/// The `option_chain` table holds dated snapshots and is never pruned, so it
/// still carries contracts that expired months ago. Those must not reach a
/// picker: you can't open a position in a dead contract, and offering one
/// invites a scenario built on an instrument that no longer exists. Today
/// itself counts as live — an option is tradeable right up to expiry.
/// Distinct strikes for one expiry and option type, ascending.
///
/// `option_chain` is keyed by `(snapshot_time, symbol)`, so the same contract
/// appears once per snapshot the pipeline has taken. Without deduping, a picker
/// lists the same strike over and over.
pub fn live_strikes(meta: &[OptionMetaEntry], expiry: &str, option_type: &str) -> Vec<f64> {
    let mut v: Vec<f64> = meta
        .iter()
        .filter(|e| e.expiry == expiry && e.option_type == option_type)
        .map(|e| e.strike)
        .collect();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    v.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
    v
}

pub fn live_expiries(meta: &[OptionMetaEntry], today: chrono::NaiveDate) -> Vec<String> {
    let mut v: Vec<String> = meta
        .iter()
        .filter(|e| {
            chrono::NaiveDate::parse_from_str(&e.expiry, "%Y-%m-%d")
                .map(|d| d >= today)
                .unwrap_or(false)
        })
        .map(|e| e.expiry.clone())
        .collect();
    v.sort();
    v.dedup();
    v
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
    #[serde(default)]
    pub last: Option<f64>,
    pub implied_vol: Option<f64>,
    pub delta: Option<f64>,
    pub gamma: Option<f64>,
    pub theta: Option<f64>,
    pub vega: Option<f64>,
    pub open_interest: Option<f64>,
    pub volume: Option<f64>,
}

/// One point in a contract's historical time series (`/option-history`).
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct OptionHistoryPoint {
    pub t: String, // snapshot date, YYYY-MM-DD
    pub mid: Option<f64>,
    pub implied_vol: Option<f64>,
}

/// One page of an on-demand option-chain fetch (`/option-chain-live`). The
/// frontend loops while `next_page_token` is `Some`, merging each page into the
/// expiry/strike dropdowns as it arrives.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct OptionChainPage {
    pub entries: Vec<OptionChainEntry>,
    pub next_page_token: Option<String>,
    #[serde(default)]
    pub cached: bool,
}
