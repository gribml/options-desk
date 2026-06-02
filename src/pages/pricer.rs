use leptos::prelude::*;

use crate::models::option::OptionType;
use crate::pricing::black_scholes::{implied_vol, BsInputs};

fn parse_f64(s: &str) -> Option<f64> {
    let v: f64 = s.trim().parse().ok()?;
    if v.is_finite() && v > 0.0 { Some(v) } else { None }
}

#[component]
pub fn PricerPage() -> impl IntoView {
    // Inputs
    let spot = RwSignal::new("100".to_string());
    let strike = RwSignal::new("100".to_string());
    let vol = RwSignal::new("25".to_string()); // as percent
    let rate = RwSignal::new("3.75".to_string()); // as percent
    let expiry_days = RwSignal::new("30".to_string());
    let opt_type = RwSignal::new(OptionType::Call);
    let market_price = RwSignal::new("".to_string()); // for IV calc

    let result = Memo::new(move |_| {
        let inputs = BsInputs {
            spot: parse_f64(&spot.get())?,
            strike: parse_f64(&strike.get())?,
            vol: parse_f64(&vol.get())? / 100.0,
            rate: parse_f64(&rate.get())? / 100.0,
            expiry_years: parse_f64(&expiry_days.get())? / 365.0,
        };
        Some(inputs.greeks(opt_type.get()))
    });

    let iv = Memo::new(move |_| {
        let mp: f64 = market_price.get().trim().parse().ok()?;
        let inputs = BsInputs {
            spot: parse_f64(&spot.get())?,
            strike: parse_f64(&strike.get())?,
            vol: 0.25, // seed — irrelevant, bisection finds the answer
            rate: parse_f64(&rate.get())? / 100.0,
            expiry_years: parse_f64(&expiry_days.get())? / 365.0,
        };
        implied_vol(mp, inputs, opt_type.get())
    });

    view! {
        <div class="max-w-2xl mx-auto space-y-8">
            <h1 class="text-xl font-semibold">"Black-Scholes Pricer"</h1>

            // ── Option type toggle ─────────────────────────────────────────
            <div class="flex gap-2">
                {[OptionType::Call, OptionType::Put].map(|t| {
                    let label = match t { OptionType::Call => "Call", OptionType::Put => "Put" };
                    view! {
                        <button
                            class=move || {
                                let active = opt_type.get() == t;
                                format!(
                                    "px-6 py-2 rounded text-sm font-medium border transition-colors {}",
                                    if active {
                                        "bg-blue-600 border-blue-600 text-white"
                                    } else {
                                        "bg-panel border-border text-gray-400 hover:border-gray-500"
                                    }
                                )
                            }
                            on:click=move |_| opt_type.set(t)
                        >
                            {label}
                        </button>
                    }
                })}
            </div>

            // ── Inputs grid ───────────────────────────────────────────────
            <div class="grid grid-cols-2 gap-4">
                <InputField label="Spot (S)" signal=spot placeholder="100" />
                <InputField label="Strike (K)" signal=strike placeholder="100" />
                <InputField label="Volatility (%)" signal=vol placeholder="25" />
                <InputField label="Risk-free rate (%)" signal=rate placeholder="3.75" />
                <InputField label="Days to expiry" signal=expiry_days placeholder="30" />
            </div>

            // ── Results ───────────────────────────────────────────────────
            {move || result.get().map(|r| view! {
                <div class="bg-panel border border-border rounded-xl p-6 grid grid-cols-2 gap-4">
                    <GreekRow label="Price"  value=format!("{:.4}", r.price) />
                    <GreekRow label="Delta"  value=format!("{:.4}", r.delta) />
                    <GreekRow label="Gamma"  value=format!("{:.4}", r.gamma) />
                    <GreekRow label="Vega (per vol pt)" value=format!("{:.4}", r.vega) />
                    <GreekRow label="Theta (per day)"   value=format!("{:.4}", r.theta) />
                    <GreekRow label="Rho (per 1% rate)" value=format!("{:.4}", r.rho) />
                </div>
            })}

            // ── Implied vol calculator ─────────────────────────────────────
            <div class="bg-panel border border-border rounded-xl p-6 space-y-4">
                <h2 class="text-sm font-medium text-gray-300">"Implied Volatility Calculator"</h2>
                <div class="flex gap-3 items-end">
                    <div class="flex-1">
                        <label class="block text-xs text-gray-400 mb-1">"Market price"</label>
                        <input
                            class="w-full bg-surface border border-border rounded px-3 py-2 text-sm focus:outline-none focus:border-blue-500"
                            prop:value=move || market_price.get()
                            on:input=move |ev| market_price.set(event_target_value(&ev))
                            placeholder="e.g. 3.50"
                        />
                    </div>
                </div>
                {move || iv.get().map(|v| view! {
                    <p class="text-blue-300 text-sm">
                        "Implied vol: " <span class="font-semibold">{format!("{:.2}%", v * 100.0)}</span>
                    </p>
                })}
                {move || {
                    if market_price.get().trim().is_empty() {
                        None
                    } else if iv.get().is_none() {
                        Some(view! { <p class="text-yellow-500 text-sm">"Could not converge — check inputs."</p> })
                    } else {
                        None
                    }
                }}
            </div>
        </div>
    }
}

#[component]
fn InputField(label: &'static str, signal: RwSignal<String>, placeholder: &'static str) -> impl IntoView {
    view! {
        <div>
            <label class="block text-xs text-gray-400 mb-1">{label}</label>
            <input
                class="w-full bg-surface border border-border rounded px-3 py-2 text-sm focus:outline-none focus:border-blue-500"
                prop:value=move || signal.get()
                on:input=move |ev| signal.set(event_target_value(&ev))
                placeholder=placeholder
            />
        </div>
    }
}

#[component]
fn GreekRow(label: &'static str, value: String) -> impl IntoView {
    view! {
        <div class="flex justify-between items-center border-b border-border pb-2">
            <span class="text-xs text-gray-400">{label}</span>
            <span class="text-sm font-medium text-blue-200">{value}</span>
        </div>
    }
}
