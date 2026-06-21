use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::option::OptionSpec;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PositionKind {
    Stock,
    Option,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PositionEntryMode {
    #[default]
    Snapshot,
    TradeLog,
}

/// Sale/exercise lot-matching method, consumed by [`match_trades`]. This is a
/// realization-time tax choice (which lots a sale closes), not a property of the
/// trade log — the portfolio ledger always shows FIFO. `MinTax` is reserved for
/// scenario modeling, where the user chooses how a sale or assignment is matched.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
#[allow(dead_code)] // MinTax is exercised by scenario sale-matching (WIP), not the ledger.
pub enum LotAllocation {
    #[default]
    Fifo,
    MinTax,
}

/// A single trade that contributes to a position.
/// `quantity` is signed: positive = buy/open-long, negative = sell/open-short.
/// `price` is per share (for options: per-share premium, not per-contract).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trade {
    pub id: Uuid,
    pub date: NaiveDate,
    pub quantity: i32,
    pub price: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OpenLot {
    pub trade_id: Uuid,
    pub date: NaiveDate,
    pub quantity: i32,
    pub price: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClosedLot {
    pub open_date: NaiveDate,
    pub close_date: NaiveDate,
    pub quantity: i32,
    pub open_price: f64,
    pub close_price: f64,
    pub realized_pnl: f64,
    pub is_long_term: bool,
}

/// A single position in the portfolio — stock lot or option contract.
///
/// `quantity` and `cost_basis` are used in Snapshot mode.
/// In TradeLog mode, use `effective_quantity()` and `effective_cost_basis()` instead.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Position {
    pub id: Uuid,
    pub symbol: String,
    pub kind: PositionKind,
    /// Snapshot mode only: signed quantity (positive = long, negative = short).
    pub quantity: i32,
    /// Snapshot mode only: per-share cost basis.
    pub cost_basis: f64,
    pub option_spec: Option<OptionSpec>,
    pub opened_at: DateTime<Utc>,
    pub notes: String,
    #[serde(default)]
    pub entry_mode: PositionEntryMode,
    #[serde(default)]
    pub trades: Vec<Trade>,
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
            entry_mode: PositionEntryMode::Snapshot,
            trades: vec![],
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
            entry_mode: PositionEntryMode::Snapshot,
            trades: vec![],
        }
    }

    /// Total cost of the position in Snapshot mode.
    pub fn total_cost(&self) -> f64 {
        match self.kind {
            PositionKind::Stock => self.cost_basis * self.quantity as f64,
            PositionKind::Option => self.cost_basis * self.quantity as f64 * 100.0,
        }
    }

    /// Net signed quantity — from trades in TradeLog mode, field in Snapshot mode.
    pub fn effective_quantity(&self) -> i32 {
        match self.entry_mode {
            PositionEntryMode::Snapshot => self.quantity,
            PositionEntryMode::TradeLog => self.trades.iter().map(|t| t.quantity).sum(),
        }
    }

    /// Weighted-average cost basis of open lots in TradeLog mode, field in Snapshot mode.
    pub fn effective_cost_basis(&self) -> f64 {
        match self.entry_mode {
            PositionEntryMode::Snapshot => self.cost_basis,
            PositionEntryMode::TradeLog => {
                let (open_lots, _) = self.compute_lots();
                weighted_avg_cost(&open_lots)
            }
        }
    }

    /// Date of the oldest open lot — used for LT/ST holding period determination.
    pub fn oldest_open_lot_date(&self) -> DateTime<Utc> {
        match self.entry_mode {
            PositionEntryMode::Snapshot => self.opened_at,
            PositionEntryMode::TradeLog => {
                let (open_lots, _) = self.compute_lots();
                open_lots.iter()
                    .map(|l| l.date)
                    .min()
                    .and_then(|d| d.and_hms_opt(0, 0, 0))
                    .map(|dt| dt.and_utc())
                    .unwrap_or(self.opened_at)
            }
        }
    }

    /// Open/closed lots for the portfolio ledger view. Uses FIFO — the standard
    /// convention for showing which lots remain open and their holding period.
    /// The choice of allocation method only matters when realizing sales/exercises
    /// for tax (handled in scenarios), not for recording or displaying the log.
    pub fn compute_lots(&self) -> (Vec<OpenLot>, Vec<ClosedLot>) {
        match_trades(&self.trades, LotAllocation::Fifo)
    }
}

fn weighted_avg_cost(open_lots: &[OpenLot]) -> f64 {
    let qty: i32 = open_lots.iter().map(|l| l.quantity.abs()).sum();
    if qty == 0 {
        return 0.0;
    }
    let cost: f64 = open_lots.iter().map(|l| l.price * l.quantity.abs() as f64).sum();
    cost / qty as f64
}

/// Matches buys and sells using the given allocation method.
///
/// Processes trades in chronological order. When a trade of opposite sign to
/// the current pool is encountered, it closes existing lots. FIFO closes the
/// oldest lot first; MinTax closes the lot that minimises the realised gain
/// (highest cost for longs, lowest cost for shorts).
pub fn match_trades(trades: &[Trade], allocation: LotAllocation) -> (Vec<OpenLot>, Vec<ClosedLot>) {
    let mut sorted: Vec<&Trade> = trades.iter().collect();
    sorted.sort_by_key(|t| t.date);

    // pool entries: (date, trade_id, remaining_quantity, price)
    let mut pool: Vec<(NaiveDate, Uuid, i32, f64)> = Vec::new();
    let mut closed: Vec<ClosedLot> = Vec::new();

    for trade in sorted {
        let mut remaining = trade.quantity;

        loop {
            if remaining == 0 {
                break;
            }

            // Find a lot of opposite sign to close.
            let close_idx = match allocation {
                LotAllocation::Fifo => pool
                    .iter()
                    .enumerate()
                    .filter(|(_, l)| l.2 != 0 && l.2.signum() != remaining.signum())
                    .min_by_key(|(_, l)| l.0)
                    .map(|(i, _)| i),

                LotAllocation::MinTax => pool
                    .iter()
                    .enumerate()
                    .filter(|(_, l)| l.2 != 0 && l.2.signum() != remaining.signum())
                    .max_by(|(_, a), (_, b)| {
                        if remaining < 0 {
                            // Closing longs: highest cost first → minimum gain realised.
                            a.3.partial_cmp(&b.3).unwrap_or(std::cmp::Ordering::Equal)
                        } else {
                            // Closing shorts: lowest cost first → minimum gain realised
                            // (pnl for shorts = open_price − close_price, so low open_price = low gain).
                            b.3.partial_cmp(&a.3).unwrap_or(std::cmp::Ordering::Equal)
                        }
                    })
                    .map(|(i, _)| i),
            };

            let Some(idx) = close_idx else { break };

            let lot = &pool[idx];
            let close_qty = remaining.abs().min(lot.2.abs());
            let lot_sign = lot.2.signum(); // +1 long, -1 short
            let pnl = (trade.price - lot.3) * lot_sign as f64 * close_qty as f64;
            let holding_days = (trade.date - lot.0).num_days();

            closed.push(ClosedLot {
                open_date: lot.0,
                close_date: trade.date,
                quantity: close_qty,
                open_price: lot.3,
                close_price: trade.price,
                realized_pnl: pnl,
                is_long_term: holding_days > 365,
            });

            pool[idx].2 += close_qty * remaining.signum(); // shrink the lot
            remaining += close_qty * lot_sign;              // consume from remaining

            if pool[idx].2 == 0 {
                pool.remove(idx);
            }
        }

        if remaining != 0 {
            pool.push((trade.date, trade.id, remaining, trade.price));
        }
    }

    let open_lots = pool
        .into_iter()
        .filter(|l| l.2 != 0)
        .map(|l| OpenLot { trade_id: l.1, date: l.0, quantity: l.2, price: l.3 })
        .collect();

    (open_lots, closed)
}
