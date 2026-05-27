use gloo_net::http::Request;
use serde::Deserialize;

use crate::config::{SUPABASE_ANON_KEY, SUPABASE_URL, WORKER_URL};
use crate::models::market::{OptionChainEntry, OptionQuote, Quote};

// ── Live quotes (via Cloudflare Worker → Polygon) ────────────────────────────

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

// ── Option quotes (via Cloudflare Worker → Polygon) ──────────────────────────

/// Fetches the live mid-price (bid+ask)/2 for a specific options contract.
/// `option_type` is `"call"` or `"put"`.
/// Returns Err if the contract is not found or if the provider plan doesn't
/// include options — callers should fall back to B-S in that case.
pub async fn fetch_option_quote(
    token: &str,
    symbol: &str,
    expiry: &str,       // YYYY-MM-DD
    option_type: &str,  // "call" | "put"
    strike: f64,
) -> Result<OptionQuote, String> {
    let url = format!(
        "{}/option-quote?symbol={}&expiry={}&type={}&strike={}",
        worker_base(),
        symbol,
        expiry,
        option_type,
        strike,
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

pub async fn fetch_option_chain(
    token: &str,
    symbol: &str,
) -> Result<Vec<OptionChainEntry>, String> {
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

// ── Historical data sync (via Cloudflare Worker → Alpaca → Supabase) ────────

/// Triggers the worker to fetch 2 years of daily bars for each symbol and
/// store them in the Supabase price_history table.
/// Returns the per-symbol result map from the worker (e.g. "ok (504 bars)").
pub async fn trigger_history_sync(
    token: &str,
    symbols: &[String],
) -> Result<serde_json::Value, String> {
    let url = format!("{}/history", worker_base());
    let body = serde_json::json!({ "symbols": symbols });
    let resp = Request::post(&url)
        .header("Authorization", &format!("Bearer {}", token))
        .header("Content-Type", "application/json")
        .json(&body)
        .map_err(|e| e.to_string())?
        .send()
        .await
        .map_err(|e| e.to_string())?;

    if resp.ok() {
        resp.json::<serde_json::Value>().await.map_err(|e| e.to_string())
    } else {
        Err(format!("History sync failed: {}", resp.status()))
    }
}

// ── Price history reads (direct from Supabase) ────────────────────────────────

#[derive(Deserialize)]
struct BarRow {
    close: f64,
}

/// Returns close prices in descending date order (most recent first).
pub async fn fetch_close_prices(
    token: &str,
    symbol: &str,
    limit: usize,
) -> Result<Vec<f64>, String> {
    let url = format!(
        "{}/rest/v1/price_history?symbol=eq.{}&select=close&order=date.desc&limit={}",
        supabase_rest_base(),
        symbol,
        limit,
    );
    let resp = Request::get(&url)
        .header("apikey", SUPABASE_ANON_KEY)
        .header("Authorization", &format!("Bearer {}", token))
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|e| e.to_string())?;

    if resp.ok() {
        let rows: Vec<BarRow> = resp.json().await.map_err(|e| e.to_string())?;
        Ok(rows.into_iter().map(|r| r.close).collect())
    } else {
        Err(format!("Fetch close prices failed: {}", resp.status()))
    }
}

// ── Volatility estimation ─────────────────────────────────────────────────────

/// Computes annualised realised volatility from `lookback` daily log-returns.
/// `closes` must be in descending date order (most recent first).
/// Returns None if there is insufficient data.
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

// ── URL helpers ───────────────────────────────────────────────────────────────

fn worker_base() -> String {
    WORKER_URL.trim_end_matches('/').to_string()
}

fn supabase_rest_base() -> String {
    SUPABASE_URL
        .trim_end_matches('/')
        .trim_end_matches("/rest/v1")
        .trim_end_matches('/')
        .to_string()
        + "/rest/v1"
}
