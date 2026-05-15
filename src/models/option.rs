use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OptionType {
    Call,
    Put,
}

impl OptionType {
    pub fn label(&self) -> &'static str {
        match self {
            OptionType::Call => "Call",
            OptionType::Put => "Put",
        }
    }
}

/// Parameters describing a single option contract.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptionSpec {
    pub symbol: String,
    pub option_type: OptionType,
    pub strike: f64,
    pub expiry: NaiveDate,
}

impl OptionSpec {
    /// Time to expiry in years from today.
    pub fn years_to_expiry(&self) -> f64 {
        let today = chrono::Local::now().date_naive();
        let days = (self.expiry - today).num_days().max(0);
        days as f64 / 365.0
    }
}
