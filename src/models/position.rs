use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::option::OptionSpec;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PositionKind {
    Stock,
    Option,
}

/// A single position in the portfolio — stock lot or option contract.
///
/// `quantity` is signed: positive = long, negative = short.
/// For options, `quantity` is number of contracts (each covering 100 shares).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Position {
    pub id: Uuid,
    pub symbol: String,
    pub kind: PositionKind,
    /// Signed quantity: positive = long, negative = short.
    pub quantity: i32,
    /// Per-share cost basis (for options: per-share premium, not per-contract).
    pub cost_basis: f64,
    /// Present only when kind == Option.
    pub option_spec: Option<OptionSpec>,
    pub opened_at: DateTime<Utc>,
    pub notes: String,
}

impl Position {
    pub fn new_stock(symbol: &str, quantity: i32, cost_basis: f64) -> Self {
        Self {
            id: Uuid::new_v4(),
            symbol: symbol.to_uppercase(),
            kind: PositionKind::Stock,
            quantity,
            cost_basis,
            option_spec: None,
            opened_at: Utc::now(),
            notes: String::new(),
        }
    }

    pub fn new_option(symbol: &str, quantity: i32, cost_basis: f64, spec: OptionSpec) -> Self {
        Self {
            id: Uuid::new_v4(),
            symbol: symbol.to_uppercase(),
            kind: PositionKind::Option,
            quantity,
            cost_basis,
            option_spec: Some(spec),
            opened_at: Utc::now(),
            notes: String::new(),
        }
    }

    /// Total cost of the position (positive = paid, negative = received).
    /// For options, multiplied by 100 (contract multiplier).
    pub fn total_cost(&self) -> f64 {
        match self.kind {
            PositionKind::Stock => self.cost_basis * self.quantity as f64,
            PositionKind::Option => self.cost_basis * self.quantity as f64 * 100.0,
        }
    }
}
