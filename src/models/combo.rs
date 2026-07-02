use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::option::OptionType;

/// One leg of a saved combination.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComboLegSpec {
    pub option_type: OptionType,
    pub strike: f64,
    pub expiry: String, // YYYY-MM-DD
    pub quantity: i32,  // signed contracts (+long / −short)
}

/// A tracked option combination, persisted to Supabase. Slider values (spot,
/// vol, rate) are intentionally not stored — they re-derive from live market
/// data when the combo's symbol loads.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Combo {
    pub id: Uuid,
    pub label: String,
    pub symbol: String,
    pub legs: Vec<ComboLegSpec>,
    #[serde(default)]
    pub vol_mode: bool,
}
