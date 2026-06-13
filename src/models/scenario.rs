use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::option::{OptionSpec, OptionType};

// ── Market inputs ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ScenarioMarketInput {
    pub symbol: String,
    pub price: f64,
    pub vol: f64,   // decimal (0.25 = 25%)
    pub rate: f64,  // decimal (0.05 = 5%)
}

// ── Trades ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum TradeDirection {
    Buy,
    Sell,
}

impl TradeDirection {
    pub fn label(&self) -> &'static str {
        match self { TradeDirection::Buy => "Buy", TradeDirection::Sell => "Sell" }
    }
    pub fn sign(&self) -> f64 {
        match self { TradeDirection::Buy => 1.0, TradeDirection::Sell => -1.0 }
    }
}

/// One leg of a hypothetical trade within a scenario.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScenarioTrade {
    pub id: Uuid,
    pub label: String,
    pub symbol: String,
    pub direction: TradeDirection,
    pub contracts: u32,           // always positive; direction carries sign
    pub price: f64,               // per share / per-share premium
    pub option_spec: Option<OptionSpec>,

    // Populated when this trade closes an existing portfolio position.
    pub closes_position_id: Option<Uuid>,
    pub closes_cost_basis: Option<f64>,
    pub closes_is_long: Option<bool>,    // true if the original was long
    pub closes_opened_at: Option<NaiveDate>,
}

impl ScenarioTrade {
    pub fn multiplier(&self) -> f64 {
        if self.option_spec.is_some() { 100.0 } else { 1.0 }
    }

    /// Positive = cash received, negative = cash paid.
    pub fn cash_flow(&self) -> f64 {
        // Buy = pay out, Sell = receive
        -self.direction.sign() * self.price * self.contracts as f64 * self.multiplier()
    }

    /// Realized gain/loss if this closes an existing position. None for opening trades.
    pub fn realized_gain(&self) -> Option<f64> {
        let cb = self.closes_cost_basis?;
        let is_long = self.closes_is_long?;
        let qty = self.contracts as f64;
        let mult = self.multiplier();
        // Closing a long: sell at price, gain = price − cb
        // Closing a short: buy back at price, gain = cb − price
        let gain = if is_long { (self.price - cb) * qty * mult }
                   else       { (cb - self.price) * qty * mult };
        Some(gain)
    }

    pub fn is_long_term(&self, eval_date: NaiveDate) -> bool {
        if self.option_spec.is_some() {
            return false; // options are always short-term regardless of holding period
        }
        self.closes_opened_at
            .map(|d| (eval_date - d).num_days() > 365)
            .unwrap_or(false)
    }
}

// ── Result types ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeResult {
    pub label: String,
    pub cash_flow: f64,
    pub realized_gain: Option<f64>,
    pub is_long_term: bool,
}

/// An auto-detected assignment: short ITM option at/past expiry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssignmentEvent {
    pub description: String,
    /// Net P&L on the option itself (premium received − intrinsic loss).
    pub option_pnl: f64,
    pub is_long_term: bool,
    /// Cash from the resulting stock transaction (positive = received, negative = paid).
    pub stock_cash_flow: f64,
    pub option_type: OptionType,
    pub strike: f64,
    pub contracts: u32,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct ScenarioGreeks {
    pub delta: f64,
    pub gamma: f64,
    pub vega: f64,
    pub theta: f64,
    pub rho: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScenarioResult {
    pub evaluated_at: DateTime<Utc>,
    pub trade_results: Vec<TradeResult>,
    pub assignments: Vec<AssignmentEvent>,
    pub net_cash: f64,
    pub total_st_gain: f64,
    pub total_lt_gain: f64,
    pub greeks: ScenarioGreeks,
}

impl ScenarioResult {
    pub fn total_realized(&self) -> f64 {
        self.total_st_gain + self.total_lt_gain
    }
}

// ── Scenario ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Scenario {
    pub id: Uuid,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub evaluation_date: NaiveDate,
    #[serde(default)]
    pub market_inputs: Vec<ScenarioMarketInput>,
    #[serde(default)]
    pub trades: Vec<ScenarioTrade>,
    #[serde(default)]
    pub archived: bool,
}

impl Scenario {
    pub fn new(name: &str, evaluation_date: NaiveDate) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.to_owned(),
            created_at: Utc::now(),
            evaluation_date,
            market_inputs: vec![],
            trades: vec![],
            archived: false,
        }
    }

    pub fn price_for(&self, symbol: &str) -> Option<f64> {
        self.market_inputs.iter().find(|m| m.symbol == symbol).map(|m| m.price)
    }
}
