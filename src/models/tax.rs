use chrono::{DateTime, NaiveDate, Utc};
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

impl Default for FilingStatus {
    fn default() -> Self {
        FilingStatus::Single
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

impl Default for DeductionChoice {
    fn default() -> Self {
        DeductionChoice::Standard
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TaxEntryMode {
    #[default]
    Snapshot,
    LineItem,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LineItemCategory {
    W2,
    Interest,
    NonQualDiv,
    QualDiv,
    StGain,
    LtGain,
    Rental,
}

impl LineItemCategory {
    pub fn label(self) -> &'static str {
        match self {
            Self::W2 => "W-2 income",
            Self::Interest => "Interest income",
            Self::NonQualDiv => "Non-qual dividends",
            Self::QualDiv => "Qual dividends",
            Self::StGain => "Short-term gain",
            Self::LtGain => "Long-term gain",
            Self::Rental => "Rental income",
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::W2 => "w2",
            Self::Interest => "interest",
            Self::NonQualDiv => "non_qual_div",
            Self::QualDiv => "qual_div",
            Self::StGain => "st_gain",
            Self::LtGain => "lt_gain",
            Self::Rental => "rental",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "w2" => Some(Self::W2),
            "interest" => Some(Self::Interest),
            "non_qual_div" => Some(Self::NonQualDiv),
            "qual_div" => Some(Self::QualDiv),
            "st_gain" => Some(Self::StGain),
            "lt_gain" => Some(Self::LtGain),
            "rental" => Some(Self::Rental),
            _ => None,
        }
    }

    pub fn all() -> [LineItemCategory; 7] {
        [Self::W2, Self::Interest, Self::NonQualDiv, Self::QualDiv, Self::StGain, Self::LtGain, Self::Rental]
    }
}

impl Default for LineItemCategory {
    fn default() -> Self {
        LineItemCategory::W2
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaxLineItem {
    pub id: Uuid,
    pub entered_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<NaiveDate>,
    pub category: LineItemCategory,
    pub amount: f64,
    #[serde(default)]
    pub description: String,
}

/// Non-accumulating personal settings used in line-item mode.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaxSettings {
    pub filing_status: FilingStatus,
    pub deduction_choice: DeductionChoice,
    pub itemized_deductions: f64,
    pub carryforward_st_loss: f64,
    pub carryforward_lt_loss: f64,
}

impl Default for TaxSettings {
    fn default() -> Self {
        Self {
            filing_status: FilingStatus::Single,
            deduction_choice: DeductionChoice::Standard,
            itemized_deductions: 0.0,
            carryforward_st_loss: 0.0,
            carryforward_lt_loss: 0.0,
        }
    }
}

/// One immutable, timestamped snapshot of the user's tax profile for a year.
///
/// Field names must match the Worker's `TaxInputs` interface 1:1 so the JSON
/// deserializes directly. `qualified_dividends` is a subset of
/// `ordinary_dividends`; carryforward losses are positive numbers (a $5,000 loss
/// carried in is `5000.0`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
/// (user, tax_year). `revisions` is append-only in snapshot mode; in line-item
/// mode it always holds exactly one entry (the computed aggregate) so the Worker
/// always reads a consistent `revisions.last()`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaxProfile {
    pub id: Uuid,
    pub tax_year: u16,
    #[serde(default)]
    pub revisions: Vec<TaxRevision>,
    #[serde(default)]
    pub mode: TaxEntryMode,
    #[serde(default)]
    pub settings: TaxSettings,
    #[serde(default)]
    pub line_items: Vec<TaxLineItem>,
}

impl TaxProfile {
    pub fn new(tax_year: u16) -> Self {
        Self {
            id: Uuid::new_v4(),
            tax_year,
            revisions: vec![],
            mode: TaxEntryMode::Snapshot,
            settings: TaxSettings::default(),
            line_items: vec![],
        }
    }

    /// The current (most recent) revision, if any.
    pub fn current(&self) -> Option<&TaxRevision> {
        self.revisions.last()
    }

    /// Computes the effective tax inputs regardless of entry mode.
    /// In line-item mode this sums the line items and merges in personal settings.
    pub fn effective_revision(&self) -> TaxRevision {
        match self.mode {
            TaxEntryMode::Snapshot => self.revisions.last().cloned().unwrap_or_default(),
            TaxEntryMode::LineItem => {
                let s = &self.settings;
                let sum = |cat: LineItemCategory| -> f64 {
                    self.line_items.iter().filter(|i| i.category == cat).map(|i| i.amount).sum()
                };
                let qual = sum(LineItemCategory::QualDiv);
                let non_qual = sum(LineItemCategory::NonQualDiv);
                TaxRevision {
                    entered_at: Utc::now(),
                    filing_status: s.filing_status,
                    deduction_choice: s.deduction_choice,
                    itemized_deductions: s.itemized_deductions,
                    carryforward_st_loss: s.carryforward_st_loss,
                    carryforward_lt_loss: s.carryforward_lt_loss,
                    w2_income: sum(LineItemCategory::W2),
                    interest_income: sum(LineItemCategory::Interest),
                    ordinary_dividends: non_qual + qual,
                    qualified_dividends: qual,
                    st_capital_gains: sum(LineItemCategory::StGain),
                    lt_capital_gains: sum(LineItemCategory::LtGain),
                    rental_income: sum(LineItemCategory::Rental),
                }
            }
        }
    }

    /// In line-item mode, keeps `revisions` in sync with the computed aggregate
    /// so the Worker always reads the current effective inputs via `revisions.last()`.
    pub fn sync_revisions(&mut self) {
        if self.mode == TaxEntryMode::LineItem {
            self.revisions = vec![self.effective_revision()];
        }
    }
}
