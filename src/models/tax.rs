use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilingStatus {
    Single,
    Mfj,
    Mfs,
    Hoh,
}

impl FilingStatus {
    pub fn label(&self) -> &'static str {
        match self {
            FilingStatus::Single => "Single",
            FilingStatus::Mfj => "Married filing jointly",
            FilingStatus::Mfs => "Married filing separately",
            FilingStatus::Hoh => "Head of household",
        }
    }

    /// Serde wire value — must match the Worker's `Filing` type and the
    /// `#[serde(rename_all = "snake_case")]` encoding.
    pub fn as_str(&self) -> &'static str {
        match self {
            FilingStatus::Single => "single",
            FilingStatus::Mfj => "mfj",
            FilingStatus::Mfs => "mfs",
            FilingStatus::Hoh => "hoh",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "single" => Some(FilingStatus::Single),
            "mfj" => Some(FilingStatus::Mfj),
            "mfs" => Some(FilingStatus::Mfs),
            "hoh" => Some(FilingStatus::Hoh),
            _ => None,
        }
    }

    pub fn all() -> [FilingStatus; 4] {
        [FilingStatus::Single, FilingStatus::Mfj, FilingStatus::Mfs, FilingStatus::Hoh]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeductionChoice {
    Standard,
    Itemized,
}

impl DeductionChoice {
    pub fn as_str(&self) -> &'static str {
        match self {
            DeductionChoice::Standard => "standard",
            DeductionChoice::Itemized => "itemized",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "standard" => Some(DeductionChoice::Standard),
            "itemized" => Some(DeductionChoice::Itemized),
            _ => None,
        }
    }
}

/// One immutable, timestamped snapshot of the user's tax profile for a year.
///
/// Field names must match the Worker's `TaxInputs` interface 1:1 so the JSON
/// deserializes directly. `qualified_dividends` is a subset of
/// `ordinary_dividends`; carryforward losses are positive numbers (a $5,000 loss
/// carried in is `5000.0`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaxRevision {
    pub entered_at: DateTime<Utc>,
    pub filing_status: FilingStatus,
    pub w2_income: f64,
    pub interest_income: f64,
    pub ordinary_dividends: f64,
    pub qualified_dividends: f64,
    pub st_capital_gains: f64,
    pub lt_capital_gains: f64,
    pub rental_income: f64,
    pub deduction_choice: DeductionChoice,
    pub itemized_deductions: f64,
    pub carryforward_st_loss: f64,
    pub carryforward_lt_loss: f64,
}

impl Default for TaxRevision {
    fn default() -> Self {
        Self {
            entered_at: Utc::now(),
            filing_status: FilingStatus::Single,
            w2_income: 0.0,
            interest_income: 0.0,
            ordinary_dividends: 0.0,
            qualified_dividends: 0.0,
            st_capital_gains: 0.0,
            lt_capital_gains: 0.0,
            rental_income: 0.0,
            deduction_choice: DeductionChoice::Standard,
            itemized_deductions: 0.0,
            carryforward_st_loss: 0.0,
            carryforward_lt_loss: 0.0,
        }
    }
}

/// A user's tax profile for a single year — stored as one Supabase row per
/// (user, tax_year). `revisions` is append-only; the last entry is current.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaxProfile {
    pub id: Uuid,
    pub tax_year: u16,
    #[serde(default)]
    pub revisions: Vec<TaxRevision>,
}

impl TaxProfile {
    pub fn new(tax_year: u16) -> Self {
        Self { id: Uuid::new_v4(), tax_year, revisions: vec![] }
    }

    /// The current (most recent) revision, if any.
    pub fn current(&self) -> Option<&TaxRevision> {
        self.revisions.last()
    }
}
