use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

use super::option::OptionSpec;

/// A hypothetical trade within a scenario (does not affect real portfolio).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HypotheticalTrade {
    pub symbol: String,
    pub quantity: i32,       // signed
    pub price: f64,          // execution price assumed
    pub option_spec: Option<OptionSpec>,
}

/// Assumptions about future prices on a given evaluation date.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriceAssumption {
    pub symbol: String,
    pub assumed_price: f64,
}

/// P&L breakdown for a single leg within a scenario result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LegResult {
    pub description: String,
    pub pnl: f64,
    /// Estimated short/long-term split (simplistic: held > 1 year = LT).
    pub short_term_gain: f64,
    pub long_term_gain: f64,
}

/// Full result of evaluating a scenario.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScenarioResult {
    pub evaluated_at: DateTime<Utc>,
    pub evaluation_date: NaiveDate,
    pub legs: Vec<LegResult>,
    pub total_pnl: f64,
    pub total_short_term: f64,
    pub total_long_term: f64,
}

impl ScenarioResult {
    pub fn total_tax_estimate(&self, st_rate: f64, lt_rate: f64) -> f64 {
        self.total_short_term * st_rate + self.total_long_term * lt_rate
    }
}

/// A saved scenario: a named set of hypothetical trades and price assumptions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Scenario {
    pub id: Uuid,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub evaluation_date: NaiveDate,
    pub hypothetical_trades: Vec<HypotheticalTrade>,
    pub price_assumptions: Vec<PriceAssumption>,
    /// Cached result — recomputed on demand if None.
    pub result: Option<ScenarioResult>,
}

impl Scenario {
    pub fn new(name: &str, evaluation_date: NaiveDate) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.to_owned(),
            created_at: Utc::now(),
            evaluation_date,
            hypothetical_trades: vec![],
            price_assumptions: vec![],
            result: None,
        }
    }

    /// Build a lookup map: symbol → assumed price.
    pub fn price_map(&self) -> HashMap<String, f64> {
        self.price_assumptions
            .iter()
            .map(|a| (a.symbol.clone(), a.assumed_price))
            .collect()
    }
}
