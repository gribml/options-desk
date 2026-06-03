use gloo_net::http::Request;

use crate::config::WORKER_URL;
use crate::models::market::{ForwardVolResult, LatestBar, OptionChainEntry, OptionMetaEntry, OptionQuote, Quote};

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

// ── Latest bar (live Alpaca fetch, 15-min D1 cache) ───────────────────────────

pub async fn fetch_latest_bar(token: &str, symbol: &str) -> Result<LatestBar, String> {
    let url = format!("{}/latest-bar?symbol={}", worker_base(), symbol);
    let resp = Request::get(&url)
        .header("Authorization", &format!("Bearer {}", token))
        .send()
        .await
        .map_err(|e| e.to_string())?;

    if resp.ok() {
        resp.json::<LatestBar>().await.map_err(|e| e.to_string())
    } else {
        let body: serde_json::Value = resp.json().await.unwrap_or_default();
        Err(body["error"].as_str().unwrap_or("Latest bar fetch failed").to_string())
    }
}

pub async fn fetch_latest_bars(token: &str, symbols: &[String]) -> Vec<LatestBar> {
    let mut out = Vec::new();
    for sym in symbols {
        if let Ok(b) = fetch_latest_bar(token, sym).await {
            out.push(b);
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

// ── Forward volatility ────────────────────────────────────────────────────────

/// Returns forward vol from eval_date to expiry using the SABR variance curve.
/// Returns Err if the vol surface table has no data for the symbol yet.
pub async fn fetch_forward_vol(
    token: &str,
    symbol: &str,
    eval_date: &str,
    expiry: &str,
) -> Result<ForwardVolResult, String> {
    let url = format!(
        "{}/forward-vol?symbol={}&eval_date={}&expiry={}",
        worker_base(), symbol, eval_date, expiry,
    );
    let resp = Request::get(&url)
        .header("Authorization", &format!("Bearer {}", token))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if resp.ok() {
        resp.json::<ForwardVolResult>().await.map_err(|e| e.to_string())
    } else {
        let body: serde_json::Value = resp.json().await.unwrap_or_default();
        Err(body["error"].as_str().unwrap_or("Forward vol fetch failed").to_string())
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn worker_base() -> String {
    WORKER_URL.trim_end_matches('/').to_string()
}
