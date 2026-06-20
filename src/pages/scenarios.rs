use std::rc::Rc;
use std::sync::Arc;

use chrono::{Datelike, NaiveDate, Utc};
use leptos::prelude::*;
use uuid::Uuid;
use wasm_bindgen_futures::spawn_local;

use crate::api::{market, supabase};
use crate::app::AuthState;
use crate::models::market::{OptionChainEntry, OptionMetaEntry};
use crate::store::MarketStore;
use crate::models::{
    option::{OptionSpec, OptionType},
    position::Position,
    scenario::{
        AssignmentEvent, CoverageSummary, Scenario, ScenarioGreeks, ScenarioMarketInput,
        ScenarioResult, ScenarioTrade, TradeDirection, TradeResult,
    },
};
use crate::pricing::black_scholes::BsInputs;

// ── Evaluation ────────────────────────────────────────────────────────────────

/// Allocates `shares` against short call tiers (lowest strike first) and returns
/// the total capped upside: Σ min(remaining_shares, tier_shares) × strike.
fn covered_upside(shares: i32, mut call_tiers: Vec<(f64, i32)>) -> f64 {
    call_tiers.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    let mut remaining = shares;
    let mut upside = 0.0f64;
    for (strike, contracts) in call_tiers {
        if remaining <= 0 { break; }
        let assigned = remaining.min(contracts * 100);
        upside += assigned as f64 * strike;
        remaining -= assigned;
    }
    upside
}

fn evaluate(scenario: &Scenario, positions: &[Position]) -> ScenarioResult {
    let eval_date = scenario.evaluation_date;
    let mut trade_results: Vec<TradeResult> = vec![];
    let mut assignments: Vec<AssignmentEvent> = vec![];
    let mut net_cash = 0.0f64;
    let mut st_gain = 0.0f64;
    let mut lt_gain = 0.0f64;

    for t in &scenario.trades {
        let cf = t.cash_flow();
        let rg = t.realized_gain();
        let lt = t.is_long_term(eval_date);
        net_cash += cf;
        if let Some(g) = rg {
            if lt { lt_gain += g; } else { st_gain += g; }
        }
        trade_results.push(TradeResult { label: t.label.clone(), cash_flow: cf, realized_gain: rg, is_long_term: lt });
    }

    for pos in positions {
        let spec = match &pos.option_spec { Some(s) => s, None => continue };
        let pos_qty = pos.effective_quantity();
        if pos_qty >= 0 { continue; }
        if eval_date < spec.expiry { continue; }
        let price = match scenario.price_for(&pos.symbol) { Some(p) => p, None => continue };
        let intrinsic = match spec.option_type {
            OptionType::Call => price - spec.strike,
            OptionType::Put  => spec.strike - price,
        };
        if intrinsic <= 0.0 { continue; }
        let contracts = pos_qty.unsigned_abs();
        let stock_cf = match spec.option_type {
            OptionType::Call =>  spec.strike * contracts as f64 * 100.0,
            OptionType::Put  => -spec.strike * contracts as f64 * 100.0,
        };

        // For a covered call, IRS requires the premium received to be folded into
        // the stock sale proceeds: gain = (strike + premium) − stock_basis.
        // For everything else (naked, puts) fall back to option-only P&L.
        let underlying = positions.iter().find(|p| {
            p.option_spec.is_none() && p.symbol == pos.symbol && p.effective_quantity() > 0
        });
        let (realized_gain, lt) = match (spec.option_type, underlying) {
            (OptionType::Call, Some(stock)) => {
                let adjusted_basis = stock.effective_cost_basis() - pos.effective_cost_basis();
                let gain = (spec.strike - adjusted_basis) * contracts as f64 * 100.0;
                let lt = (eval_date - stock.oldest_open_lot_date().date_naive()).num_days() > 365;
                (gain, lt)
            }
            _ => {
                let gain = (pos.effective_cost_basis() - intrinsic) * contracts as f64 * 100.0;
                let lt = (eval_date - pos.oldest_open_lot_date().date_naive()).num_days() > 365;
                (gain, lt)
            }
        };

        net_cash += stock_cf;
        if lt { lt_gain += realized_gain; } else { st_gain += realized_gain; }
        assignments.push(AssignmentEvent {
            description: format!("Assignment: {} short {} ${:.0} {} ×{}",
                pos.symbol, spec.option_type.label(), spec.strike,
                spec.expiry.format("%d-%b-%y"), contracts),
            realized_gain, is_long_term: lt, stock_cash_flow: stock_cf,
            option_type: spec.option_type, strike: spec.strike, contracts,
        });
    }

    // Compute option Greeks given a signed quantity and market inputs.
    let option_greeks = |qty: f64, spec: &OptionSpec, mi: &ScenarioMarketInput| -> ScenarioGreeks {
        let t = (spec.expiry - eval_date).num_days().max(0) as f64 / 365.0;
        if t <= 0.0 { return ScenarioGreeks::default(); }
        let r = BsInputs { spot: mi.price, strike: spec.strike, expiry_years: t, vol: mi.vol, rate: mi.rate }
            .greeks(spec.option_type);
        let m = qty * 100.0;
        ScenarioGreeks { delta: r.delta*m, gamma: r.gamma*m, vega: r.vega*m, theta: r.theta*m, rho: r.rho*m }
    };
    let add = |acc: &mut ScenarioGreeks, g: ScenarioGreeks| {
        acc.delta += g.delta; acc.gamma += g.gamma;
        acc.vega  += g.vega;  acc.theta += g.theta; acc.rho += g.rho;
    };
    let sub = |acc: &mut ScenarioGreeks, g: ScenarioGreeks| {
        acc.delta -= g.delta; acc.gamma -= g.gamma;
        acc.vega  -= g.vega;  acc.theta -= g.theta; acc.rho -= g.rho;
    };

    let mut greeks = ScenarioGreeks::default();

    // 1. Existing portfolio.
    for pos in positions {
        let qty = pos.quantity as f64;
        match &pos.option_spec {
            None => { greeks.delta += qty; }
            Some(spec) => {
                if let Some(mi) = scenario.market_inputs.iter().find(|m| m.symbol == pos.symbol) {
                    add(&mut greeks, option_greeks(qty, spec, mi));
                }
            }
        }
    }

    // 2. Subtract Greeks for positions being closed by trades.
    for trade in &scenario.trades {
        if let Some(pos_id) = trade.closes_position_id {
            if let Some(pos) = positions.iter().find(|p| p.id == pos_id) {
                let frac = trade.contracts as f64 / pos.quantity.unsigned_abs().max(1) as f64;
                let qty = pos.quantity as f64 * frac;
                match &pos.option_spec {
                    None => { greeks.delta -= qty; }
                    Some(spec) => {
                        if let Some(mi) = scenario.market_inputs.iter().find(|m| m.symbol == pos.symbol) {
                            sub(&mut greeks, option_greeks(qty, spec, mi));
                        }
                    }
                }
            }
        }
    }

    // 3. Add Greeks for new positions opened by trades (no closes_position_id).
    for trade in &scenario.trades {
        if trade.closes_position_id.is_some() { continue; }
        let qty = match trade.direction {
            TradeDirection::Buy  =>  trade.contracts as f64,
            TradeDirection::Sell => -(trade.contracts as f64),
        };
        match &trade.option_spec {
            None => { greeks.delta += qty; }
            Some(spec) => {
                if let Some(mi) = scenario.market_inputs.iter().find(|m| m.symbol == trade.symbol) {
                    add(&mut greeks, option_greeks(qty, spec, mi));
                }
            }
        }
    }

    // ── Per-symbol coverage analysis ─────────────────────────────────────────
    let mut coverage: Vec<CoverageSummary> = vec![];
    let mut stock_syms: Vec<&str> = positions.iter()
        .filter(|p| p.option_spec.is_none() && p.quantity > 0)
        .map(|p| p.symbol.as_str())
        .collect();
    stock_syms.sort_unstable();
    stock_syms.dedup();

    for sym in stock_syms {
        let net_shares: i32 = positions.iter()
            .filter(|p| p.option_spec.is_none() && p.symbol == sym && p.quantity > 0)
            .map(|p| p.quantity)
            .sum::<i32>()
            + scenario.trades.iter()
                .filter(|t| t.option_spec.is_none() && t.symbol == sym)
                .map(|t| match t.direction {
                    TradeDirection::Buy  =>  t.contracts as i32,
                    TradeDirection::Sell => -(t.contracts as i32),
                })
                .sum::<i32>();

        let portfolio_short_calls: i32 = positions.iter()
            .filter(|p| p.symbol == sym && p.quantity < 0
                && p.option_spec.as_ref().map(|s| s.option_type == OptionType::Call).unwrap_or(false))
            .map(|p| (-p.quantity) as i32)
            .sum();

        let calls_closed: i32 = scenario.trades.iter()
            .filter(|t| t.symbol == sym)
            .filter(|t| {
                t.closes_position_id
                    .and_then(|pid| positions.iter().find(|p| p.id == pid))
                    .and_then(|p| p.option_spec.as_ref())
                    .map(|s| s.option_type == OptionType::Call)
                    .unwrap_or(false)
            })
            .map(|t| t.contracts as i32)
            .sum();

        let new_short_calls: i32 = scenario.trades.iter()
            .filter(|t| t.symbol == sym && t.closes_position_id.is_none()
                && t.direction == TradeDirection::Sell
                && t.option_spec.as_ref().map(|s| s.option_type == OptionType::Call).unwrap_or(false))
            .map(|t| t.contracts as i32)
            .sum();

        let net_short_call_contracts = portfolio_short_calls - calls_closed + new_short_calls;

        // Build per-strike call tiers for the portfolio (before scenario).
        let before_calls: Vec<(f64, i32)> = {
            let mut tiers: Vec<(f64, i32)> = vec![];
            for p in positions.iter().filter(|p| p.symbol == sym && p.quantity < 0) {
                if let Some(s) = p.option_spec.as_ref() {
                    if s.option_type == OptionType::Call {
                        if let Some(e) = tiers.iter_mut().find(|(k, _)| (*k - s.strike).abs() < 0.001) {
                            e.1 += (-p.quantity) as i32;
                        } else {
                            tiers.push((s.strike, (-p.quantity) as i32));
                        }
                    }
                }
            }
            tiers
        };

        // Build after-scenario tiers: start from before, subtract closed, add new.
        let after_calls: Vec<(f64, i32)> = {
            let mut tiers = before_calls.clone();
            // Remove contracts closed by the scenario.
            for t in scenario.trades.iter().filter(|t| t.symbol == sym) {
                if let Some(pid) = t.closes_position_id {
                    if let Some(pos) = positions.iter().find(|p| p.id == pid) {
                        if let Some(s) = pos.option_spec.as_ref() {
                            if s.option_type == OptionType::Call && pos.quantity < 0 {
                                if let Some(e) = tiers.iter_mut().find(|(k, _)| (*k - s.strike).abs() < 0.001) {
                                    e.1 -= t.contracts as i32;
                                }
                            }
                        }
                    }
                }
            }
            // Add new short calls opened by the scenario.
            for t in scenario.trades.iter()
                .filter(|t| t.symbol == sym && t.closes_position_id.is_none() && t.direction == TradeDirection::Sell)
            {
                if let Some(s) = t.option_spec.as_ref() {
                    if s.option_type == OptionType::Call {
                        if let Some(e) = tiers.iter_mut().find(|(k, _)| (*k - s.strike).abs() < 0.001) {
                            e.1 += t.contracts as i32;
                        } else {
                            tiers.push((s.strike, t.contracts as i32));
                        }
                    }
                }
            }
            tiers.retain(|(_, n)| *n > 0);
            tiers
        };

        // Portfolio shares before any scenario stock trades (for upside_before).
        let portfolio_shares: i32 = positions.iter()
            .filter(|p| p.option_spec.is_none() && p.symbol == sym && p.quantity > 0)
            .map(|p| p.quantity)
            .sum();

        let upside_before = covered_upside(portfolio_shares, before_calls);
        let upside_after  = covered_upside(net_shares, after_calls);

        coverage.push(CoverageSummary { symbol: sym.to_string(), net_shares, net_short_call_contracts, upside_before, upside_after });
    }

    // Tax is no longer subtracted here — it's computed per-card by the Worker
    // against the user's income profile. `net_cash` is pre-tax.
    ScenarioResult { evaluated_at: Utc::now(), trade_results, assignments, net_cash,
        total_st_gain: st_gain, total_lt_gain: lt_gain, greeks, coverage }
}

// ── B-S price helper ──────────────────────────────────────────────────────────

fn bs_price(spec: &OptionSpec, mi: &ScenarioMarketInput, eval_date: NaiveDate) -> Option<f64> {
    let t = (spec.expiry - eval_date).num_days().max(0) as f64 / 365.0;
    if t <= 0.0 {
        return Some(match spec.option_type {
            OptionType::Call => (mi.price - spec.strike).max(0.0),
            OptionType::Put  => (spec.strike - mi.price).max(0.0),
        });
    }
    Some(BsInputs { spot: mi.price, strike: spec.strike, expiry_years: t, vol: mi.vol, rate: mi.rate }
        .price(spec.option_type))
}

// Derives ATM implied vol from the nearest-expiry straddle in the option chain.
// Returns the average of the ATM call and put IVs (or whichever is available).
fn atm_iv_from_chain(chain: &[OptionChainEntry], spot: f64) -> Option<f64> {
    let today = Utc::now().date_naive();
    let nearest_expiry = chain.iter()
        .filter_map(|e| NaiveDate::parse_from_str(&e.expiry, "%Y-%m-%d").ok())
        .filter(|&d| d > today)
        .min()?;
    let exp_str = nearest_expiry.format("%Y-%m-%d").to_string();

    let best_call_iv = chain.iter()
        .filter(|e| e.expiry == exp_str && e.option_type == "call" && e.implied_vol.is_some())
        .min_by(|a, b| (a.strike - spot).abs().partial_cmp(&(b.strike - spot).abs())
            .unwrap_or(std::cmp::Ordering::Equal))
        .and_then(|e| e.implied_vol);

    let best_put_iv = chain.iter()
        .filter(|e| e.expiry == exp_str && e.option_type == "put" && e.implied_vol.is_some())
        .min_by(|a, b| (a.strike - spot).abs().partial_cmp(&(b.strike - spot).abs())
            .unwrap_or(std::cmp::Ordering::Equal))
        .and_then(|e| e.implied_vol);

    match (best_call_iv, best_put_iv) {
        (Some(c), Some(p)) => Some((c + p) / 2.0),
        (Some(c), None)    => Some(c),
        (None,    Some(p)) => Some(p),
        (None,    None)    => None,
    }
}

// ── Page ─────────────────────────────────────────────────────────────────────

#[component]
pub fn ScenariosPage() -> impl IntoView {
    let auth = use_context::<AuthState>().expect("AuthState missing");
    let scenarios  = RwSignal::new(Vec::<Scenario>::new());
    let positions  = RwSignal::new(Vec::<Position>::new());
    let loading    = RwSignal::new(true);
    let fetch_err  = RwSignal::new(Option::<String>::None);
    let show_new   = RwSignal::new(false);
    let show_archived = RwSignal::new(false);
    let editing_id = RwSignal::new(Option::<Uuid>::None);

    Effect::new(move |_| {
        let token = auth.token.get();
        let user_id = auth.user_id.get();
        if let (Some(tok), Some(uid)) = (token, user_id) {
            let tok2 = tok.clone(); let uid2 = uid.clone();
            fetch_err.set(None);
            spawn_local(async move {
                match supabase::fetch_scenarios(&tok, &uid).await {
                    Ok(s)  => scenarios.set(s),
                    Err(e) => fetch_err.set(Some(e)),
                }
                if let Ok(p) = supabase::fetch_positions(&tok2, &uid2).await { positions.set(p); }
                loading.set(false);
            });
        } else { loading.set(false); }
    });

    view! {
        <div class="space-y-6">
            <div class="flex items-center justify-between">
                <h1 class="text-xl font-semibold">"Scenarios"</h1>
                <button
                    class="bg-blue-600 hover:bg-blue-500 px-4 py-2 rounded text-sm font-medium transition-colors"
                    on:click=move |_| {
                        editing_id.set(None);
                        show_new.update(|v| *v = !*v);
                    }
                >
                    {move || if show_new.get() || editing_id.get().is_some() { "Cancel" } else { "+ New scenario" }}
                </button>
            </div>

            // New scenario form
            {move || (show_new.get() && editing_id.get().is_none()).then(|| view! {
                <ScenarioForm
                    auth=auth
                    positions=positions.get()
                    existing=None
                    on_saved=move |s: Scenario| {
                        scenarios.update(|ss| ss.insert(0, s));
                        show_new.set(false);
                    }
                    on_cancel=move || show_new.set(false)
                />
            })}

            {move || loading.get().then(|| view! {
                <p class="text-gray-400 text-sm">"Loading…"</p>
            })}

            {move || fetch_err.get().map(|e| view! {
                <p class="text-red-400 text-sm">"Failed to load scenarios: " {e}</p>
            })}

            // Active scenarios
            {move || {
                let ss = scenarios.get();
                let ps = positions.get();
                let eid = editing_id.get();
                let active: Vec<_> = ss.into_iter().filter(|s| !s.archived).collect();
                if !loading.get() && active.is_empty() && fetch_err.get().is_none() {
                    view! { <p class="text-gray-500 text-sm">"No scenarios yet. Create one above."</p> }.into_any()
                } else {
                    active.into_iter().map(|s| {
                        let id = s.id;
                        if eid == Some(id) {
                            let pos = ps.clone();
                            view! {
                                <ScenarioForm
                                    auth=auth positions=pos existing=Some(s)
                                    on_saved=move |updated: Scenario| {
                                        scenarios.update(|ss| {
                                            if let Some(e) = ss.iter_mut().find(|x| x.id == updated.id) { *e = updated; }
                                        });
                                        editing_id.set(None);
                                    }
                                    on_cancel=move || editing_id.set(None)
                                />
                            }.into_any()
                        } else {
                            let result = evaluate(&s, &ps);
                            view! {
                                <ScenarioCard scenario=s result=result auth=auth all_scenarios=scenarios
                                    on_edit=move || { show_new.set(false); editing_id.set(Some(id)); }
                                />
                            }.into_any()
                        }
                    }).collect_view().into_any()
                }
            }}

            // Archived section
            {move || {
                let n = scenarios.get().iter().filter(|s| s.archived).count();
                (n > 0).then(|| view! {
                    <div class="space-y-3 pt-2 border-t border-border">
                        <button
                            class="text-xs text-gray-500 hover:text-gray-300 transition-colors"
                            on:click=move |_| show_archived.update(|v| *v = !*v)
                        >
                            {move || {
                                let n = scenarios.get().iter().filter(|s| s.archived).count();
                                if show_archived.get() { format!("▴ Hide archived ({})", n) }
                                else { format!("▾ Show archived ({})", n) }
                            }}
                        </button>
                        {move || show_archived.get().then(|| {
                            let ss = scenarios.get();
                            let ps = positions.get();
                            let eid = editing_id.get();
                            ss.into_iter().filter(|s| s.archived).map(|s| {
                                let id = s.id;
                                if eid == Some(id) {
                                    let pos = ps.clone();
                                    view! {
                                        <ScenarioForm
                                            auth=auth positions=pos existing=Some(s)
                                            on_saved=move |updated: Scenario| {
                                                scenarios.update(|ss| {
                                                    if let Some(e) = ss.iter_mut().find(|x| x.id == updated.id) { *e = updated; }
                                                });
                                                editing_id.set(None);
                                            }
                                            on_cancel=move || editing_id.set(None)
                                        />
                                    }.into_any()
                                } else {
                                    let result = evaluate(&s, &ps);
                                    view! {
                                        <ScenarioCard scenario=s result=result auth=auth all_scenarios=scenarios
                                            on_edit=move || { editing_id.set(Some(id)); }
                                        />
                                    }.into_any()
                                }
                            }).collect_view()
                        })}
                    </div>
                })
            }}
        </div>
    }
}

// ── Scenario form (create + edit) ─────────────────────────────────────────────

#[derive(Clone)]
struct TradeEntry {
    id: Uuid,
    label: RwSignal<String>,
    symbol: RwSignal<String>,
    direction: RwSignal<TradeDirection>,
    contracts: RwSignal<String>,
    price: RwSignal<String>,
    is_option: RwSignal<bool>,
    opt_type: RwSignal<OptionType>,
    strike: RwSignal<String>,
    expiry: RwSignal<String>,
    closes_id: RwSignal<Option<Uuid>>,
}

impl TradeEntry {
    fn new() -> Self {
        Self {
            id: Uuid::new_v4(),
            label: RwSignal::new(String::new()),
            symbol: RwSignal::new(String::new()),
            direction: RwSignal::new(TradeDirection::Buy),
            contracts: RwSignal::new("1".to_string()),
            price: RwSignal::new(String::new()),
            is_option: RwSignal::new(true),
            opt_type: RwSignal::new(OptionType::Call),
            strike: RwSignal::new(String::new()),
            expiry: RwSignal::new(String::new()),
            closes_id: RwSignal::new(None),
        }
    }

    fn from_trade(t: &ScenarioTrade) -> Self {
        Self {
            id: t.id,
            label: RwSignal::new(t.label.clone()),
            symbol: RwSignal::new(t.symbol.clone()),
            direction: RwSignal::new(t.direction),
            contracts: RwSignal::new(t.contracts.to_string()),
            price: RwSignal::new(format!("{}", t.price)),
            is_option: RwSignal::new(t.option_spec.is_some()),
            opt_type: RwSignal::new(t.option_spec.as_ref().map(|s| s.option_type).unwrap_or(OptionType::Call)),
            strike: RwSignal::new(t.option_spec.as_ref().map(|s| format!("{}", s.strike)).unwrap_or_default()),
            expiry: RwSignal::new(t.option_spec.as_ref().map(|s| s.expiry.format("%Y-%m-%d").to_string()).unwrap_or_default()),
            closes_id: RwSignal::new(t.closes_position_id),
        }
    }

    fn to_trade(&self, positions: &[Position]) -> Result<ScenarioTrade, String> {
        let sym = self.symbol.get().trim().to_uppercase();
        if sym.is_empty() { return Err("Symbol required".into()); }
        let qty: u32 = self.contracts.get().trim().parse().map_err(|_| "Invalid contracts")?;
        let price: f64 = self.price.get().trim().parse().map_err(|_| "Invalid price")?;

        let option_spec = if self.is_option.get() {
            let s: f64 = self.strike.get().trim().parse().map_err(|_| "Invalid strike")?;
            let exp = NaiveDate::parse_from_str(self.expiry.get().trim(), "%Y-%m-%d")
                .map_err(|_| "Expiry must be YYYY-MM-DD")?;
            Some(OptionSpec { symbol: sym.clone(), option_type: self.opt_type.get(), strike: s, expiry: exp })
        } else { None };

        let (closes_position_id, closes_cost_basis, closes_is_long, closes_opened_at) =
            if let Some(pid) = self.closes_id.get() {
                if let Some(p) = positions.iter().find(|p| p.id == pid) {
                    (
                        Some(pid),
                        Some(p.effective_cost_basis()),
                        Some(p.effective_quantity() > 0),
                        Some(p.oldest_open_lot_date().date_naive()),
                    )
                } else { (None, None, None, None) }
            } else { (None, None, None, None) };

        let label = {
            let l = self.label.get();
            if l.trim().is_empty() {
                format!("{} {} {}", self.direction.get().label(), sym,
                    if self.is_option.get() { "option" } else { "stock" })
            } else { l }
        };

        Ok(ScenarioTrade {
            id: self.id, label, symbol: sym,
            direction: self.direction.get(), contracts: qty, price, option_spec,
            closes_position_id, closes_cost_basis, closes_is_long, closes_opened_at,
        })
    }
}

#[derive(Clone)]
struct MarketEntry {
    symbol: RwSignal<String>,
    price: RwSignal<String>,
    vol: RwSignal<String>,
    rate: RwSignal<String>,
}

impl MarketEntry {
    fn new() -> Self {
        Self {
            symbol: RwSignal::new(String::new()),
            price: RwSignal::new(String::new()),
            vol: RwSignal::new("25".to_string()),
            rate: RwSignal::new("3.75".to_string()),
        }
    }

    fn from_input(mi: &ScenarioMarketInput) -> Self {
        Self {
            symbol: RwSignal::new(mi.symbol.clone()),
            price: RwSignal::new(format!("{:.2}", mi.price)),
            vol: RwSignal::new(format!("{:.2}", mi.vol * 100.0)),
            rate: RwSignal::new(format!("{:.2}", mi.rate * 100.0)),
        }
    }

    fn to_input(&self) -> Result<ScenarioMarketInput, String> {
        let sym = self.symbol.get().trim().to_uppercase();
        if sym.is_empty() { return Err("Symbol required".into()); }
        let price: f64 = self.price.get().trim().parse().map_err(|_| "Invalid price")?;
        let vol: f64 = self.vol.get().trim().parse::<f64>().map_err(|_| "Invalid vol")? / 100.0;
        let rate: f64 = self.rate.get().trim().parse::<f64>().map_err(|_| "Invalid rate")? / 100.0;
        Ok(ScenarioMarketInput { symbol: sym, price, vol, rate })
    }

    fn as_mi(&self) -> Option<ScenarioMarketInput> { self.to_input().ok() }
}

#[component]
fn ScenarioForm(
    auth: AuthState,
    positions: Vec<Position>,
    existing: Option<Scenario>,
    on_saved: impl Fn(Scenario) + 'static,
    on_cancel: impl Fn() + 'static,
) -> impl IntoView {
    let on_saved  = Rc::new(on_saved);
    let on_cancel = Rc::new(on_cancel);

    let is_edit = existing.is_some();

    let name = RwSignal::new(existing.as_ref().map(|s| s.name.clone()).unwrap_or_default());
    let eval_date = RwSignal::new(existing.as_ref()
        .map(|s| s.evaluation_date.format("%Y-%m-%d").to_string())
        .unwrap_or_default());
    let market_entries = RwSignal::new(match &existing {
        Some(s) if !s.market_inputs.is_empty() => s.market_inputs.iter().map(MarketEntry::from_input).collect(),
        _ => vec![MarketEntry::new()],
    });
    let trade_entries = RwSignal::new(match &existing {
        Some(s) if !s.trades.is_empty() => s.trades.iter().map(TradeEntry::from_trade).collect(),
        _ => vec![TradeEntry::new()],
    });
    let err    = RwSignal::new(Option::<String>::None);
    let saving = RwSignal::new(false);

    // Set to true on first user edit of market inputs; gates the auto-recompute
    // effect so it doesn't overwrite saved trade prices on initial form load.
    let market_touched = RwSignal::new(false);

    Effect::new(move |_| {
        if !market_touched.get() { return; }
        let inputs: Vec<ScenarioMarketInput> = market_entries.get()
            .iter()
            .filter_map(|me| me.as_mi())
            .collect();
        let ed = NaiveDate::parse_from_str(eval_date.get().trim(), "%Y-%m-%d").ok();
        for te in trade_entries.get() {
            let sym = te.symbol.get().trim().to_uppercase();
            let mi = match inputs.iter().find(|m| m.symbol == sym) {
                Some(m) => m.clone(),
                None => continue,
            };
            if te.is_option.get() {
                let strike = match te.strike.get().trim().parse::<f64>() { Ok(v) => v, Err(_) => continue };
                let exp = match NaiveDate::parse_from_str(te.expiry.get().trim(), "%Y-%m-%d") { Ok(v) => v, Err(_) => continue };
                let ed = match ed { Some(d) => d, None => continue };
                let spec = OptionSpec { symbol: sym, option_type: te.opt_type.get(), strike, expiry: exp };
                if let Some(p) = bs_price(&spec, &mi, ed) {
                    te.price.set(format!("{:.4}", p));
                }
            } else {
                te.price.set(format!("{:.2}", mi.price));
            }
        }
    });

    let positions = Arc::new(positions);

    let existing_for_submit = existing.clone();

    let add_market = move |_: web_sys::MouseEvent| market_entries.update(|v| v.push(MarketEntry::new()));
    let add_trade  = move |_: web_sys::MouseEvent| trade_entries.update(|v| v.push(TradeEntry::new()));

    let on_submit = {
        let positions  = Arc::clone(&positions);
        let on_cancel2 = Rc::clone(&on_cancel);
        move |ev: web_sys::SubmitEvent| {
            ev.prevent_default();
            let n = name.get().trim().to_string();
            if n.is_empty() { err.set(Some("Name required.".into())); return; }
            let ed = match NaiveDate::parse_from_str(eval_date.get().trim(), "%Y-%m-%d") {
                Ok(d) => d,
                Err(_) => { err.set(Some("Evaluation date must be YYYY-MM-DD.".into())); return; }
            };

            // Preserve id / created_at / archived when editing
            let mut scenario = match &existing_for_submit {
                Some(ex) => Scenario { name: n, evaluation_date: ed, market_inputs: vec![], trades: vec![], ..ex.clone() },
                None => Scenario::new(&n, ed),
            };

            for me in market_entries.get() {
                match me.to_input() {
                    Ok(mi) => scenario.market_inputs.push(mi),
                    Err(e) => { err.set(Some(format!("Market input: {}", e))); return; }
                }
            }
            for te in trade_entries.get() {
                match te.to_trade(&positions) {
                    Ok(t) => scenario.trades.push(t),
                    Err(e) => { err.set(Some(format!("Trade: {}", e))); return; }
                }
            }

            saving.set(true);
            let token   = auth.token.get().unwrap_or_default();
            let user_id = auth.user_id.get().unwrap_or_default();
            let s    = scenario.clone();
            let cb   = Rc::clone(&on_saved);
            let cancel = Rc::clone(&on_cancel2);
            spawn_local(async move {
                match supabase::upsert_scenario(&token, &user_id, &s).await {
                    Ok(_)  => cb(s),
                    Err(e) => { err.set(Some(e)); saving.set(false); let _ = cancel; }
                }
            });
        }
    };

    view! {
        <form on:submit=on_submit class="bg-panel border border-border rounded-xl p-6 space-y-6">
            <div class="flex items-center justify-between">
                <h2 class="text-sm font-medium text-gray-300">
                    {if is_edit { "Edit scenario" } else { "New scenario" }}
                </h2>
                <button type="button" class="text-xs text-gray-500 hover:text-gray-300"
                    on:click=move |_| on_cancel()>
                    "Cancel"
                </button>
            </div>

            <div class="grid grid-cols-2 gap-3">
                <FormInput label="Name" signal=name ph="e.g. Roll AAPL puts to June" />
                <FormInput label="Evaluation date (YYYY-MM-DD)" signal=eval_date ph="2025-06-20" />
            </div>

            <div class="space-y-2">
                <p class="text-xs font-medium text-gray-400 uppercase tracking-wider">"Market inputs"</p>
                <div class="grid grid-cols-[1fr_1fr_1fr_1fr_auto_auto] gap-2 items-center text-xs text-gray-500">
                    <span>"Symbol"</span><span>"Price"</span><span>"IV %"</span><span>"Rate %"</span><span/><span/>
                </div>
                {move || market_entries.get().into_iter().enumerate().map(|(i, me)| {
                    let me_fill = me.clone();
                    // Extract signals as named Copy bindings so each closure below
                    // captures an unambiguous RwSignal<String> rather than a field
                    // path through a non-Copy struct.
                    let (sym_sig, price_sig, vol_sig, rate_sig) =
                        (me.symbol, me.price, me.vol, me.rate);
                    let fill = move |_: web_sys::MouseEvent| {
                        let sym = me_fill.symbol.get().trim().to_uppercase();
                        if sym.is_empty() { return; }
                        let token = auth.token.get().unwrap_or_default();
                        let me2 = me_fill.clone();
                        spawn_local(async move {
                            // Live spot price
                            let spot = match market::fetch_quote(&token, &sym).await {
                                Ok(q) => { me2.price.set(format!("{:.2}", q.price)); q.price }
                                Err(_) => return,
                            };
                            // ATM forward vol: nearest-expiry straddle from live option chain
                            if let Ok(chain) = market::fetch_option_chain(&token, &sym).await {
                                if let Some(vol) = atm_iv_from_chain(&chain, spot) {
                                    me2.vol.set(format!("{:.1}", vol * 100.0));
                                }
                            }
                            market_touched.set(true);
                        });
                    };
                    view! {
                        <div class="grid grid-cols-[1fr_1fr_1fr_1fr_auto_auto] gap-2 items-center">
                            <MicroInput signal=sym_sig ph="AAPL" />
                            <input
                                class="w-full bg-surface border border-border rounded px-2 py-1 text-sm focus:outline-none focus:border-blue-500"
                                prop:value=move || price_sig.get()
                                on:input=move |ev| { price_sig.set(event_target_value(&ev)); market_touched.set(true); }
                                on:keydown=move |ev: web_sys::KeyboardEvent| {
                                    let key = ev.key();
                                    if key == "ArrowUp" || key == "ArrowDown" {
                                        ev.prevent_default();
                                        let dir = if key == "ArrowUp" { 1.0_f64 } else { -1.0_f64 };
                                        if let Ok(p) = price_sig.get().trim().parse::<f64>() {
                                            price_sig.set(format!("{:.2}", (p * (1.0 + dir * 0.01)).max(0.0)));
                                            market_touched.set(true);
                                        }
                                    }
                                }
                                placeholder="155.00"
                            />
                            <input
                                class="w-full bg-surface border border-border rounded px-2 py-1 text-sm focus:outline-none focus:border-blue-500"
                                prop:value=move || vol_sig.get()
                                on:input=move |ev| { vol_sig.set(event_target_value(&ev)); market_touched.set(true); }
                                on:keydown=move |ev: web_sys::KeyboardEvent| {
                                    let key = ev.key();
                                    if key == "ArrowUp" || key == "ArrowDown" {
                                        ev.prevent_default();
                                        let dir = if key == "ArrowUp" { 1.0_f64 } else { -1.0_f64 };
                                        if let Ok(v) = vol_sig.get().trim().parse::<f64>() {
                                            vol_sig.set(format!("{:.2}", (v + dir).max(0.0)));
                                            market_touched.set(true);
                                        }
                                    }
                                }
                                placeholder="25"
                            />
                            <input
                                class="w-full bg-surface border border-border rounded px-2 py-1 text-sm focus:outline-none focus:border-blue-500"
                                prop:value=move || rate_sig.get()
                                on:input=move |ev| { rate_sig.set(event_target_value(&ev)); market_touched.set(true); }
                                on:keydown=move |ev: web_sys::KeyboardEvent| {
                                    let key = ev.key();
                                    if key == "ArrowUp" || key == "ArrowDown" {
                                        ev.prevent_default();
                                        let dir = if key == "ArrowUp" { 1.0_f64 } else { -1.0_f64 };
                                        if let Ok(r) = rate_sig.get().trim().parse::<f64>() {
                                            rate_sig.set(format!("{:.2}", r + dir * 0.1));
                                            market_touched.set(true);
                                        }
                                    }
                                }
                                placeholder="3.75"
                            />
                            <button type="button"
                                class="text-gray-500 hover:text-blue-300 text-xs border border-border rounded px-1.5 py-1 transition-colors"
                                on:click=fill
                                title="Fill spot price from live quote; fill IV from ATM straddle (nearest expiry)"
                            >"↗"</button>
                            <button type="button" class="text-gray-600 hover:text-red-400 text-xs"
                                on:click=move |_| market_entries.update(|v| { if v.len() > 1 { v.remove(i); } })>
                                "✕"
                            </button>
                        </div>
                    }
                }).collect_view()}
                <button type="button" class="text-xs text-blue-400 hover:text-blue-300" on:click=add_market>
                    "+ add symbol"
                </button>
            </div>

            <div class="space-y-3">
                <p class="text-xs font-medium text-gray-400 uppercase tracking-wider">"Trades"</p>
                {move || trade_entries.get().into_iter().enumerate().map(|(i, te)| {
                    let positions_for_row = Arc::clone(&positions);
                    let market_for_row = market_entries;
                    view! {
                        <TradeEntryRow
                            entry=te
                            positions=(*positions_for_row).clone()
                            market_entries=market_for_row
                            eval_date=eval_date
                            auth=auth
                            on_remove=move || trade_entries.update(|v| { if v.len() > 1 { v.remove(i); } })
                        />
                    }
                }).collect_view()}
                <button type="button" class="text-xs text-blue-400 hover:text-blue-300" on:click=add_trade>
                    "+ add leg"
                </button>
            </div>

            {move || err.get().map(|e| view! { <p class="text-red-400 text-xs">{e}</p> })}

            <button type="submit"
                class="bg-blue-600 hover:bg-blue-500 disabled:opacity-50 px-4 py-2 rounded text-sm font-medium"
                prop:disabled=move || saving.get()
            >
                {move || if saving.get() { "Saving…" } else if is_edit { "Save changes" } else { "Create & evaluate" }}
            </button>
        </form>
    }
}

#[component]
fn TradeEntryRow(
    entry: TradeEntry,
    positions: Vec<Position>,
    market_entries: RwSignal<Vec<MarketEntry>>,
    eval_date: RwSignal<String>,
    auth: AuthState,
    on_remove: impl Fn() + 'static,
) -> impl IntoView {
    let positions = Rc::new(positions);
    let pos_for_select = Rc::clone(&positions);

    // Forward vol from eval_date to this option's expiry (None until surface is fetched).
    let fwd_vol = RwSignal::new(Option::<f64>::None);

    // Fetch option chain metadata when symbol + is_option are set; cache in MarketStore.
    let store = use_context::<MarketStore>().expect("MarketStore missing");
    let option_meta = RwSignal::new(Vec::<OptionMetaEntry>::new());
    Effect::new(move |_| {
        let sym = entry.symbol.get().trim().to_uppercase();
        let is_opt = entry.is_option.get();
        if is_opt && !sym.is_empty() {
            if let Some(cached) = store.option_meta.get_untracked().get(&sym).cloned() {
                option_meta.set(cached);
                return;
            }
            let tok = auth.token.get().unwrap_or_default();
            spawn_local(async move {
                match market::fetch_option_meta(&tok, &sym).await {
                    Ok(meta) => {
                        store.option_meta.update(|map| { map.insert(sym.clone(), meta.clone()); });
                        option_meta.set(meta);
                    }
                    Err(_) => option_meta.set(vec![]),
                }
            });
        } else {
            option_meta.set(vec![]);
        }
    });

    // Fetch forward vol whenever symbol / expiry / eval_date changes.
    // Fails silently if the vol surface table has no data yet.
    Effect::new(move |_| {
        let sym = entry.symbol.get().trim().to_uppercase();
        let expiry_str = entry.expiry.get();
        let eval_str = eval_date.get();
        fwd_vol.set(None);
        if entry.is_option.get() && !sym.is_empty() && !expiry_str.is_empty() && !eval_str.is_empty() {
            let tok = auth.token.get().unwrap_or_default();
            spawn_local(async move {
                if let Ok(fv) = market::fetch_forward_vol(&tok, &sym, &eval_str, &expiry_str).await {
                    fwd_vol.set(Some(fv.forward_vol));
                }
            });
        }
    });

    let expiries = Memo::new(move |_| {
        let mut seen = std::collections::HashSet::new();
        let mut v: Vec<String> = option_meta.get()
            .into_iter()
            .filter_map(|e| seen.insert(e.expiry.clone()).then_some(e.expiry))
            .collect();
        v.sort();
        v
    });

    let strikes = Memo::new(move |_| {
        let type_str = if entry.opt_type.get() == OptionType::Call { "call" } else { "put" };
        let sel_exp = entry.expiry.get();
        let mut v: Vec<f64> = option_meta.get()
            .into_iter()
            .filter(|e| e.expiry == sel_exp && e.option_type == type_str)
            .map(|e| e.strike)
            .collect();
        v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        v
    });

    let on_closes_change = {
        let positions = Rc::clone(&positions);
        let entry = entry.clone();
        move |ev: web_sys::Event| {
            let val = event_target_value(&ev);
            if val.is_empty() { entry.closes_id.set(None); return; }
            let pid: Uuid = match val.parse() { Ok(v) => v, Err(_) => return };
            entry.closes_id.set(Some(pid));
            if let Some(p) = positions.iter().find(|p| p.id == pid) {
                entry.symbol.set(p.symbol.clone());
                entry.direction.set(if p.quantity > 0 { TradeDirection::Sell } else { TradeDirection::Buy });
                entry.contracts.set(p.quantity.unsigned_abs().to_string());
                if let Some(spec) = &p.option_spec {
                    entry.is_option.set(true);
                    entry.opt_type.set(spec.option_type);
                    entry.strike.set(format!("{}", spec.strike));
                    entry.expiry.set(spec.expiry.format("%Y-%m-%d").to_string());
                } else {
                    entry.is_option.set(false);
                }
            }
        }
    };

    let entry_for_compute = entry.clone();
    let compute_price = move |_: web_sys::MouseEvent| {
        if !entry_for_compute.is_option.get() { return; }
        let sym = entry_for_compute.symbol.get().trim().to_uppercase();
        if sym.is_empty() { return; }
        let strike: f64 = match entry_for_compute.strike.get().trim().parse() {
            Ok(v) => v, Err(_) => return,
        };
        let expiry_str = entry_for_compute.expiry.get();
        if expiry_str.trim().is_empty() { return; }
        let opt_type = entry_for_compute.opt_type.get();

        // Price via B-S at eval_date. Use forward vol (eval_date→expiry) from the
        // SABR surface when available; otherwise fall back to the scenario's ATM vol.
        let mi = market_entries.get().into_iter()
            .find(|m| m.symbol.get().trim().to_uppercase() == sym)
            .and_then(|m| m.as_mi());
        let ed = NaiveDate::parse_from_str(eval_date.get().trim(), "%Y-%m-%d").ok();
        if let (Some(mut mi), Some(ed)) = (mi, ed) {
            if let Some(fv) = fwd_vol.get_untracked() {
                mi.vol = fv;
            }
            if let Ok(exp) = NaiveDate::parse_from_str(expiry_str.trim(), "%Y-%m-%d") {
                let spec = OptionSpec { symbol: sym, option_type: opt_type, strike, expiry: exp };
                if let Some(p) = bs_price(&spec, &mi, ed) {
                    entry_for_compute.price.set(format!("{:.4}", p));
                }
            }
        }
    };

    view! {
        <div class="bg-surface border border-border rounded-lg p-3 space-y-2">
            <div class="flex flex-wrap gap-2 items-center">
                <input class=MICRO_CLS prop:value=move || entry.label.get()
                    on:input=move |ev| entry.label.set(event_target_value(&ev))
                    placeholder="Label (optional)" style="width:12rem" />

                {[TradeDirection::Buy, TradeDirection::Sell].map(|d| view! {
                    <button type="button"
                        class=move || format!("px-3 py-1 rounded text-xs border transition-colors {}",
                            if entry.direction.get() == d { "bg-blue-600 border-blue-600 text-white" }
                            else { "bg-panel border-border text-gray-400" })
                        on:click=move |_| entry.direction.set(d)
                    >{d.label()}</button>
                })}

                <input class=MICRO_CLS prop:value=move || entry.symbol.get()
                    on:input=move |ev| entry.symbol.set(event_target_value(&ev))
                    placeholder="Symbol" style="width:5rem" />
                <input class=MICRO_CLS prop:value=move || entry.contracts.get()
                    on:input=move |ev| entry.contracts.set(event_target_value(&ev))
                    placeholder="Qty" style="width:4rem" />
                <input class=MICRO_CLS prop:value=move || entry.price.get()
                    on:input=move |ev| entry.price.set(event_target_value(&ev))
                    placeholder="Price" style="width:6rem" />

                <button type="button"
                    class=move || format!("px-2 py-1 rounded text-xs border transition-colors {}",
                        if entry.is_option.get() { "bg-indigo-700 border-indigo-600 text-white" }
                        else { "bg-panel border-border text-gray-400" })
                    on:click=move |_| entry.is_option.update(|v| *v = !*v)
                >
                    {move || if entry.is_option.get() { "Option" } else { "Stock" }}
                </button>

                <button type="button"
                    class="text-xs text-gray-500 hover:text-blue-300 border border-border rounded px-2 py-1"
                    on:click=compute_price title="Compute B-S price from market inputs"
                >"B-S ▶"</button>

                <button type="button" class="text-gray-600 hover:text-red-400 text-xs ml-auto"
                    on:click=move |_| on_remove()>"✕"</button>
            </div>

            {move || entry.is_option.get().then(|| view! {
                <div class="flex flex-wrap gap-2 items-center pl-2">
                    {[OptionType::Call, OptionType::Put].map(|t| view! {
                        <button type="button"
                            class=move || format!("px-2 py-0.5 rounded text-xs border {}",
                                if entry.opt_type.get() == t { "bg-blue-600 border-blue-600 text-white" }
                                else { "bg-panel border-border text-gray-400" })
                            on:click=move |_| entry.opt_type.set(t)
                        >{t.label()}</button>
                    })}
                    {move || {
                        if entry.closes_id.get().is_some() {
                            // Closing trade: expiry/strike locked to the position being closed.
                            view! {
                                <span class="text-xs font-mono text-gray-400 px-2 py-1 border border-border rounded">
                                    {move || entry.expiry.get()}
                                </span>
                                <span class="text-xs font-mono text-gray-400 px-2 py-1 border border-border rounded">
                                    {move || format!("${}", entry.strike.get())}
                                </span>
                            }.into_any()
                        } else {
                            view! {
                                <select
                                    class=MICRO_CLS
                                    style="min-width:9rem"
                                    prop:value=move || { let _ = expiries.get(); entry.expiry.get() }
                                    on:change=move |ev| {
                                        entry.expiry.set(event_target_value(&ev));
                                        entry.strike.set(String::new());
                                    }
                                >
                                    <option value="">"— expiry —"</option>
                                    {move || {
                                        let list = expiries.get();
                                        let current = entry.expiry.get_untracked();
                                        let show_saved = !current.is_empty() && !list.contains(&current);
                                        list.into_iter()
                                            .chain(show_saved.then(|| current).into_iter())
                                            .map(|exp| view! { <option value=exp.clone()>{exp.clone()}</option> })
                                            .collect_view()
                                    }}
                                </select>
                                <select
                                    class=MICRO_CLS
                                    style="min-width:6rem"
                                    prop:value=move || { let _ = strikes.get(); entry.strike.get() }
                                    on:change=move |ev| entry.strike.set(event_target_value(&ev))
                                >
                                    <option value="">"— strike —"</option>
                                    {move || {
                                        let list = strikes.get();
                                        let current = entry.strike.get_untracked();
                                        let current_f: Option<f64> = current.trim().parse().ok();
                                        let show_saved = current_f.is_some() && !list.contains(current_f.as_ref().unwrap());
                                        list.into_iter()
                                            .chain(show_saved.then(|| current_f.unwrap()).into_iter())
                                            .map(|s| {
                                                let val = format!("{}", s);
                                                view! { <option value=val.clone()>{format!("${:.0}", s)}</option> }
                                            })
                                            .collect_view()
                                    }}
                                </select>
                            }.into_any()
                        }
                    }}
                    // Forward vol badge — appears once the SABR surface has data.
                    {move || fwd_vol.get().map(|v| view! {
                        <span
                            class="text-xs font-mono text-indigo-300 px-1.5 py-0.5 bg-indigo-900/30 border border-indigo-800/50 rounded"
                            title="Forward vol from eval date to expiry (SABR variance surface)"
                        >
                            {format!("fwd {:.1}%", v * 100.0)}
                        </span>
                    })}
                </div>
            })}

            <div class="flex items-center gap-2 pl-2">
                <span class="text-xs text-gray-500">"Closes:"</span>
                <select
                    class="bg-surface border border-border rounded px-2 py-1 text-xs text-gray-300 focus:outline-none focus:border-blue-500"
                    prop:value=move || entry.closes_id.get().map(|id| id.to_string()).unwrap_or_default()
                    on:change=on_closes_change
                >
                    <option value="">"— none —"</option>
                    {pos_for_select.iter().map(|p| {
                        let id = p.id.to_string();
                        let label = match &p.option_spec {
                            Some(spec) => format!("{} {} ${:.0} {} ×{:+}",
                                p.symbol, spec.option_type.label(), spec.strike,
                                spec.expiry.format("%d-%b-%y"), p.quantity),
                            None => format!("{} stock ×{:+}", p.symbol, p.quantity),
                        };
                        view! { <option value=id.clone()>{label}</option> }
                    }).collect_view()}
                </select>
                {move || entry.closes_id.get().map(|_| view! {
                    <span class="text-xs text-yellow-400">"⚡ P&L will be computed"</span>
                })}
            </div>
        </div>
    }
}

const MICRO_CLS: &str =
    "bg-surface border border-border rounded px-2 py-1 text-sm focus:outline-none focus:border-blue-500";

// ── Scenario card ─────────────────────────────────────────────────────────────

#[component]
fn ScenarioCard(
    scenario: Scenario,
    result: ScenarioResult,
    auth: AuthState,
    all_scenarios: RwSignal<Vec<Scenario>>,
    on_edit: impl Fn() + 'static,
) -> impl IntoView {
    let expanded = RwSignal::new(false);

    let tax = RwSignal::new(Option::<f64>::None);
    let baseline_tax = RwSignal::new(Option::<f64>::None);
    let tax_err = RwSignal::new(Option::<String>::None);
    let tax_year = scenario.evaluation_date.year();

    let cash_class;

    let id            = scenario.id;
    let is_archived   = scenario.archived;
    let name          = scenario.name.clone();
    let eval_date_str = scenario.evaluation_date.format("%d %b %Y").to_string();
    let market_inputs = scenario.market_inputs.clone();
    let trade_results = result.trade_results.clone();
    let assignments   = result.assignments.clone();
    let coverage      = result.coverage.clone();
    let greeks        = result.greeks;
    let net_cash      = result.net_cash;
    let total_st      = result.total_st_gain;
    let total_lt      = result.total_lt_gain;
    let after_tax_cash = move || net_cash - tax.get().unwrap_or(0.0);
    cash_class = move || if after_tax_cash() >= 0.0 { "text-green-400" } else { "text-red-400" };
    let has_market    = !market_inputs.is_empty();

    // Marginal federal tax for this scenario's realized gains, computed by the
    // Worker against the user's profile for the scenario's tax year.
    Effect::new(move |_| {
        let tok = match auth.token.get() { Some(t) => t, None => return };
        tax_err.set(None);

        // Still fetch baseline_tax even when gains are zero.
        let (st, lt) = if total_st.abs() < 1e-9 && total_lt.abs() < 1e-9 {
            (0.0, 0.0)
        } else {
            (total_st, total_lt)
        };

        spawn_local(async move {
            match market::estimate_trade_tax(&tok, tax_year, st, lt).await {
                Ok(r) => { tax.set(Some(r.tax)); baseline_tax.set(Some(r.baseline_tax)); }
                Err(e) => { tax.set(None); baseline_tax.set(None); tax_err.set(Some(e)); }
            }
        });
    });

    let toggle_archive = move |ev: web_sys::MouseEvent| {
        ev.stop_propagation();
        let tok = auth.token.get().unwrap_or_default();
        let uid = auth.user_id.get().unwrap_or_default();
        all_scenarios.update(|ss| {
            if let Some(s) = ss.iter_mut().find(|s| s.id == id) { s.archived = !s.archived; }
        });
        if let Some(s) = all_scenarios.get().into_iter().find(|s| s.id == id) {
            spawn_local(async move { let _ = supabase::upsert_scenario(&tok, &uid, &s).await; });
        }
    };

    view! {
        <div class="bg-panel border border-border rounded-xl overflow-hidden">

            // ── Header ────────────────────────────────────────────────────
            <div
                class="flex items-center justify-between px-6 py-4 cursor-pointer hover:bg-white/5 transition-colors select-none"
                on:click=move |_| expanded.update(|v| *v = !*v)
            >
                <div class="flex items-center gap-3 min-w-0">
                    <span class="text-gray-500 text-xs w-3 shrink-0">
                        {move || if expanded.get() { "▾" } else { "▸" }}
                    </span>
                    <div>
                        <p class="font-medium">{name}</p>
                        <p class="text-xs text-gray-500">{eval_date_str}</p>
                    </div>
                </div>
                <div class="flex items-center gap-2 shrink-0">
                    <div class="text-right mr-2">
                        <p class="text-xs text-gray-500">"Net cash"</p>
                        <p class=move || format!("text-lg font-semibold {}", cash_class())>
                            {move || view! { <Num value=after_tax_cash() signed=true /> }}
                        </p>
                    </div>
                    <button
                        class="text-xs text-gray-500 hover:text-blue-400 border border-border rounded px-2 py-1 transition-colors"
                        on:click=move |ev: web_sys::MouseEvent| { ev.stop_propagation(); on_edit(); }
                    >"Edit"</button>
                    <button
                        class="text-xs text-gray-500 hover:text-yellow-400 border border-border rounded px-2 py-1 transition-colors"
                        on:click=toggle_archive
                    >{if is_archived { "Unarchive" } else { "Archive" }}</button>
                </div>
            </div>

            // ── Expanded body ─────────────────────────────────────────────
            {move || expanded.get().then(|| view! {
                <div class="px-6 pb-6 pt-4 border-t border-border space-y-4">

                    {(!market_inputs.is_empty()).then(|| view! {
                        <div class="flex flex-wrap gap-3 text-xs text-gray-500">
                            {market_inputs.iter().map(|mi| view! {
                                <span>
                                    <span class="font-semibold text-gray-300">{mi.symbol.clone()}</span>
                                    " $" {format!("{:.2}", mi.price)}
                                    " IV " {format!("{:.0}%", mi.vol * 100.0)}
                                </span>
                            }).collect_view()}
                        </div>
                    })}

                    {(!trade_results.is_empty()).then(|| view! {
                        <div class="space-y-1">
                            <p class="text-xs text-gray-500 uppercase tracking-wider">"Trades"</p>
                            {trade_results.iter().map(|tr| view! {
                                <div class="flex justify-between items-start text-xs">
                                    <span class="text-gray-400">{tr.label.clone()}</span>
                                    <div class="text-right space-y-0.5">
                                        <p class={if tr.cash_flow >= 0.0 { "text-green-400" } else { "text-red-400" }}>
                                            {fmt_cash(tr.cash_flow)} " cash"
                                        </p>
                                        {tr.realized_gain.map(|g| view! {
                                            <p class={if g >= 0.0 { "text-green-300" } else { "text-red-300" }}>
                                                {fmt_cash(g)} " gain (" {if tr.is_long_term { "LT" } else { "ST" }} ")"
                                            </p>
                                        })}
                                    </div>
                                </div>
                            }).collect_view()}
                        </div>
                    })}

                    {(!assignments.is_empty()).then(|| view! {
                        <div class="space-y-1">
                            <p class="text-xs text-yellow-500 uppercase tracking-wider">"⚡ Auto-detected assignments"</p>
                            {assignments.iter().map(|a| view! {
                                <div class="flex justify-between items-start text-xs">
                                    <div>
                                        <p class="text-yellow-300">{a.description.clone()}</p>
                                        <p class="text-gray-500">
                                            {match a.option_type {
                                                OptionType::Call => format!("Shares called away at ${:.2}", a.strike),
                                                OptionType::Put  => format!("Shares put at ${:.2}", a.strike),
                                            }}
                                        </p>
                                    </div>
                                    <div class="text-right space-y-0.5">
                                        <p class={if a.stock_cash_flow >= 0.0 { "text-green-400" } else { "text-red-400" }}>
                                            {fmt_cash(a.stock_cash_flow)} " stock cash"
                                        </p>
                                        <p class={if a.realized_gain >= 0.0 { "text-green-300" } else { "text-red-300" }}>
                                            {fmt_cash(a.realized_gain)} " gain (" {if a.is_long_term { "LT" } else { "ST" }} ")"
                                        </p>
                                    </div>
                                </div>
                            }).collect_view()}
                        </div>
                    })}

                    {(!coverage.is_empty()).then(|| view! {
                        <div class="space-y-1">
                            <p class="text-xs text-gray-500 uppercase tracking-wider">"Position coverage"</p>
                            {coverage.iter().map(|c| {
                                let uncovered      = c.uncovered_shares();
                                let excess         = c.excess_short_delta();
                                let sym            = c.symbol.clone();
                                let net_shares     = c.net_shares;
                                let net_calls      = c.net_short_call_contracts;
                                let upside_before  = c.upside_before;
                                let upside_after   = c.upside_after;
                                let upside_delta   = upside_after - upside_before;
                                view! {
                                    <div class="flex justify-between items-start text-xs">
                                        <span class="text-gray-400">{sym}</span>
                                        <div class="text-right space-y-0.5">
                                            // Upside row — always show if there are short calls before or after.
                                            {(upside_before > 0.0 || upside_after > 0.0).then(|| {
                                                let delta_str = if upside_delta.abs() < 1.0 {
                                                    String::new()
                                                } else if upside_delta > 0.0 {
                                                    format!(" (+${:.0})", upside_delta)
                                                } else {
                                                    format!(" (−${:.0})", -upside_delta)
                                                };
                                                let delta_class = if upside_delta >= 0.0 { "text-blue-300" } else { "text-red-400" };
                                                view! {
                                                    <p class=delta_class>
                                                        {format!("Upside ${:.0} → ${:.0}{}", upside_before, upside_after, delta_str)}
                                                    </p>
                                                }
                                            })}
                                            {(excess > 0).then(|| view! {
                                                <p class="text-red-400">
                                                    {format!("−{} net delta ({} excess contracts)", excess, excess / 100)}
                                                </p>
                                            })}
                                            {(uncovered > 0).then(|| view! {
                                                <p class="text-yellow-400">
                                                    {format!("{} shares uncovered", uncovered)}
                                                </p>
                                            })}
                                            {(uncovered == 0 && excess == 0).then(|| view! {
                                                <p class="text-gray-500">
                                                    {format!("{} shares, {} contracts", net_shares, net_calls)}
                                                </p>
                                            })}
                                        </div>
                                    </div>
                                }
                            }).collect_view()}
                        </div>
                    })}

                    <div class="border-t border-border pt-3 grid grid-cols-2 gap-4 text-xs">
                        <div>
                            <p class="text-gray-500 mb-1">"Realized gains"</p>
                            <p class="text-yellow-300">"ST: " {fmt_cash(total_st)}</p>
                            <p class="text-blue-300">"LT: " {fmt_cash(total_lt)}</p>
                            <div class="mt-2 space-y-0.5">
                                {move || {
                                    if let Some(e) = tax_err.get() {
                                        let is_missing = e.contains("No tax profile");
                                        view! {
                                            <p class="text-orange-300">
                                                {if is_missing {
                                                    format!("Tax ({tax_year}): no profile — ")
                                                } else {
                                                    format!("Tax ({tax_year}): {} — ", e)
                                                }}
                                                <a href="/tax" class="underline text-gray-400">"set up Taxes"</a>
                                            </p>
                                        }.into_any()
                                    } else if let Some(bt) = baseline_tax.get() {
                                        let delta = tax.get().unwrap_or(0.0);
                                        let impact_str = if delta.abs() < 1.0 {
                                            "no tax impact".to_string()
                                        } else if delta > 0.0 {
                                            format!("costs {} in tax", fmt_cash(delta))
                                        } else {
                                            format!("saves {} in tax", fmt_cash(-delta))
                                        };
                                        view! {
                                            <p class="text-gray-400">
                                                {format!("Base tax ({tax_year}): {}", fmt_cash(bt))}
                                            </p>
                                            <p class=move || if delta <= 0.0 { "text-green-400" } else { "text-orange-300" }>
                                                {format!("Scenario: {}", impact_str)}
                                            </p>
                                        }.into_any()
                                    } else {
                                        view! {
                                            <p class="text-gray-500">"Tax: computing…"</p>
                                        }.into_any()
                                    }
                                }}
                            </div>
                        </div>
                        <div class="text-right">
                            <p class="text-gray-500 mb-1">"Pre-tax cash"</p>
                            <p class=format!("text-xl font-semibold {}", if net_cash >= 0.0 { "text-green-300" } else { "text-red-300" })>
                                <Num value=net_cash signed=true />
                            </p>
                        </div>
                    </div>

                    {has_market.then(|| view! {
                        <div class="border-t border-border pt-3">
                            <p class="text-xs text-gray-500 mb-2">"New Portfolio Greeks"</p>
                            <div class="flex flex-wrap gap-x-6 gap-y-1 text-xs font-mono">
                                <span class="text-gray-500">"Δ "<span class="text-gray-200">{format!("{:.2}", greeks.delta)}</span></span>
                                <span class="text-gray-500">"Γ "<span class="text-gray-200">{format!("{:.4}", greeks.gamma)}</span></span>
                                <span class="text-gray-500">"ν "<span class="text-gray-200">{format!("{:.2}", greeks.vega)}</span></span>
                                <span class="text-gray-500">"θ "<span class="text-gray-200">{format!("{:.2}", greeks.theta)}</span></span>
                                <span class="text-gray-500">"ρ "<span class="text-gray-200">{format!("{:.2}", greeks.rho)}</span></span>
                            </div>
                        </div>
                    })}
                </div>
            })}
        </div>
    }
}

use crate::format::{fmt_cash, Num};

// ── Shared form components ────────────────────────────────────────────────────

#[component]
fn FormInput(label: &'static str, signal: RwSignal<String>, ph: &'static str) -> impl IntoView {
    view! {
        <div>
            <label class="block text-xs text-gray-400 mb-1">{label}</label>
            <input
                class="w-full bg-surface border border-border rounded px-3 py-1.5 text-sm focus:outline-none focus:border-blue-500"
                prop:value=move || signal.get()
                on:input=move |ev| signal.set(event_target_value(&ev))
                placeholder=ph
            />
        </div>
    }
}

#[component]
fn MicroInput(signal: RwSignal<String>, ph: &'static str) -> impl IntoView {
    view! {
        <input
            class="w-full bg-surface border border-border rounded px-2 py-1 text-sm focus:outline-none focus:border-blue-500"
            prop:value=move || signal.get()
            on:input=move |ev| signal.set(event_target_value(&ev))
            placeholder=ph
        />
    }
}
