//! Option-combination pricing: net premium of a set of signed legs, plus
//! sensitivity sweeps (vs spot / vol / rate) for the Combos tab in the Pricer.
//!
//! A "combo" is N legs, each a signed contract count (+long / −short). The
//! tracked quantity is the net premium *difference* `Σ qty · price(leg)` (per
//! share — no contract multiplier). A roll, for example, is two legs; a
//! vertical is two same-expiry legs, etc.

use std::collections::BTreeMap;

use crate::models::option::OptionType;
use crate::pricing::black_scholes::BsInputs;

/// One leg of a combination.
#[derive(Clone, Debug, PartialEq)]
pub struct ComboLeg {
    pub option_type: OptionType,
    pub strike: f64,
    /// Time to expiry in years from "today".
    pub expiry_years: f64,
    /// Signed contract count (+long / −short).
    pub quantity: i32,
}

/// Per-share theoretical price of a single leg. Uses intrinsic value once
/// expiry has passed (`t <= 0`).
fn leg_price(leg: &ComboLeg, spot: f64, vol: f64, rate: f64, t: f64) -> f64 {
    if t <= 0.0 {
        match leg.option_type {
            OptionType::Call => (spot - leg.strike).max(0.0),
            OptionType::Put => (leg.strike - spot).max(0.0),
        }
    } else {
        BsInputs { spot, strike: leg.strike, expiry_years: t, vol, rate }.price(leg.option_type)
    }
}

/// Net premium of the combination at the given market state, per share (no
/// contract multiplier). Cash convention: a long leg (qty > 0) is a debit paid
/// (negative), a short leg (qty < 0) is a credit received (positive), so a
/// net-credit combo reads positive.
pub fn combo_value(legs: &[ComboLeg], spot: f64, vol: f64, rate: f64) -> f64 {
    legs.iter()
        .map(|l| -(l.quantity as f64) * leg_price(l, spot, vol, rate, l.expiry_years))
        .sum()
}

/// `n` evenly spaced points on `[lo, hi]` (inclusive). `n < 2` yields `[lo]`.
fn linspace(lo: f64, hi: f64, n: usize) -> Vec<f64> {
    if n < 2 {
        return vec![lo];
    }
    let step = (hi - lo) / (n - 1) as f64;
    (0..n).map(|i| lo + step * i as f64).collect()
}

/// Net combo value as spot sweeps `[lo, hi]`, vol and rate held fixed.
pub fn sweep_spot(legs: &[ComboLeg], vol: f64, rate: f64, lo: f64, hi: f64, n: usize) -> (Vec<f64>, Vec<f64>) {
    let xs = linspace(lo, hi, n);
    let ys = xs.iter().map(|&s| combo_value(legs, s, vol, rate)).collect();
    (xs, ys)
}

/// Net combo value as vol sweeps `[lo, hi]` (decimal, e.g. 0.1–0.8), spot and
/// rate held fixed.
pub fn sweep_vol(legs: &[ComboLeg], spot: f64, rate: f64, lo: f64, hi: f64, n: usize) -> (Vec<f64>, Vec<f64>) {
    let xs = linspace(lo, hi, n);
    let ys = xs.iter().map(|&v| combo_value(legs, spot, v, rate)).collect();
    (xs, ys)
}

/// Net combo value as the risk-free rate sweeps `[lo, hi]` (decimal), spot and
/// vol held fixed.
pub fn sweep_rate(legs: &[ComboLeg], spot: f64, vol: f64, lo: f64, hi: f64, n: usize) -> (Vec<f64>, Vec<f64>) {
    let xs = linspace(lo, hi, n);
    let ys = xs.iter().map(|&r| combo_value(legs, spot, vol, r)).collect();
    (xs, ys)
}

/// Net combo value as the evaluation date moves forward over `[0, horizon]`
/// years, reducing every leg's time to expiry together (expired legs settle to
/// intrinsic). Returns `(days_forward, value)` — the combo's decay curve.
pub fn sweep_time(legs: &[ComboLeg], spot: f64, vol: f64, rate: f64, horizon_years: f64, n: usize) -> (Vec<f64>, Vec<f64>) {
    let offsets = linspace(0.0, horizon_years, n);
    let xs = offsets.iter().map(|o| o * 365.0).collect();
    let ys = offsets.iter()
        .map(|&off| {
            legs.iter()
                .map(|l| -(l.quantity as f64) * leg_price(l, spot, vol, rate, (l.expiry_years - off).max(0.0)))
                .sum()
        })
        .collect();
    (xs, ys)
}

/// Combine per-leg historical series into a net combo series, aligned on
/// timestamp. Each input is `(quantity, [(timestamp, value)])`; only timestamps
/// present in *every* leg are kept. `value` is per-share (mid price or IV).
///
/// For the price series pass `weight_by_qty = true` → net premium in the cash
/// convention `Σ −qty · mid` (short legs positive). For an IV "vol diff" series
/// pass `false` → the signed sum `Σ −sign(qty) · iv`.
pub fn combine_series(legs: &[(i32, Vec<(String, f64)>)], weight_by_qty: bool) -> Vec<(String, f64)> {
    if legs.is_empty() {
        return vec![];
    }
    // Accumulate contributions and per-timestamp presence counts. Signs match
    // combo_value's cash convention (long = debit negative, short = credit positive).
    let mut acc: BTreeMap<String, (f64, usize)> = BTreeMap::new();
    for (qty, series) in legs {
        let weight = if weight_by_qty { -(*qty as f64) } else { -(qty.signum() as f64) };
        for (ts, v) in series {
            let e = acc.entry(ts.clone()).or_insert((0.0, 0));
            e.0 += weight * v;
            e.1 += 1;
        }
    }
    acc.into_iter()
        .filter(|(_, (_, count))| *count == legs.len())
        .map(|(ts, (net, _))| (ts, net))
        .collect()
}
