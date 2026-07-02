use gloo_net::http::Request;
use serde::{Deserialize, Serialize};

use crate::config::{SUPABASE_ANON_KEY, SUPABASE_URL};
use crate::models::{combo::Combo, position::Position, scenario::Scenario, tax::TaxProfile};

// ── Auth ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct AuthResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub user: AuthUser,
}

#[derive(Debug, Deserialize)]
pub struct AuthUser {
    pub id: String,
    pub email: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SupabaseError {
    pub message: String,
}

pub async fn refresh_session(refresh_token: &str) -> Result<AuthResponse, String> {
    let url = format!("{}/auth/v1/token?grant_type=refresh_token", base_url());
    let body = serde_json::json!({ "refresh_token": refresh_token });

    let resp = Request::post(&url)
        .header("apikey", SUPABASE_ANON_KEY)
        .header("Content-Type", "application/json")
        .json(&body)
        .map_err(|e| e.to_string())?
        .send()
        .await
        .map_err(|e| e.to_string())?;

    if resp.ok() {
        resp.json::<AuthResponse>().await.map_err(|e| e.to_string())
    } else {
        let err = resp
            .json::<SupabaseError>()
            .await
            .map(|e| e.message)
            .unwrap_or_else(|_| "Token refresh failed".to_string());
        Err(err)
    }
}

pub async fn login(email: &str, password: &str) -> Result<AuthResponse, String> {
    let url = format!("{}/auth/v1/token?grant_type=password", base_url());
    let body = serde_json::json!({ "email": email, "password": password });

    let resp = Request::post(&url)
        .header("apikey", SUPABASE_ANON_KEY)
        .header("Content-Type", "application/json")
        .json(&body)
        .map_err(|e| e.to_string())?
        .send()
        .await
        .map_err(|e| e.to_string())?;

    if resp.ok() {
        resp.json::<AuthResponse>().await.map_err(|e| e.to_string())
    } else {
        let err = resp
            .json::<SupabaseError>()
            .await
            .map(|e| e.message)
            .unwrap_or_else(|_| "Login failed".to_string());
        Err(err)
    }
}

// ── Generic REST helpers ──────────────────────────────────────────────────────

fn base_url() -> &'static str {
    SUPABASE_URL
        .trim_end_matches('/')
        .trim_end_matches("/rest/v1")
        .trim_end_matches('/')
}

fn rest_url(table: &str) -> String {
    format!("{}/rest/v1/{}", base_url(), table)
}

fn authed_get(url: &str, token: &str) -> gloo_net::http::RequestBuilder {
    Request::get(url)
        .header("apikey", SUPABASE_ANON_KEY)
        .header("Authorization", &format!("Bearer {}", token))
        .header("Accept", "application/json")
}

fn authed_post(url: &str, token: &str) -> gloo_net::http::RequestBuilder {
    Request::post(url)
        .header("apikey", SUPABASE_ANON_KEY)
        .header("Authorization", &format!("Bearer {}", token))
        .header("Content-Type", "application/json")
}

fn authed_delete(url: &str, token: &str) -> gloo_net::http::RequestBuilder {
    Request::delete(url)
        .header("apikey", SUPABASE_ANON_KEY)
        .header("Authorization", &format!("Bearer {}", token))
}

// ── Positions ─────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
struct DbPosition {
    id: String,
    user_id: String,
    payload: serde_json::Value, // full Position serialised as JSON
}

pub async fn fetch_positions(token: &str, user_id: &str) -> Result<Vec<Position>, String> {
    let url = format!(
        "{}?user_id=eq.{}&select=payload",
        rest_url("positions"),
        user_id
    );
    let resp = authed_get(&url, token)
        .send()
        .await
        .map_err(|e| e.to_string())?;

    if !resp.ok() {
        return Err(format!("Fetch positions failed: {}", resp.status()));
    }

    let rows: Vec<serde_json::Value> = resp.json().await.map_err(|e| e.to_string())?;
    rows.into_iter()
        .map(|r| {
            serde_json::from_value::<Position>(r["payload"].clone())
                .map_err(|e| e.to_string())
        })
        .collect()
}

pub async fn upsert_position(
    token: &str,
    user_id: &str,
    position: &Position,
) -> Result<(), String> {
    let body = serde_json::json!({
        "id": position.id.to_string(),
        "user_id": user_id,
        "payload": serde_json::to_value(position).map_err(|e| e.to_string())?,
    });

    let url = rest_url("positions");
    let resp = authed_post(&url, token)
        .header("Prefer", "resolution=merge-duplicates,return=minimal")
        .json(&body)
        .map_err(|e| e.to_string())?
        .send()
        .await
        .map_err(|e| e.to_string())?;

    if resp.ok() {
        Ok(())
    } else {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        Err(format!("Upsert position failed: {} — {}", status, body))
    }
}

pub async fn delete_position(token: &str, position_id: &str) -> Result<(), String> {
    let url = format!("{}?id=eq.{}", rest_url("positions"), position_id);
    let resp = authed_delete(&url, token)
        .send()
        .await
        .map_err(|e| e.to_string())?;

    if resp.ok() {
        Ok(())
    } else {
        Err(format!("Delete position failed: {}", resp.status()))
    }
}

// ── Scenarios ─────────────────────────────────────────────────────────────────

pub async fn fetch_scenarios(token: &str, user_id: &str) -> Result<Vec<Scenario>, String> {
    let url = format!(
        "{}?user_id=eq.{}&select=payload&order=created_at.desc",
        rest_url("scenarios"),
        user_id
    );
    let resp = authed_get(&url, token)
        .send()
        .await
        .map_err(|e| e.to_string())?;

    if !resp.ok() {
        return Err(format!("Fetch scenarios failed: {}", resp.status()));
    }

    let rows: Vec<serde_json::Value> = resp.json().await.map_err(|e| e.to_string())?;
    Ok(rows.into_iter()
        .filter_map(|r| serde_json::from_value::<Scenario>(r["payload"].clone()).ok())
        .collect())
}

pub async fn upsert_scenario(
    token: &str,
    user_id: &str,
    scenario: &Scenario,
) -> Result<(), String> {
    let body = serde_json::json!({
        "id": scenario.id.to_string(),
        "user_id": user_id,
        "payload": serde_json::to_value(scenario).map_err(|e| e.to_string())?,
    });

    let url = rest_url("scenarios");
    let resp = authed_post(&url, token)
        .header("Prefer", "resolution=merge-duplicates,return=minimal")
        .json(&body)
        .map_err(|e| e.to_string())?
        .send()
        .await
        .map_err(|e| e.to_string())?;

    if resp.ok() {
        Ok(())
    } else {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        Err(format!("Upsert scenario failed: {} — {}", status, body))
    }
}

pub async fn delete_scenario(token: &str, scenario_id: &str) -> Result<(), String> {
    let url = format!("{}?id=eq.{}", rest_url("scenarios"), scenario_id);
    let resp = authed_delete(&url, token)
        .send()
        .await
        .map_err(|e| e.to_string())?;

    if resp.ok() {
        Ok(())
    } else {
        Err(format!("Delete scenario failed: {}", resp.status()))
    }
}

// ── Combos ────────────────────────────────────────────────────────────────────

pub async fn fetch_combos(token: &str, user_id: &str) -> Result<Vec<Combo>, String> {
    let url = format!(
        "{}?user_id=eq.{}&select=payload&order=created_at.desc",
        rest_url("combos"),
        user_id
    );
    let resp = authed_get(&url, token)
        .send()
        .await
        .map_err(|e| e.to_string())?;

    if !resp.ok() {
        return Err(format!("Fetch combos failed: {}", resp.status()));
    }

    let rows: Vec<serde_json::Value> = resp.json().await.map_err(|e| e.to_string())?;
    Ok(rows.into_iter()
        .filter_map(|r| serde_json::from_value::<Combo>(r["payload"].clone()).ok())
        .collect())
}

pub async fn upsert_combo(token: &str, user_id: &str, combo: &Combo) -> Result<(), String> {
    let body = serde_json::json!({
        "id": combo.id.to_string(),
        "user_id": user_id,
        "payload": serde_json::to_value(combo).map_err(|e| e.to_string())?,
    });

    let url = rest_url("combos");
    let resp = authed_post(&url, token)
        .header("Prefer", "resolution=merge-duplicates,return=minimal")
        .json(&body)
        .map_err(|e| e.to_string())?
        .send()
        .await
        .map_err(|e| e.to_string())?;

    if resp.ok() {
        Ok(())
    } else {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        Err(format!("Upsert combo failed: {} — {}", status, body))
    }
}

pub async fn delete_combo(token: &str, combo_id: &str) -> Result<(), String> {
    let url = format!("{}?id=eq.{}", rest_url("combos"), combo_id);
    let resp = authed_delete(&url, token)
        .send()
        .await
        .map_err(|e| e.to_string())?;

    if resp.ok() {
        Ok(())
    } else {
        Err(format!("Delete combo failed: {}", resp.status()))
    }
}

// ── Tax profiles ──────────────────────────────────────────────────────────────

/// Fetch all of the user's tax profiles (one per year).
pub async fn fetch_tax_profiles(token: &str, user_id: &str) -> Result<Vec<TaxProfile>, String> {
    let url = format!(
        "{}?user_id=eq.{}&select=payload&order=tax_year.desc",
        rest_url("tax_profiles"),
        user_id
    );
    let resp = authed_get(&url, token)
        .send()
        .await
        .map_err(|e| e.to_string())?;

    if !resp.ok() {
        return Err(format!("Fetch tax profiles failed: {}", resp.status()));
    }

    let rows: Vec<serde_json::Value> = resp.json().await.map_err(|e| e.to_string())?;
    Ok(rows.into_iter()
        .filter_map(|r| serde_json::from_value::<TaxProfile>(r["payload"].clone()).ok())
        .collect())
}

/// Upsert a single year's tax profile. Resolves on the (user_id, tax_year)
/// unique constraint, so `tax_year` is sent as a top-level column.
pub async fn upsert_tax_profile(
    token: &str,
    user_id: &str,
    profile: &TaxProfile,
) -> Result<(), String> {
    let body = serde_json::json!({
        "id": profile.id.to_string(),
        "user_id": user_id,
        "tax_year": profile.tax_year,
        "payload": serde_json::to_value(profile).map_err(|e| e.to_string())?,
    });

    let url = format!("{}?on_conflict=user_id,tax_year", rest_url("tax_profiles"));
    let resp = authed_post(&url, token)
        .header("Prefer", "resolution=merge-duplicates,return=minimal")
        .json(&body)
        .map_err(|e| e.to_string())?
        .send()
        .await
        .map_err(|e| e.to_string())?;

    if resp.ok() {
        Ok(())
    } else {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        Err(format!("Upsert tax profile failed: {} — {}", status, body))
    }
}
