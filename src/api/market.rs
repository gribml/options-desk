use gloo_net::http::Request;

use crate::config::WORKER_URL;
use crate::models::market::{OptionChainEntry, OptionMetaEntry, OptionQuote, Quote};

// ── Live quotes ───────────────────────────────────────────────────────────────

pub async fn fetch_quote(token: &str, symbol: &str) -> Result<Quote, String> {
    let url = format!("{}/quote?symbol={}", worker_base(), symbol);
    let resp = Request::get(&url)
        .header("Authorization", &format!("Bearer {}", token))
        .send()
        .await
        .map_err(|e| e.to_string())?;

    if resp.ok() {
        resp.json::<Quote>().await.map_err(|e| e.to_string())
    } else {
        let body: serde_json::Value = resp.json().await.unwrap_or_default();
        Err(body["error"].as_str().unwrap_or("Quote fetch failed").to_string())
    }
}

/// Fetches quotes for all symbols, silently skipping any that fail.
pub async fn fetch_quotes(token: &str, symbols: &[String]) -> Vec<Quote> {
    let mut out = Vec::new();
    for sym in symbols {
        if let Ok(q) = fetch_quote(token, sym).await {
            out.push(q);
        }
    }
    out
}

// ── Option quotes ─────────────────────────────────────────────────────────────

pub async fn fetch_option_quote(
    token: &str,
    symbol: &str,
    expiry: &str,
    option_type: &str,
    strike: f64,
) -> Result<OptionQuote, String> {
    let url = format!(
        "{}/option-quote?symbol={}&expiry={}&type={}&strike={}",
        worker_base(), symbol, expiry, option_type, strike,
    );
    let resp = Request::get(&url)
        .header("Authorization", &format!("Bearer {}", token))
        .send()
        .await
        .map_err(|e| e.to_string())?;

    if resp.ok() {
        resp.json::<OptionQuote>().await.map_err(|e| e.to_string())
    } else {
        let body: serde_json::Value = resp.json().await.unwrap_or_default();
        Err(body["error"].as_str().unwrap_or("Option quote fetch failed").to_string())
    }
}

// ── Option chain ──────────────────────────────────────────────────────────────

pub async fn fetch_option_chain(token: &str, symbol: &str) -> Result<Vec<OptionChainEntry>, String> {
    let url = format!("{}/option-chain?symbol={}", worker_base(), symbol);
    let resp = Request::get(&url)
        .header("Authorization", &format!("Bearer {}", token))
        .send()
        .await
        .map_err(|e| e.to_string())?;

    if resp.ok() {
        resp.json::<Vec<OptionChainEntry>>().await.map_err(|e| e.to_string())
    } else {
        let body: serde_json::Value = resp.json().await.unwrap_or_default();
        Err(body["error"].as_str().unwrap_or("Option chain fetch failed").to_string())
    }
}

// ── Option chain metadata (for expiry/strike dropdowns) ──────────────────────

/// Returns distinct (expiry, option_type, strike) tuples from the latest snapshot.
pub async fn fetch_option_meta(token: &str, symbol: &str) -> Result<Vec<OptionMetaEntry>, String> {
    let url = format!("{}/option-meta?symbol={}", worker_base(), symbol);
    let resp = Request::get(&url)
        .header("Authorization", &format!("Bearer {}", token))
        .send()
        .await
        .map_err(|e| e.to_string())?;

    if resp.ok() {
        resp.json::<Vec<OptionMetaEntry>>().await.map_err(|e| e.to_string())
    } else {
        let body: serde_json::Value = resp.json().await.unwrap_or_default();
        Err(body["error"].as_str().unwrap_or("Option meta fetch failed").to_string())
    }
}

// ── Close prices (for realised vol) ──────────────────────────────────────────

/// Returns close prices in descending date order (most recent first).
pub async fn fetch_close_prices(token: &str, symbol: &str, limit: usize) -> Result<Vec<f64>, String> {
    let url = format!("{}/close-prices?symbol={}&limit={}", worker_base(), symbol, limit);
    let resp = Request::get(&url)
        .header("Authorization", &format!("Bearer {}", token))
        .send()
        .await
        .map_err(|e| e.to_string())?;

    if resp.ok() {
        resp.json::<Vec<f64>>().await.map_err(|e| e.to_string())
    } else {
        let body: serde_json::Value = resp.json().await.unwrap_or_default();
        Err(body["error"].as_str().unwrap_or("Close prices fetch failed").to_string())
    }
}

// ── Volatility estimation ─────────────────────────────────────────────────────

/// Computes annualised realised volatility from `lookback` daily log-returns.
/// `closes` must be in descending date order (most recent first).
pub fn realized_vol(closes: &[f64], lookback: usize) -> Option<f64> {
    if closes.len() < lookback + 1 {
        return None;
    }
    let window = &closes[..lookback + 1];
    let log_returns: Vec<f64> = window.windows(2).map(|w| (w[0] / w[1]).ln()).collect();
    let n = log_returns.len() as f64;
    let mean = log_returns.iter().sum::<f64>() / n;
    let var = log_returns.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / (n - 1.0);
    Some(var.sqrt() * 252_f64.sqrt())
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn worker_base() -> String {
    WORKER_URL.trim_end_matches('/').to_string()
}
