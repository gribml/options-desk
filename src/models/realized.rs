//! Capital gains realised by the trades in the portfolio's trade logs.
//!
//! A sell against a long lot (or a buy against a short one) closes it, and the
//! gain on that lot is taxable in the year it closed. This module turns every
//! position's trade log into a flat list of such closings, characterised the
//! way the return would characterise them, so the Taxes page can show them and
//! the Worker can add them to the year's baseline.

use std::collections::BTreeMap;

use chrono::{Datelike, NaiveDate};

use super::position::{Position, PositionKind};
use super::tax::TradeGains;

/// One closed lot, with enough context to list it on its own.
#[derive(Debug, Clone, PartialEq)]
pub struct RealizedLot {
    pub symbol: String,
    /// "Stock" or the contract, e.g. "Call $150 17-Jan-25".
    pub instrument: String,
    pub is_option: bool,
    pub open_date: NaiveDate,
    pub close_date: NaiveDate,
    pub quantity: i32,
    pub open_price: f64,
    pub close_price: f64,
    /// Dollar gain (negative for a loss); options × 100 per contract.
    pub gain: f64,
    pub is_long_term: bool,
}

impl RealizedLot {
    pub fn year(&self) -> i32 {
        self.close_date.year()
    }
}

/// Every closed lot across `positions`, FIFO-matched, oldest closing first.
///
/// Long-term status follows the holding period for stock. Options are always
/// short-term here — the same rule the portfolio's mark-to-market tax uses —
/// so the two never disagree about a contract.
pub fn realized_lots(positions: &[Position]) -> Vec<RealizedLot> {
    let mut out = Vec::new();
    for p in positions {
        if p.trades.is_empty() {
            continue;
        }
        let is_option = p.kind == PositionKind::Option;
        let instrument = match &p.option_spec {
            Some(s) => format!(
                "{} ${:.0} {}",
                s.option_type.label(),
                s.strike,
                s.expiry.format("%d-%b-%y")
            ),
            None => "Stock".to_string(),
        };
        let mult = if is_option { 100.0 } else { 1.0 };
        let (_, closed) = p.compute_lots();
        for c in closed {
            out.push(RealizedLot {
                symbol: p.symbol.clone(),
                instrument: instrument.clone(),
                is_option,
                open_date: c.open_date,
                close_date: c.close_date,
                quantity: c.quantity,
                open_price: c.open_price,
                close_price: c.close_price,
                gain: c.realized_pnl * mult,
                is_long_term: !is_option && c.is_long_term,
            });
        }
    }
    out.sort_by_key(|l| (l.close_date, l.symbol.clone()));
    out
}

/// Short/long-term totals of `lots`.
pub fn trade_gains(lots: &[RealizedLot]) -> TradeGains {
    let mut g = TradeGains::default();
    for l in lots {
        g.add(l);
    }
    g
}

/// Totals per closing year. Years with nothing closed are absent.
pub fn trade_gains_by_year(lots: &[RealizedLot]) -> BTreeMap<i32, TradeGains> {
    let mut out: BTreeMap<i32, TradeGains> = BTreeMap::new();
    for l in lots {
        out.entry(l.year()).or_default().add(l);
    }
    out
}

impl TradeGains {
    fn add(&mut self, l: &RealizedLot) {
        if l.is_long_term { self.lt += l.gain } else { self.st += l.gain }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::option::{OptionSpec, OptionType};
    use crate::models::position::{PositionEntryMode, Trade};
    use uuid::Uuid;

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    fn t(date: &str, quantity: i32, price: f64) -> Trade {
        Trade { id: Uuid::new_v4(), date: d(date), quantity, price }
    }

    fn logged(mut p: Position, trades: Vec<Trade>) -> Position {
        p.entry_mode = PositionEntryMode::TradeLog;
        p.trades = trades;
        p
    }

    #[test]
    fn stock_is_long_term_only_past_a_year_fifo() {
        // Two lots; the sell closes the older (long-term) one first, then part
        // of the newer (short-term) one.
        let p = logged(Position::new_stock("AAPL", 0, 0.0), vec![
            t("2024-01-10", 100, 100.0),
            t("2025-06-01", 100, 150.0),
            t("2025-09-01", -150, 200.0),
        ]);
        let lots = realized_lots(&[p]);
        assert_eq!(lots.len(), 2);
        assert!(lots[0].is_long_term);
        assert_eq!(lots[0].quantity, 100);
        assert_eq!(lots[0].gain, 10_000.0);
        assert!(!lots[1].is_long_term);
        assert_eq!(lots[1].quantity, 50);
        assert_eq!(lots[1].gain, 2_500.0);

        let by_year = trade_gains_by_year(&lots);
        assert_eq!(by_year[&2025], TradeGains { st: 2_500.0, lt: 10_000.0 });
    }

    #[test]
    fn options_are_always_short_term_and_scaled_by_100() {
        let spec = OptionSpec {
            symbol: "AAPL".into(),
            option_type: OptionType::Call,
            strike: 150.0,
            expiry: d("2026-01-16"),
        };
        // Held well over a year, yet still short-term.
        let p = logged(Position::new_option("AAPL", 0, 0.0, spec), vec![
            t("2024-01-10", 2, 5.0),
            t("2025-09-01", -2, 3.0),
        ]);
        let lots = realized_lots(&[p]);
        assert_eq!(lots.len(), 1);
        assert!(!lots[0].is_long_term);
        assert_eq!(lots[0].gain, -400.0);
        assert_eq!(trade_gains(&lots), TradeGains { st: -400.0, lt: 0.0 });
    }

    #[test]
    fn a_snapshot_converted_by_record_trade_keeps_its_holding_period() {
        let mut p = Position::new_stock("MSFT", 100, 50.0);
        p.opened_at = d("2023-03-01").and_hms_opt(0, 0, 0).unwrap().and_utc();
        p.record_trade(t("2025-09-01", -40, 80.0));
        let lots = realized_lots(&[p.clone()]);
        assert_eq!(lots.len(), 1);
        assert!(lots[0].is_long_term);
        assert_eq!(lots[0].gain, 1_200.0);
        assert_eq!(p.effective_quantity(), 60);
    }

    #[test]
    fn open_positions_realise_nothing() {
        let p = logged(Position::new_stock("AAPL", 0, 0.0), vec![t("2025-01-10", 100, 100.0)]);
        assert!(realized_lots(&[p]).is_empty());
        assert!(trade_gains_by_year(&[]).is_empty());
    }
}
