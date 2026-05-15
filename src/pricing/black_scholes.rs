use std::f64::consts::{PI, SQRT_2};

use crate::models::option::OptionType;

/// Abramowitz & Stegun approximation (max error ~1.5e-7).
fn erf(x: f64) -> f64 {
    let t = 1.0 / (1.0 + 0.3275911 * x.abs());
    let poly = t
        * (0.254_829_592
            + t * (-0.284_496_736
                + t * (1.421_413_741 + t * (-1.453_152_027 + t * 1.061_405_429))));
    let result = 1.0 - poly * (-x * x).exp();
    if x >= 0.0 { result } else { -result }
}

fn norm_cdf(x: f64) -> f64 {
    0.5 * (1.0 + erf(x / SQRT_2))
}

fn norm_pdf(x: f64) -> f64 {
    (-0.5 * x * x).exp() / (2.0 * PI).sqrt()
}

/// Inputs to the Black-Scholes model.
#[derive(Debug, Clone, Copy)]
pub struct BsInputs {
    pub spot: f64,
    pub strike: f64,
    pub expiry_years: f64,
    pub vol: f64,
    pub rate: f64,
}

/// Full output: price + first-order Greeks.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BsResult {
    pub price: f64,
    pub delta: f64,
    pub gamma: f64,
    /// Per 1-vol-point (÷100).
    pub vega: f64,
    /// Per calendar day.
    pub theta: f64,
    /// Per 1% rate move.
    pub rho: f64,
}

impl BsInputs {
    fn d1(&self) -> f64 {
        let BsInputs { spot, strike, expiry_years: t, vol, rate } = *self;
        ((spot / strike).ln() + (rate + 0.5 * vol * vol) * t) / (vol * t.sqrt())
    }

    pub fn price(&self, kind: OptionType) -> f64 {
        let BsInputs { spot, strike, expiry_years: t, vol, rate } = *self;
        let d1 = self.d1();
        let d2 = d1 - vol * t.sqrt();
        let df = (-rate * t).exp();
        match kind {
            OptionType::Call => spot * norm_cdf(d1) - strike * df * norm_cdf(d2),
            OptionType::Put => strike * df * norm_cdf(-d2) - spot * norm_cdf(-d1),
        }
    }

    pub fn greeks(&self, kind: OptionType) -> BsResult {
        let BsInputs { spot, strike, expiry_years: t, vol, rate } = *self;
        let d1 = self.d1();
        let d2 = d1 - vol * t.sqrt();
        let nd1 = norm_pdf(d1);
        let df = (-rate * t).exp();
        let sqrt_t = t.sqrt();

        let price = self.price(kind);

        let delta = match kind {
            OptionType::Call => norm_cdf(d1),
            OptionType::Put => norm_cdf(d1) - 1.0,
        };

        let gamma = nd1 / (spot * vol * sqrt_t);
        let vega = spot * nd1 * sqrt_t / 100.0;

        let theta_base = -(spot * nd1 * vol) / (2.0 * sqrt_t);
        let theta = match kind {
            OptionType::Call => (theta_base - rate * strike * df * norm_cdf(d2)) / 365.0,
            OptionType::Put => (theta_base + rate * strike * df * norm_cdf(-d2)) / 365.0,
        };

        let rho = match kind {
            OptionType::Call => strike * t * df * norm_cdf(d2) / 100.0,
            OptionType::Put => -strike * t * df * norm_cdf(-d2) / 100.0,
        };

        BsResult { price, delta, gamma, vega, theta, rho }
    }
}

/// Implied volatility via bisection (±0.0001 vol tolerance).
pub fn implied_vol(market_price: f64, inputs: BsInputs, kind: OptionType) -> Option<f64> {
    let mut lo = 0.001_f64;
    let mut hi = 5.0_f64;

    for _ in 0..100 {
        let mid = 0.5 * (lo + hi);
        let p = BsInputs { vol: mid, ..inputs }.price(kind);
        if (p - market_price).abs() < 1e-6 {
            return Some(mid);
        }
        if p < market_price { lo = mid; } else { hi = mid; }
        if hi - lo < 1e-4 {
            return Some(0.5 * (lo + hi));
        }
    }
    None
}
