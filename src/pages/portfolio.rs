use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use chrono::NaiveDate;
use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

use crate::api::{market, supabase};
use crate::app::AuthState;
use crate::format::{fmt_cash, Num};
use crate::models::market::OptionMetaEntry;
use crate::store::MarketStore;
use crate::models::{
    option::{OptionSpec, OptionType},
    position::{Position, PositionKind},
};
use crate::pricing::black_scholes::BsInputs;

// ── Market data ───────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq)]
struct MarketData {
    price: String,
    vol: String,
    rate: String,
    change: Option<f64>,
    change_pct: Option<f64>,
}

impl Default for MarketData {
    fn default() -> Self {
        Self {
            price: String::new(),
            vol: "25".to_string(),
            rate: "3.75".to_string(),
            change: None,
            change_pct: None,
        }
    }
}

impl MarketData {
    fn parsed_price(&self) -> Option<f64> {
        self.price.parse::<f64>().ok().filter(|&v| v > 0.0)
    }
    fn parsed_vol(&self) -> Option<f64> {
        self.vol.parse::<f64>().ok().filter(|&v| v > 0.0).map(|v| v / 100.0)
    }
    fn parsed_rate(&self) -> f64 {
        self.rate.parse::<f64>().unwrap_or(3.75) / 100.0
    }
}

// ── Per-position computed metrics ─────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq)]
struct PositionMetrics {
    mark_price: f64,
    mark_value: f64,
    pnl: f64,
    delta: f64,
    gamma: f64,
    vega: f64,
    theta: f64,
    rho: f64,
}

fn compute_metrics(pos: &Position, md: &MarketData) -> Option<PositionMetrics> {
    let price = md.parsed_price()?;
    let qty = pos.quantity as f64;

    match &pos.kind {
        PositionKind::Stock => Some(PositionMetrics {
            mark_price: price,
            mark_value: price * qty,
            pnl: (price - pos.cost_basis) * qty,
            delta: qty,
            gamma: 0.0,
            vega: 0.0,
            theta: 0.0,
            rho: 0.0,
        }),
        PositionKind::Option => {
            let spec = pos.option_spec.as_ref()?;
            let t = spec.years_to_expiry();
            let mult = qty * 100.0;

            if t <= 0.0 {
                let intrinsic = match spec.option_type {
                    OptionType::Call => (price - spec.strike).max(0.0),
                    OptionType::Put => (spec.strike - price).max(0.0),
                };
                return Some(PositionMetrics {
                    mark_price: intrinsic,
                    mark_value: intrinsic * mult,
                    pnl: (intrinsic - pos.cost_basis) * mult,
                    delta: 0.0,
                    gamma: 0.0,
                    vega: 0.0,
                    theta: 0.0,
                    rho: 0.0,
                });
            }

            let vol = md.parsed_vol()?;
            let rate = md.parsed_rate();
            let g = BsInputs { spot: price, strike: spec.strike, expiry_years: t, vol, rate }
                .greeks(spec.option_type);

            let mark_price = g.price;
            Some(PositionMetrics {
                mark_price,
                mark_value: mark_price * mult,
                pnl: (mark_price - pos.cost_basis) * mult,
                delta: g.delta * mult,
                gamma: g.gamma * mult,
                vega: g.vega * mult,
                theta: g.theta * mult,
                rho: g.rho * mult,
            })
        }
    }
}

// ── Portfolio summary ─────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Default)]
struct PortfolioSummary {
    total_value: f64,
    total_pnl: f64,
    net_delta: f64,
    net_gamma: f64,
    net_vega: f64,
    net_theta: f64,
    net_rho: f64,
    has_data: bool,
}

fn summarize(positions: &[Position], metrics: &[Option<PositionMetrics>]) -> PortfolioSummary {
    let _ = positions;
    let mut s = PortfolioSummary::default();
    for m in metrics.iter().flatten() {
        s.has_data = true;
        s.total_value += m.mark_value;
        s.total_pnl += m.pnl;
        s.net_delta += m.delta;
        s.net_gamma += m.gamma;
        s.net_vega += m.vega;
        s.net_theta += m.theta;
        s.net_rho += m.rho;
    }
    s
}

// ── Page ─────────────────────────────────────────────────────────────────────

#[component]
pub fn PortfolioPage() -> impl IntoView {
    let auth = use_context::<AuthState>().expect("AuthState missing");
    let store = use_context::<MarketStore>().expect("MarketStore missing");
    let positions = RwSignal::new(Vec::<Position>::new());
    let loading = RwSignal::new(true);
    let error = RwSignal::new(Option::<String>::None);
    let show_add = RwSignal::new(false);
    let market_data = RwSignal::new(HashMap::<String, MarketData>::new());
    let quote_loading = RwSignal::new(false);

    // Apply quotes to market_data (shared by both initial load and refresh).
    let apply_quotes = move |quotes: Vec<crate::models::market::Quote>| {
        market_data.update(|map| {
            for q in quotes {
                let md = map.entry(q.symbol.clone()).or_default();
                md.price = format!("{:.2}", q.price);
                md.change = Some(q.change);
                md.change_pct = Some(q.change_pct);
            }
        });
    };

    // Load positions, then auto-fetch stock quotes + actual option prices.
    Effect::new(move |_| {
        let token = auth.token.get();
        let user_id = auth.user_id.get();
        if let (Some(tok), Some(uid)) = (token, user_id) {
            spawn_local(async move {
                match supabase::fetch_positions(&tok, &uid).await {
                    Ok(ps) => {
                        let syms: Vec<String> = ps.iter()
                            .map(|p| p.symbol.clone())
                            .collect::<HashSet<_>>()
                            .into_iter()
                            .collect();
                        let option_infos: Vec<(String, String, &'static str, f64)> = ps.iter()
                            .filter_map(|p| {
                                let spec = p.option_spec.as_ref()?;
                                Some((
                                    p.symbol.clone(),
                                    spec.expiry.format("%Y-%m-%d").to_string(),
                                    match spec.option_type {
                                        OptionType::Call => "call",
                                        OptionType::Put  => "put",
                                    },
                                    spec.strike,
                                ))
                            })
                            .collect();

                        positions.set(ps);

                        if !syms.is_empty() {
                            // Apply any already-cached quotes immediately.
                            let cached: Vec<_> = syms.iter()
                                .filter_map(|s| store.quotes.get_untracked().get(s).cloned())
                                .collect();
                            if !cached.is_empty() { apply_quotes(cached); }

                            // Fetch only the symbols not yet in the cache.
                            let missing: Vec<String> = syms.iter()
                                .filter(|s| !store.quotes.get_untracked().contains_key(*s))
                                .cloned()
                                .collect();
                            if !missing.is_empty() {
                                quote_loading.set(true);
                                let fetched = market::fetch_quotes(&tok, &missing).await;
                                store.quotes.update(|map| {
                                    for q in &fetched { map.insert(q.symbol.clone(), q.clone()); }
                                });
                                apply_quotes(fetched);
                                quote_loading.set(false);
                            }
                        }

                        // Fetch each option contract's IV to seed market_data.vol for B-S Greeks.
                        for (sym, expiry, opt_type, strike) in option_infos {
                            let tok2 = tok.clone();
                            let sym2 = sym.clone();
                            spawn_local(async move {
                                if let Ok(oq) = market::fetch_option_quote(&tok2, &sym2, &expiry, opt_type, strike).await {
                                    if let Some(iv) = oq.implied_vol {
                                        market_data.update(|map| {
                                            if let Some(md) = map.get_mut(&sym2) {
                                                md.vol = format!("{:.1}", iv * 100.0);
                                            }
                                        });
                                    }
                                }
                            });
                        }
                    }
                    Err(e) => error.set(Some(e)),
                }
                loading.set(false);
            });
        } else {
            loading.set(false);
        }
    });

    // Keep market_data keys in sync with position symbols.
    Effect::new(move |_| {
        let ps = positions.get();
        market_data.update(|map| {
            for p in &ps {
                map.entry(p.symbol.clone()).or_default();
            }
        });
    });

    let metrics = Memo::new(move |_| {
        let ps = positions.get();
        let md = market_data.get();
        let empty = MarketData::default();
        ps.iter()
            .map(|p| compute_metrics(p, md.get(&p.symbol).unwrap_or(&empty)))
            .collect::<Vec<_>>()
    });

    let summary = Memo::new(move |_| summarize(&positions.get(), &metrics.get()));

    // Refresh live quotes + option prices on demand.
    let refresh_quotes = move || {
        let token = auth.token.get_untracked().unwrap_or_default();
        let ps = positions.get_untracked();
        let syms: Vec<String> = ps.iter()
            .map(|p| p.symbol.clone())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        if syms.is_empty() { return; }

        quote_loading.set(true);
        let tok1 = token.clone();
        spawn_local(async move {
            let quotes = market::fetch_quotes(&tok1, &syms).await;
            store.quotes.update(|map| {
                for q in &quotes { map.insert(q.symbol.clone(), q.clone()); }
            });
            apply_quotes(quotes);
            quote_loading.set(false);
        });

        for p in ps {
            if let Some(spec) = p.option_spec {
                let tok2 = token.clone();
                let sym = p.symbol.clone();
                let expiry = spec.expiry.format("%Y-%m-%d").to_string();
                let opt_type = match spec.option_type {
                    OptionType::Call => "call",
                    OptionType::Put  => "put",
                };
                let strike = spec.strike;
                spawn_local(async move {
                    if let Ok(oq) = market::fetch_option_quote(&tok2, &sym, &expiry, opt_type, strike).await {
                        if let Some(iv) = oq.implied_vol {
                            market_data.update(|map| {
                                if let Some(md) = map.get_mut(&sym) {
                                    md.vol = format!("{:.1}", iv * 100.0);
                                }
                            });
                        }
                    }
                });
            }
        }
    };

    view! {
        <div class="space-y-6">

            // ── Header ─────────────────────────────────────────────────────
            <div class="flex items-center justify-between">
                <h1 class="text-xl font-semibold">"Portfolio"</h1>
                <button
                    class="bg-blue-600 hover:bg-blue-500 px-4 py-2 rounded text-sm font-medium transition-colors"
                    on:click=move |_| show_add.update(|v| *v = !*v)
                >
                    {move || if show_add.get() { "Cancel" } else { "+ Add position" }}
                </button>
            </div>

            {move || show_add.get().then(|| view! {
                <AddPositionForm
                    auth=auth
                    on_added=move |p: Position| {
                        positions.update(|ps| ps.push(p));
                        show_add.set(false);
                    }
                />
            })}

            {move || error.get().map(|e| view! { <p class="text-red-400 text-sm">{e}</p> })}
            {move || loading.get().then(|| view! { <p class="text-gray-400 text-sm">"Loading…"</p> })}

            // ── Market inputs (per symbol) ──────────────────────────────────
            {move || {
                let ps = positions.get();
                if ps.is_empty() { return None; }
                let mut seen = HashSet::new();
                let syms: Vec<String> = ps
                    .iter()
                    .filter(|p| seen.insert(p.symbol.clone()))
                    .map(|p| p.symbol.clone())
                    .collect();
                Some(view! {
                    <MarketInputsPanel
                        symbols=syms
                        market_data=market_data
                        quote_loading=quote_loading
                        on_refresh_quotes=move || refresh_quotes()
                    />
                })
            }}

            // ── Portfolio summary ───────────────────────────────────────────
            {move || summary.get().has_data.then(|| view! {
                <SummaryCard summary=summary.get() />
            })}

            // ── Position rows ───────────────────────────────────────────────
            {move || (!loading.get() && positions.get().is_empty()).then(|| view! {
                <p class="text-gray-500 text-sm">"No positions yet. Add one above."</p>
            })}

            <div class="space-y-2">
                {move || {
                    positions.get().into_iter()
                        .zip(metrics.get().into_iter())
                        .map(|(p, m)| {
                            let id = p.id;
                            view! {
                                <PositionRow
                                    position=p
                                    metrics=m
                                    on_delete=move || {
                                        let token = auth.token.get().unwrap_or_default();
                                        spawn_local(async move {
                                            if supabase::delete_position(&token, &id.to_string()).await.is_ok() {
                                                positions.update(|ps| ps.retain(|p| p.id != id));
                                            }
                                        });
                                    }
                                />
                            }
                        })
                        .collect_view()
                }}
            </div>
        </div>
    }
}

// ── Market inputs panel ───────────────────────────────────────────────────────

#[component]
fn MarketInputsPanel(
    symbols: Vec<String>,
    market_data: RwSignal<HashMap<String, MarketData>>,
    quote_loading: RwSignal<bool>,
    on_refresh_quotes: impl Fn() + 'static,
) -> impl IntoView {
    view! {
        <div class="bg-panel border border-border rounded-xl p-4 space-y-3">
            <div class="flex items-center justify-between">
                <p class="text-xs font-medium text-gray-400 uppercase tracking-wider">"Market Inputs"</p>
                <button
                    class="text-xs text-gray-500 hover:text-blue-300 disabled:opacity-40 transition-colors"
                    prop:disabled=move || quote_loading.get()
                    on:click=move |_| on_refresh_quotes()
                >
                    {move || if quote_loading.get() { "Refreshing…" } else { "↻ Refresh quotes" }}
                </button>
            </div>

            <div class="grid grid-cols-[auto_1fr_1fr_1fr] gap-x-4 gap-y-2 items-center">
                <span class="text-xs text-gray-500">"Symbol"</span>
                <span class="text-xs text-gray-500">"Price"</span>
                <span class="text-xs text-gray-500">"IV %"</span>
                <span class="text-xs text-gray-500">"Rate %"</span>

                {symbols.into_iter().map(|sym| {
                    let (sp, sv, sr, sc) = (sym.clone(), sym.clone(), sym.clone(), sym.clone());
                    let (wp, wv, wr) = (sym.clone(), sym.clone(), sym.clone());
                    let (kp, kv, kr) = (sym.clone(), sym.clone(), sym.clone());
                    view! {
                        // Symbol + live change badge
                        <span class="text-sm font-semibold flex items-center gap-1.5">
                            {sym.clone()}
                            {move || {
                                market_data.get().get(&sc).and_then(|md| {
                                    let pct = md.change_pct?;
                                    let cls = if pct >= 0.0 { "text-green-400" } else { "text-red-400" };
                                    let arrow = if pct >= 0.0 { "▲" } else { "▼" };
                                    Some(view! {
                                        <span class=format!("text-xs font-normal {}", cls)>
                                            {format!("{}{:.2}%", arrow, pct.abs())}
                                        </span>
                                    })
                                })
                            }}
                        </span>
                        <input
                            class="bg-surface border border-border rounded px-2 py-1 text-sm focus:outline-none focus:border-blue-500 w-full"
                            prop:value=move || market_data.get().get(&sp).map(|m| m.price.clone()).unwrap_or_default()
                            on:input=move |ev| {
                                let v = event_target_value(&ev);
                                market_data.update(|map| { if let Some(m) = map.get_mut(&wp) { m.price = v; } });
                            }
                            on:keydown=move |ev| {
                                let key = ev.key();
                                if key != "ArrowUp" && key != "ArrowDown" { return; }
                                ev.prevent_default();
                                let dir = if key == "ArrowUp" { 1.0_f64 } else { -1.0_f64 };
                                market_data.update(|map| {
                                    if let Some(m) = map.get_mut(&kp) {
                                        if let Ok(p) = m.price.parse::<f64>() {
                                            m.price = format!("{:.2}", (p * (1.0 + dir * 0.01)).max(0.0));
                                        }
                                    }
                                });
                            }
                            placeholder="155.00"
                        />
                        <input
                            class="bg-surface border border-border rounded px-2 py-1 text-sm focus:outline-none focus:border-blue-500 w-full"
                            prop:value=move || market_data.get().get(&sv).map(|m| m.vol.clone()).unwrap_or_default()
                            on:input=move |ev| {
                                let v = event_target_value(&ev);
                                market_data.update(|map| { if let Some(m) = map.get_mut(&wv) { m.vol = v; } });
                            }
                            on:keydown=move |ev| {
                                let key = ev.key();
                                if key != "ArrowUp" && key != "ArrowDown" { return; }
                                ev.prevent_default();
                                let dir = if key == "ArrowUp" { 1.0_f64 } else { -1.0_f64 };
                                market_data.update(|map| {
                                    if let Some(m) = map.get_mut(&kv) {
                                        if let Ok(v) = m.vol.parse::<f64>() {
                                            m.vol = format!("{:.2}", (v + dir).max(0.0));
                                        }
                                    }
                                });
                            }
                            placeholder="25"
                        />
                        <input
                            class="bg-surface border border-border rounded px-2 py-1 text-sm focus:outline-none focus:border-blue-500 w-full"
                            prop:value=move || market_data.get().get(&sr).map(|m| m.rate.clone()).unwrap_or_default()
                            on:input=move |ev| {
                                let v = event_target_value(&ev);
                                market_data.update(|map| { if let Some(m) = map.get_mut(&wr) { m.rate = v; } });
                            }
                            on:keydown=move |ev| {
                                let key = ev.key();
                                if key != "ArrowUp" && key != "ArrowDown" { return; }
                                ev.prevent_default();
                                let dir = if key == "ArrowUp" { 1.0_f64 } else { -1.0_f64 };
                                market_data.update(|map| {
                                    if let Some(m) = map.get_mut(&kr) {
                                        if let Ok(r) = m.rate.parse::<f64>() {
                                            m.rate = format!("{:.2}", r + dir * 0.1);
                                        }
                                    }
                                });
                            }
                            placeholder="3.75"
                        />
                    }
                }).collect_view()}
            </div>
        </div>
    }
}

// ── Portfolio summary card ────────────────────────────────────────────────────

#[component]
fn SummaryCard(summary: PortfolioSummary) -> impl IntoView {
    let pnl_class = if summary.total_pnl >= 0.0 { "text-green-400" } else { "text-red-400" };

    view! {
        <div class="bg-panel border border-blue-900 rounded-xl p-6 space-y-4">
            <div class="flex items-start justify-between">
                <div>
                    <p class="text-xs text-gray-400 uppercase tracking-wider mb-1">"Portfolio Value"</p>
                    <p class="text-3xl font-semibold">
                        <Num value=summary.total_value />
                    </p>
                </div>
                <div class="text-right">
                    <p class="text-xs text-gray-400 uppercase tracking-wider mb-1">"Unrealised P&L"</p>
                    <p class=format!("text-2xl font-semibold {}", pnl_class)>
                        <Num value=summary.total_pnl signed=true />
                    </p>
                </div>
            </div>

            <div class="border-t border-border pt-4 grid grid-cols-5 gap-3">
                <GreekStat label="Delta"  value=summary.net_delta   fmt="{:.1}" />
                <GreekStat label="Gamma"  value=summary.net_gamma   fmt="{:.4}" />
                <GreekStat label="Vega"   value=summary.net_vega    fmt="${:.2}" />
                <GreekStat label="Theta"  value=summary.net_theta   fmt="${:.2}" />
                <GreekStat label="Rho"    value=summary.net_rho     fmt="${:.2}" />
            </div>
        </div>
    }
}

#[component]
fn GreekStat(label: &'static str, value: f64, fmt: &'static str) -> impl IntoView {
    let display = if fmt.starts_with('$') {
        fmt_cash(value)
    } else if fmt.contains(".4") {
        format!("{:+.4}", value)
    } else {
        format!("{:+.1}", value)
    };
    let cls = if value >= 0.0 { "text-green-300" } else { "text-red-300" };
    view! {
        <div class="text-center">
            <p class="text-xs text-gray-500 mb-1">{label}</p>
            <p class=format!("text-sm font-mono font-medium {}", cls)>{display}</p>
        </div>
    }
}

// ── Position row ──────────────────────────────────────────────────────────────

#[component]
fn PositionRow(
    position: Position,
    metrics: Option<PositionMetrics>,
    on_delete: impl Fn() + 'static,
) -> impl IntoView {
    let is_option = position.kind == PositionKind::Option;

    let kind_label = match &position.kind {
        PositionKind::Stock => "Stock".to_string(),
        PositionKind::Option => position.option_spec.as_ref().map(|s| {
            format!("{} ${:.0} {}", s.option_type.label(), s.strike, s.expiry.format("%d-%b-%y"))
        }).unwrap_or_else(|| "Option".to_string()),
    };

    let qty_class = if position.quantity >= 0 { "text-green-400" } else { "text-red-400" };

    let (mark_str, pnl_class) = match &metrics {
        Some(m) => (format!("${:.2}", m.mark_price), if m.pnl >= 0.0 { "text-green-400" } else { "text-red-400" }),
        None => ("—".into(), "text-gray-500"),
    };
    let mark_value = metrics.as_ref().map(|m| m.mark_value);
    let pnl_value  = metrics.as_ref().map(|m| m.pnl);

    view! {
        <div class="bg-panel border border-border rounded-lg p-3 space-y-2">
            <div class="flex items-center justify-between gap-4">
                <div class="flex items-center gap-4 min-w-0">
                    <span class="font-semibold text-sm w-14 shrink-0">{position.symbol.clone()}</span>
                    <span class="text-xs text-gray-400 truncate">{kind_label}</span>
                    <span class=format!("text-sm font-mono shrink-0 {}", qty_class)>
                        {format!("{:+}", position.quantity)}
                    </span>
                    <span class="text-xs text-gray-500 shrink-0">
                        "cost " {format!("${:.2}", position.cost_basis)}
                    </span>
                </div>
                <div class="flex items-center gap-4 shrink-0">
                    <span class="text-xs text-gray-400">"mark " {mark_str}</span>
                    <span class="text-sm font-medium">
                        {match mark_value {
                            Some(v) => view! { <Num value=v /> }.into_any(),
                            None    => "—".into_any(),
                        }}
                    </span>
                    <span class=format!("text-sm font-medium {}", pnl_class)>
                        {match pnl_value {
                            Some(v) => view! { <Num value=v signed=true /> }.into_any(),
                            None    => "—".into_any(),
                        }}
                    </span>
                    <button
                        class="text-gray-600 hover:text-red-400 text-xs transition-colors"
                        on:click=move |_| on_delete()
                    >
                        "✕"
                    </button>
                </div>
            </div>

            {metrics.as_ref().filter(|_| is_option).map(|m| {
                let (d, g, v, t, r) = (m.delta, m.gamma, m.vega, m.theta, m.rho);
                view! {
                    <div class="flex gap-4 pl-14 text-xs font-mono text-gray-400">
                        <span>"Δ " <GreekVal v=d fmt="f1" /></span>
                        <span>"Γ " <GreekVal v=g fmt="f4" /></span>
                        <span>"ν " <GreekVal v=v fmt="$" /></span>
                        <span>"θ " <GreekVal v=t fmt="$" /></span>
                        <span>"ρ " <GreekVal v=r fmt="$" /></span>
                    </div>
                }
            })}
        </div>
    }
}

#[component]
fn GreekVal(v: f64, fmt: &'static str) -> impl IntoView {
    let cls = if v >= 0.0 { "text-blue-300" } else { "text-orange-300" };
    let s = match fmt {
        "$"  => format!("{}{:.2}", if v >= 0.0 { "+$" } else { "-$" }, v.abs()),
        "f4" => format!("{:+.4}", v),
        _    => format!("{:+.1}", v),
    };
    view! { <span class=cls>{s}</span> }
}

// ── Add position form ─────────────────────────────────────────────────────────

#[component]
fn AddPositionForm(
    auth: AuthState,
    on_added: impl Fn(Position) + 'static,
) -> impl IntoView {
    let on_added = Rc::new(on_added);
    let symbol    = RwSignal::new(String::new());
    let kind      = RwSignal::new(PositionKind::Stock);
    let quantity  = RwSignal::new("1".to_string());
    let cost_basis = RwSignal::new(String::new());
    let opt_type    = RwSignal::new(OptionType::Call);
    let strike      = RwSignal::new(String::new());
    let expiry      = RwSignal::new(String::new());
    let err         = RwSignal::new(Option::<String>::None);
    let saving      = RwSignal::new(false);
    let option_meta = RwSignal::new(Vec::<OptionMetaEntry>::new());

    let store = use_context::<MarketStore>().expect("MarketStore missing");
    Effect::new(move |_| {
        let sym = symbol.get().trim().to_uppercase();
        if kind.get() == PositionKind::Option && !sym.is_empty() {
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

    let on_submit = move |ev: web_sys::SubmitEvent| {
        ev.prevent_default();
        err.set(None);

        let sym = symbol.get().trim().to_uppercase();
        if sym.is_empty() { err.set(Some("Symbol required.".into())); return; }

        let qty: i32 = match quantity.get().trim().parse() {
            Ok(v) => v,
            Err(_) => { err.set(Some("Invalid quantity.".into())); return; }
        };
        let cb: f64 = match cost_basis.get().trim().parse() {
            Ok(v) => v,
            Err(_) => { err.set(Some("Invalid cost basis.".into())); return; }
        };

        let position = match kind.get() {
            PositionKind::Stock => Position::new_stock(&sym, qty, cb),
            PositionKind::Option => {
                let s: f64 = match strike.get().trim().parse() {
                    Ok(v) => v,
                    Err(_) => { err.set(Some("Invalid strike.".into())); return; }
                };
                let exp = match NaiveDate::parse_from_str(expiry.get().trim(), "%Y-%m-%d") {
                    Ok(d) => d,
                    Err(_) => { err.set(Some("Expiry must be YYYY-MM-DD.".into())); return; }
                };
                Position::new_option(&sym, qty, cb, OptionSpec {
                    symbol: sym.clone(),
                    option_type: opt_type.get(),
                    strike: s,
                    expiry: exp,
                })
            }
        };

        saving.set(true);
        let token = auth.token.get().unwrap_or_default();
        let user_id = auth.user_id.get().unwrap_or_default();
        let pos = position.clone();
        let cb_fn = Rc::clone(&on_added);
        spawn_local(async move {
            match supabase::upsert_position(&token, &user_id, &pos).await {
                Ok(_) => cb_fn(pos),
                Err(e) => { err.set(Some(e)); saving.set(false); }
            }
        });
    };

    view! {
        <form on:submit=on_submit class="bg-panel border border-border rounded-xl p-6 space-y-4">
            <h2 class="text-sm font-medium text-gray-300">"Add position"</h2>

            <div class="flex gap-2">
                {[(PositionKind::Stock, "Stock"), (PositionKind::Option, "Option")].map(|(k, label)| {
                    let k2 = k.clone();
                    view! {
                        <button type="button"
                            class=move || format!(
                                "px-4 py-1 rounded text-xs border transition-colors {}",
                                if kind.get() == k2 { "bg-blue-600 border-blue-600 text-white" }
                                else { "bg-surface border-border text-gray-400" }
                            )
                            on:click={ let k3 = k.clone(); move |_| kind.set(k3.clone()) }
                        >{label}</button>
                    }
                })}
            </div>

            <div class="grid grid-cols-2 gap-3">
                <MiniInput label="Symbol"              signal=symbol    ph="AAPL" />
                <MiniInput label="Quantity (neg=short)" signal=quantity  ph="1" />
                <MiniInput label="Cost basis / share"  signal=cost_basis ph="0.00" />

                {move || (kind.get() == PositionKind::Option).then(|| {
                    let meta = option_meta.get();

                    let mut seen = std::collections::HashSet::new();
                    let mut expiries: Vec<String> = meta.iter()
                        .filter_map(|e| seen.insert(e.expiry.clone()).then_some(e.expiry.clone()))
                        .collect();
                    expiries.sort();

                    let type_str = if opt_type.get() == OptionType::Call { "call" } else { "put" };
                    let sel_exp = expiry.get();
                    let mut strikes: Vec<f64> = meta.iter()
                        .filter(|e| e.expiry == sel_exp && e.option_type == type_str)
                        .map(|e| e.strike)
                        .collect();
                    strikes.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

                    view! {
                        <>
                            <div class="col-span-2 flex gap-2">
                                {[OptionType::Call, OptionType::Put].map(|t| view! {
                                    <button type="button"
                                        class=move || format!(
                                            "px-4 py-1 rounded text-xs border transition-colors {}",
                                            if opt_type.get() == t { "bg-blue-600 border-blue-600 text-white" }
                                            else { "bg-surface border-border text-gray-400" }
                                        )
                                        on:click=move |_| opt_type.set(t)
                                    >{t.label()}</button>
                                })}
                            </div>
                            <div>
                                <label class="block text-xs text-gray-400 mb-1">"Expiry"</label>
                                <select
                                    class="w-full bg-surface border border-border rounded px-3 py-1.5 text-sm focus:outline-none focus:border-blue-500"
                                    prop:value=move || expiry.get()
                                    on:change=move |ev| {
                                        expiry.set(event_target_value(&ev));
                                        strike.set(String::new());
                                    }
                                >
                                    <option value="">"— select expiry —"</option>
                                    {expiries.into_iter().map(|exp| {
                                        view! { <option value=exp.clone()>{exp.clone()}</option> }
                                    }).collect_view()}
                                </select>
                            </div>
                            <div>
                                <label class="block text-xs text-gray-400 mb-1">"Strike"</label>
                                <select
                                    class="w-full bg-surface border border-border rounded px-3 py-1.5 text-sm focus:outline-none focus:border-blue-500"
                                    prop:value=move || strike.get()
                                    on:change=move |ev| strike.set(event_target_value(&ev))
                                >
                                    <option value="">"— select strike —"</option>
                                    {strikes.into_iter().map(|s| {
                                        let val = format!("{}", s);
                                        view! { <option value=val.clone()>{format!("${:.0}", s)}</option> }
                                    }).collect_view()}
                                </select>
                            </div>
                        </>
                    }
                })}
            </div>

            {move || err.get().map(|e| view! { <p class="text-red-400 text-xs">{e}</p> })}

            <button type="submit"
                class="bg-blue-600 hover:bg-blue-500 disabled:opacity-50 px-4 py-2 rounded text-sm font-medium"
                prop:disabled=move || saving.get()
            >
                {move || if saving.get() { "Saving…" } else { "Add" }}
            </button>
        </form>
    }
}

#[component]
fn MiniInput(label: &'static str, signal: RwSignal<String>, ph: &'static str) -> impl IntoView {
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
