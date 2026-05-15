use std::rc::Rc;

use chrono::{NaiveDate, Utc};
use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

use crate::api::supabase;
use crate::app::AuthState;
use crate::models::{
    option::OptionType,
    position::{Position, PositionKind},
    scenario::{LegResult, Scenario, ScenarioResult},
};
use crate::pricing::black_scholes::BsInputs;

// ── Scenario evaluation ───────────────────────────────────────────────────────

fn evaluate_scenario(positions: &[Position], scenario: &Scenario) -> ScenarioResult {
    let price_map = scenario.price_map();
    let eval_date = scenario.evaluation_date;
    let rate = 0.05_f64;

    let mut legs: Vec<LegResult> = Vec::new();

    for pos in positions {
        let assumed_price = match price_map.get(&pos.symbol) {
            Some(&p) => p,
            None => continue,
        };

        let (mark_value, description) = match &pos.kind {
            PositionKind::Stock => {
                let mv = assumed_price * pos.quantity as f64;
                let desc = format!("{} stock ×{:+}", pos.symbol, pos.quantity);
                (mv, desc)
            }
            PositionKind::Option => {
                let spec = match &pos.option_spec {
                    Some(s) => s,
                    None => continue,
                };
                let t = (spec.expiry - eval_date).num_days().max(0) as f64 / 365.0;
                let mark = if t <= 0.0 {
                    match spec.option_type {
                        OptionType::Call => (assumed_price - spec.strike).max(0.0),
                        OptionType::Put => (spec.strike - assumed_price).max(0.0),
                    }
                } else {
                    BsInputs {
                        spot: assumed_price,
                        strike: spec.strike,
                        expiry_years: t,
                        vol: 0.25,
                        rate,
                    }
                    .price(spec.option_type)
                };
                let mv = mark * pos.quantity as f64 * 100.0;
                let desc = format!(
                    "{} {} ${:.0} {} ×{:+}",
                    pos.symbol,
                    spec.option_type.label(),
                    spec.strike,
                    spec.expiry.format("%d-%b-%y"),
                    pos.quantity,
                );
                (mv, desc)
            }
        };

        let cost = pos.total_cost();
        let pnl = mark_value - cost;
        let days_held = (eval_date - pos.opened_at.date_naive()).num_days();
        let (lt, st) = if pnl > 0.0 && days_held > 365 { (pnl, 0.0) } else { (0.0, pnl) };

        legs.push(LegResult {
            description,
            pnl,
            short_term_gain: st,
            long_term_gain: lt,
        });
    }

    let total_pnl = legs.iter().map(|l| l.pnl).sum();
    let total_st = legs.iter().map(|l| l.short_term_gain).sum();
    let total_lt = legs.iter().map(|l| l.long_term_gain).sum();

    ScenarioResult {
        evaluated_at: Utc::now(),
        evaluation_date: eval_date,
        legs,
        total_pnl,
        total_short_term: total_st,
        total_long_term: total_lt,
    }
}

// ── Page ─────────────────────────────────────────────────────────────────────

#[component]
pub fn ScenariosPage() -> impl IntoView {
    let auth = use_context::<AuthState>().expect("AuthState missing");
    let scenarios = RwSignal::new(Vec::<Scenario>::new());
    let positions = RwSignal::new(Vec::<Position>::new());
    let loading = RwSignal::new(true);
    let show_new = RwSignal::new(false);

    let auth_for_load = auth.clone();
    Effect::new(move |_| {
        let token = auth_for_load.token.get();
        let user_id = auth_for_load.user_id.get();
        if let (Some(tok), Some(uid)) = (token, user_id) {
            let tok2 = tok.clone();
            let uid2 = uid.clone();
            spawn_local(async move {
                if let Ok(s) = supabase::fetch_scenarios(&tok, &uid).await {
                    scenarios.set(s);
                }
                if let Ok(p) = supabase::fetch_positions(&tok2, &uid2).await {
                    positions.set(p);
                }
                loading.set(false);
            });
        } else {
            loading.set(false);
        }
    });

    view! {
        <div class="space-y-6">
            <div class="flex items-center justify-between">
                <h1 class="text-xl font-semibold">"Scenarios"</h1>
                <button
                    class="bg-blue-600 hover:bg-blue-500 px-4 py-2 rounded text-sm font-medium transition-colors"
                    on:click=move |_| show_new.update(|v| *v = !*v)
                >
                    {move || if show_new.get() { "Cancel" } else { "+ New scenario" }}
                </button>
            </div>

            {move || show_new.get().then(|| {
                let auth2 = auth.clone();
                view! {
                    <NewScenarioForm
                        auth=auth2
                        on_created=move |s: Scenario| {
                            scenarios.update(|ss| ss.insert(0, s));
                            show_new.set(false);
                        }
                    />
                }
            })}

            {move || loading.get().then(|| view! {
                <p class="text-gray-400 text-sm">"Loading…"</p>
            })}

            {move || scenarios.get().into_iter().map(|s| {
                let ps = positions.get();
                let result = evaluate_scenario(&ps, &s);
                view! { <ScenarioCard scenario=s result=result /> }
            }).collect_view()}
        </div>
    }
}

// ── New scenario form ─────────────────────────────────────────────────────────

#[component]
fn NewScenarioForm(
    auth: AuthState,
    on_created: impl Fn(Scenario) + 'static,
) -> impl IntoView {
    let on_created = Rc::new(on_created);
    let name = RwSignal::new(String::new());
    let eval_date = RwSignal::new(String::new());
    let assumptions = RwSignal::new(vec![
        (RwSignal::new(String::new()), RwSignal::new(String::new())),
    ]);
    let err = RwSignal::new(Option::<String>::None);
    let saving = RwSignal::new(false);

    let add_assumption = move |_: web_sys::MouseEvent| {
        assumptions.update(|v| {
            v.push((RwSignal::new(String::new()), RwSignal::new(String::new())));
        });
    };

    let on_submit = move |ev: web_sys::SubmitEvent| {
        ev.prevent_default();
        let n = name.get().trim().to_string();
        if n.is_empty() { err.set(Some("Name required.".into())); return; }
        let ed = match NaiveDate::parse_from_str(eval_date.get().trim(), "%Y-%m-%d") {
            Ok(d) => d,
            Err(_) => { err.set(Some("Evaluation date must be YYYY-MM-DD.".into())); return; }
        };

        let mut scenario = Scenario::new(&n, ed);
        for (sym_sig, price_sig) in assumptions.get() {
            let sym = sym_sig.get().trim().to_uppercase();
            let price: f64 = match price_sig.get().trim().parse() {
                Ok(p) => p,
                Err(_) => { err.set(Some(format!("Invalid price for {}", sym))); return; }
            };
            if !sym.is_empty() {
                scenario.price_assumptions.push(crate::models::scenario::PriceAssumption {
                    symbol: sym,
                    assumed_price: price,
                });
            }
        }

        saving.set(true);
        let token = auth.token.get().unwrap_or_default();
        let user_id = auth.user_id.get().unwrap_or_default();
        let s = scenario.clone();
        let cb = Rc::clone(&on_created);
        spawn_local(async move {
            match supabase::upsert_scenario(&token, &user_id, &s).await {
                Ok(_) => cb(s),
                Err(e) => { err.set(Some(e)); saving.set(false); }
            }
        });
    };

    view! {
        <form on:submit=on_submit class="bg-panel border border-border rounded-xl p-6 space-y-4">
            <h2 class="text-sm font-medium text-gray-300">"New scenario"</h2>

            <div class="grid grid-cols-2 gap-3">
                <div>
                    <label class="block text-xs text-gray-400 mb-1">"Name"</label>
                    <input
                        class="w-full bg-surface border border-border rounded px-3 py-1.5 text-sm focus:outline-none focus:border-blue-500"
                        prop:value=move || name.get()
                        on:input=move |ev| name.set(event_target_value(&ev))
                        placeholder="e.g. Bull case Q4"
                    />
                </div>
                <div>
                    <label class="block text-xs text-gray-400 mb-1">"Evaluation date (YYYY-MM-DD)"</label>
                    <input
                        class="w-full bg-surface border border-border rounded px-3 py-1.5 text-sm focus:outline-none focus:border-blue-500"
                        prop:value=move || eval_date.get()
                        on:input=move |ev| eval_date.set(event_target_value(&ev))
                        placeholder="2025-06-30"
                    />
                </div>
            </div>

            <div class="space-y-2">
                <p class="text-xs text-gray-400">"Price assumptions"</p>
                {move || assumptions.get().into_iter().map(|(sym, price)| view! {
                    <div class="flex gap-2">
                        <input
                            class="flex-1 bg-surface border border-border rounded px-3 py-1.5 text-sm focus:outline-none focus:border-blue-500"
                            prop:value=move || sym.get()
                            on:input=move |ev| sym.set(event_target_value(&ev))
                            placeholder="AAPL"
                        />
                        <input
                            class="flex-1 bg-surface border border-border rounded px-3 py-1.5 text-sm focus:outline-none focus:border-blue-500"
                            prop:value=move || price.get()
                            on:input=move |ev| price.set(event_target_value(&ev))
                            placeholder="200.00"
                        />
                    </div>
                }).collect_view()}
                <button
                    type="button"
                    class="text-xs text-blue-400 hover:text-blue-300"
                    on:click=add_assumption
                >
                    "+ add symbol"
                </button>
            </div>

            {move || err.get().map(|e| view! { <p class="text-red-400 text-xs">{e}</p> })}

            <button
                type="submit"
                class="bg-blue-600 hover:bg-blue-500 disabled:opacity-50 px-4 py-2 rounded text-sm font-medium transition-colors"
                prop:disabled=move || saving.get()
            >
                {move || if saving.get() { "Saving…" } else { "Create" }}
            </button>
        </form>
    }
}

// ── Scenario result card ──────────────────────────────────────────────────────

#[component]
fn ScenarioCard(scenario: Scenario, result: ScenarioResult) -> impl IntoView {
    let st_rate = 0.37_f64;
    let lt_rate = 0.20_f64;
    let tax = result.total_tax_estimate(st_rate, lt_rate);
    let after_tax = result.total_pnl - tax;

    let pnl_class = if result.total_pnl >= 0.0 { "text-green-400" } else { "text-red-400" };
    let at_class = if after_tax >= 0.0 { "text-green-300" } else { "text-red-300" };

    view! {
        <div class="bg-panel border border-border rounded-xl p-6 space-y-4">
            <div class="flex items-start justify-between">
                <div>
                    <h2 class="font-medium">{scenario.name.clone()}</h2>
                    <p class="text-xs text-gray-500 mt-0.5">
                        "Eval date: " {scenario.evaluation_date.format("%d %b %Y").to_string()}
                    </p>
                </div>
                <div class="text-right">
                    <p class=format!("text-lg font-semibold {}", pnl_class)>
                        {format!("{}{:.2}", if result.total_pnl >= 0.0 { "+" } else { "" }, result.total_pnl)}
                    </p>
                    <p class="text-xs text-gray-500">"total P&L"</p>
                </div>
            </div>

            <div class="space-y-1">
                {result.legs.iter().map(|leg| {
                    let lc = if leg.pnl >= 0.0 { "text-green-400" } else { "text-red-400" };
                    view! {
                        <div class="flex justify-between text-xs">
                            <span class="text-gray-400">{leg.description.clone()}</span>
                            <span class=lc>
                                {format!("{}{:.2}", if leg.pnl >= 0.0 { "+" } else { "" }, leg.pnl)}
                            </span>
                        </div>
                    }
                }).collect_view()}
            </div>

            <div class="border-t border-border pt-3 grid grid-cols-3 gap-2 text-xs">
                <div>
                    <p class="text-gray-500">"ST gain (37%)"</p>
                    <p class="text-yellow-300">{format!("${:.2}", result.total_short_term)}</p>
                </div>
                <div>
                    <p class="text-gray-500">"LT gain (20%)"</p>
                    <p class="text-blue-300">{format!("${:.2}", result.total_long_term)}</p>
                </div>
                <div>
                    <p class="text-gray-500">"Est. tax"</p>
                    <p class="text-orange-300">{format!("${:.2}", tax)}</p>
                </div>
            </div>
            <div class="flex justify-between items-center text-sm">
                <span class="text-gray-400">"After-tax P&L"</span>
                <span class=format!("font-semibold {}", at_class)>
                    {format!("{}{:.2}", if after_tax >= 0.0 { "+" } else { "" }, after_tax)}
                </span>
            </div>
        </div>
    }
}
