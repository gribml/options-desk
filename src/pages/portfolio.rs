use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use chrono::{Datelike, NaiveDate, Utc};
use leptos::prelude::*;
use uuid::Uuid;
use wasm_bindgen_futures::spawn_local;

use crate::api::{market, supabase};
use crate::app::AuthState;
use crate::components::ui::{Callout, Disclosure, EmptyState, Hint, Info, Stat, Tone};
use crate::format::{fmt_cash, Num};
use crate::models::market::{LatestBar, OptionMetaEntry};
use crate::store::MarketStore;
use crate::models::{
    option::{OptionSpec, OptionType},
    position::{match_trades, LotAllocation, Position, PositionEntryMode, PositionKind, Trade},
};
use crate::pricing::black_scholes::BsInputs;

// ── Market data ───────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq)]
struct MarketData {
    price: String,
    vol: String,
    rate: String,
    ref_price: Option<f64>,
    change: Option<f64>,
    change_pct: Option<f64>,
}

impl Default for MarketData {
    fn default() -> Self {
        Self {
            price: String::new(),
            vol: "25".to_string(),
            rate: "3.75".to_string(),
            ref_price: None,
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
    let qty = pos.effective_quantity() as f64;
    let cb = pos.effective_cost_basis();

    match &pos.kind {
        PositionKind::Stock => Some(PositionMetrics {
            mark_price: price,
            mark_value: price * qty,
            pnl: (price - cb) * qty,
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
                    pnl: (intrinsic - cb) * mult,
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
                pnl: (mark_price - cb) * mult,
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
    // Symbols whose live quote fetch failed — surfaced in the UI so the user
    // knows to enter a price manually instead.
    let quote_errors = RwSignal::new(HashSet::<String>::new());

    // Apply latest bars to market_data (shared by both initial load and refresh).
    let apply_bars = move |bars: Vec<LatestBar>| {
        market_data.update(|map| {
            for b in bars {
                let md = map.entry(b.symbol.clone()).or_default();
                md.price = format!("{:.2}", b.close);
                md.ref_price = Some(b.close);
            }
        });
    };

    // Record which requested symbols came back (clear any error) vs failed to
    // return a quote (flag for manual entry).
    let mark_quote_results = move |requested: &[String], fetched: &[LatestBar]| {
        let got: HashSet<&str> = fetched.iter().map(|b| b.symbol.as_str()).collect();
        quote_errors.update(|errs| {
            for s in requested {
                if got.contains(s.as_str()) { errs.remove(s); } else { errs.insert(s.clone()); }
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
                            // Apply any already-cached bars immediately.
                            let cached: Vec<_> = syms.iter()
                                .filter_map(|s| store.quotes.get_untracked().get(s).cloned())
                                .collect();
                            if !cached.is_empty() { apply_bars(cached); }

                            // Fetch only the symbols not yet in the cache.
                            let missing: Vec<String> = syms.iter()
                                .filter(|s| !store.quotes.get_untracked().contains_key(*s))
                                .cloned()
                                .collect();
                            if !missing.is_empty() {
                                quote_loading.set(true);
                                let fetched = market::fetch_latest_bars(&tok, &missing).await;
                                store.quotes.update(|map| {
                                    for b in &fetched { map.insert(b.symbol.clone(), b.clone()); }
                                });
                                mark_quote_results(&missing, &fetched);
                                apply_bars(fetched);
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

    // Implied liquidation tax per position, computed in one batch by the Worker
    // against the user's current-year profile. Re-runs when marks change.
    let implied_tax = RwSignal::new(HashMap::<Uuid, f64>::new());
    let tax_year = Utc::now().year();
    Effect::new(move |_| {
        let token = match auth.token.get() { Some(t) => t, None => return };
        let ps = positions.get();
        let ms = metrics.get();
        let items: Vec<market::TaxItemRequest> = ps.iter().zip(ms.iter())
            .filter_map(|(p, m)| {
                let m = m.as_ref()?;
                // Full pnl (incl. losses) routed by holding period; the marginal
                // engine handles negatives — do not clamp.
                let is_lt = p.option_spec.is_none()
                    && (Utc::now().date_naive() - p.oldest_open_lot_date().date_naive()).num_days() > 365;
                Some(market::TaxItemRequest {
                    id: p.id.to_string(),
                    st_gain: if is_lt { 0.0 } else { m.pnl },
                    lt_gain: if is_lt { m.pnl } else { 0.0 },
                })
            })
            .collect();
        if items.is_empty() { return; }
        spawn_local(async move {
            if let Ok(batch) = market::estimate_portfolio_tax(&token, tax_year, items).await {
                let mut map = HashMap::new();
                for r in batch.results {
                    if let Ok(uid) = r.id.parse::<Uuid>() {
                        map.insert(uid, r.tax);
                    }
                }
                implied_tax.set(map);
            }
        });
    });

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
            let bars = market::fetch_latest_bars(&tok1, &syms).await;
            store.quotes.update(|map| {
                for b in &bars { map.insert(b.symbol.clone(), b.clone()); }
            });
            mark_quote_results(&syms, &bars);
            apply_bars(bars);
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
            <div class="flex items-start justify-between gap-4">
                <div>
                    <h1 class="text-xl font-semibold">"Portfolio"</h1>
                    <p class="text-xs text-gray-500 mt-1 font-sans">
                        "Everything you hold, what it's worth today, and what selling it would cost you in tax."
                    </p>
                </div>
                <button
                    class="bg-blue-600 hover:bg-blue-500 px-4 py-2 rounded text-sm font-medium transition-colors shrink-0"
                    on:click=move |_| show_add.update(|v| *v = !*v)
                >
                    {move || if show_add.get() { "Cancel" } else { "+ Add position" }}
                </button>
            </div>

            {move || show_add.get().then(|| view! {
                <AddPositionForm
                    auth=auth
                    on_added=move |p: Position| {
                        let sym = p.symbol.clone();
                        positions.update(|ps| ps.push(p));
                        show_add.set(false);

                        // Pull a live quote for the new symbol so its mark fills in
                        // without a page refresh. Serve from cache when we have it.
                        if let Some(cached) = store.quotes.get_untracked().get(&sym).cloned() {
                            apply_bars(vec![cached]);
                        } else {
                            let tok = auth.token.get_untracked().unwrap_or_default();
                            spawn_local(async move {
                                match market::fetch_latest_bar(&tok, &sym).await {
                                    Ok(bar) => {
                                        store.quotes.update(|map| { map.insert(bar.symbol.clone(), bar.clone()); });
                                        mark_quote_results(std::slice::from_ref(&sym), std::slice::from_ref(&bar));
                                        apply_bars(vec![bar]);
                                    }
                                    Err(_) => mark_quote_results(std::slice::from_ref(&sym), &[]),
                                }
                            });
                        }
                    }
                />
            })}

            {move || error.get().map(|e| view! { <p class="text-red-400 text-sm">{e}</p> })}
            {move || loading.get().then(|| view! { <p class="text-gray-400 text-sm">"Loading…"</p> })}

            // ── Portfolio summary ───────────────────────────────────────────
            {move || summary.get().has_data.then(|| view! {
                <SummaryCard summary=summary.get() />
            })}

            // ── Position rows ───────────────────────────────────────────────
            {move || (!loading.get() && positions.get().is_empty() && !show_add.get()).then(|| view! {
                <EmptyState
                    title="Start with what you own"
                    body="Add a stock you hold and Martingale pulls the live price, works out your gain, \
                          and shows what selling it today would cost you in federal tax. Once a holding \
                          is here you can model covered calls and rolls against it."
                >
                    <button
                        class="bg-blue-600 hover:bg-blue-500 px-4 py-2 rounded text-sm font-medium transition-colors"
                        on:click=move |_| show_add.set(true)
                    >
                        "Add my first holding"
                    </button>
                </EmptyState>
            })}

            <div class="space-y-2">
                {move || {
                    positions.get().into_iter()
                        .zip(metrics.get().into_iter())
                        .map(|(p, m)| {
                            let id = p.id;
                            view! {
                                <PositionRow
                                    auth=auth
                                    position=p
                                    metrics=m
                                    implied_tax=implied_tax
                                    on_delete=move || {
                                        let token = auth.token.get().unwrap_or_default();
                                        spawn_local(async move {
                                            if supabase::delete_position(&token, &id.to_string()).await.is_ok() {
                                                positions.update(|ps| ps.retain(|p| p.id != id));
                                            }
                                        });
                                    }
                                    on_update=move |updated: Position| {
                                        positions.update(|ps| {
                                            if let Some(slot) = ps.iter_mut().find(|x| x.id == updated.id) {
                                                *slot = updated.clone();
                                            }
                                        });
                                    }
                                />
                            }
                        })
                        .collect_view()
                }}
            </div>

            // ── Assumptions behind the numbers ──────────────────────────────
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
                        quote_errors=quote_errors
                        on_refresh_quotes=move |_| refresh_quotes()
                    />
                })
            }}
        </div>
    }
}

// ── Market inputs panel ───────────────────────────────────────────────────────

#[component]
fn MarketInputsPanel(
    symbols: Vec<String>,
    market_data: RwSignal<HashMap<String, MarketData>>,
    quote_loading: RwSignal<bool>,
    quote_errors: RwSignal<HashSet<String>>,
    #[prop(into)] on_refresh_quotes: Callback<()>,
) -> impl IntoView {
    view! {
        <div class="bg-panel border border-border rounded-xl p-4">
            <Disclosure
                summary="Assumptions behind these numbers"
                detail="Prices come from live quotes. Change any of them to see what your holdings \
                        would look like at a different price — nothing here is saved, and your \
                        cost basis is untouched. Use the arrow keys in a field to nudge it."
            >
                <div class="space-y-3">
                    <div class="flex items-center justify-end">
                        <button
                            class="text-xs text-gray-500 hover:text-blue-300 disabled:opacity-40 transition-colors font-sans"
                            prop:disabled=move || quote_loading.get()
                            on:click=move |_| on_refresh_quotes.run(())
                        >
                            {move || if quote_loading.get() { "Refreshing…" } else { "↻ Refresh quotes" }}
                        </button>
                    </div>

                    // Live-data failure notice — the price fields below are editable.
                    {move || (!quote_errors.get().is_empty()).then(|| view! {
                        <Callout tone=Tone::Warn>
                            "Couldn’t load live prices for "
                            {let mut s: Vec<String> = quote_errors.get().into_iter().collect(); s.sort(); s.join(", ")}
                            ". Type the current price in below and everything else will recalculate."
                        </Callout>
                    })}

            <div class="grid grid-cols-[auto_1fr_1fr_1fr] gap-x-4 gap-y-2 items-center">
                <span class="text-xs text-gray-500 font-sans">"Symbol"</span>
                <span class="flex items-center gap-1.5 text-xs text-gray-500 font-sans">
                    "Price" <Info term="spot" />
                </span>
                <span class="flex items-center gap-1.5 text-xs text-gray-500 font-sans">
                    "Volatility %" <Info term="implied-vol" />
                </span>
                <span class="flex items-center gap-1.5 text-xs text-gray-500 font-sans">
                    "Rate %" <Info term="risk-free-rate" align_end=true />
                </span>

                {symbols.iter().cloned().map(|sym| {
                    let (sp, sv, sr, sc) = (sym.clone(), sym.clone(), sym.clone(), sym.clone());
                    let (wp, wv, wr) = (sym.clone(), sym.clone(), sym.clone());
                    let (kp, kv, kr) = (sym.clone(), sym.clone(), sym.clone());
                    let se = sym.clone();
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
                            {move || quote_errors.get().contains(&se).then(|| view! {
                                <span class="text-xs font-normal text-amber-400"
                                    title="Live price unavailable — enter manually">
                                    "⚠ manual"
                                </span>
                            })}
                        </span>
                        <input
                            class="bg-surface border border-border rounded px-2 py-1 text-sm focus:outline-none focus:border-blue-500 w-full"
                            prop:value=move || market_data.get().get(&sp).map(|m| m.price.clone()).unwrap_or_default()
                            on:input=move |ev| {
                                let v = event_target_value(&ev);
                                market_data.update(|map| {
                                    if let Some(m) = map.get_mut(&wp) {
                                        m.price = v;
                                        if let (Ok(new_p), Some(ref_p)) = (m.price.parse::<f64>(), m.ref_price) {
                                            m.change = Some(new_p - ref_p);
                                            m.change_pct = Some((new_p - ref_p) / ref_p * 100.0);
                                        }
                                    }
                                });
                            }
                            on:keydown=move |ev| {
                                let key = ev.key();
                                if key != "ArrowUp" && key != "ArrowDown" { return; }
                                ev.prevent_default();
                                let dir = if key == "ArrowUp" { 1.0_f64 } else { -1.0_f64 };
                                market_data.update(|map| {
                                    if let Some(m) = map.get_mut(&kp) {
                                        if let Ok(p) = m.price.parse::<f64>() {
                                            let new_p = (p + 1.0 * dir).max(0.0);
                                            m.price = format!("{:.2}", new_p);
                                            if let Some(ref_p) = m.ref_price {
                                                m.change = Some(new_p - ref_p);
                                                m.change_pct = Some((new_p - ref_p) / ref_p * 100.0);
                                            }
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
            </Disclosure>
        </div>
    }
}

// ── Portfolio summary card ────────────────────────────────────────────────────

#[component]
fn SummaryCard(summary: PortfolioSummary) -> impl IntoView {
    let pnl_class = if summary.total_pnl >= 0.0 { "text-green-400" } else { "text-red-400" };

    view! {
        <div class="bg-panel border border-blue-900 rounded-xl p-6 space-y-4">
            <div class="flex items-start justify-between gap-4">
                <Stat
                    label="What it's all worth"
                    term="market-value"
                    value_class="text-3xl font-semibold"
                >
                    <Num value=summary.total_value />
                </Stat>
                <div class="text-right">
                    <div class="flex items-center justify-end gap-1.5 mb-1">
                        <span class="text-xs text-gray-400 font-sans">"Capital gains if you closed today"</span>
                        <Info term="unrealised-pnl" />
                    </div>
                    <p class=format!("text-2xl font-semibold {}", pnl_class)>
                        <Num value=summary.total_pnl signed=true />
                    </p>
                    <p class="text-[11px] text-gray-600 font-sans mt-0.5">"before tax"</p>
                </div>
            </div>

            <div class="border-t border-border pt-3">
                <Disclosure
                    summary="What moves this portfolio"
                    detail="These describe what your holdings actually react to. Delta is the one \
                            most people want: it's roughly the dollar change for a $1 move in the \
                            underlying stock."
                >
                    <div class="grid grid-cols-2 sm:grid-cols-4 gap-3">
                        <GreekStat label="Per $1 move" term="delta" value=summary.net_delta fmt="{:.1}" />
                        <GreekStat label="Per 1% vol" term="vega" value=summary.net_vega fmt="${:.2}" />
                        <GreekStat label="Per day" term="theta" value=summary.net_theta fmt="${:.2}" />
                        <GreekStat label="Per 1% rate" term="rho" value=summary.net_rho fmt="${:.2}" />
                    </div>
                </Disclosure>
            </div>
        </div>
    }
}

#[component]
fn GreekStat(
    label: &'static str,
    term: &'static str,
    value: f64,
    fmt: &'static str,
) -> impl IntoView {
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
            <div class="flex items-center justify-center gap-1 mb-1">
                <span class="text-xs text-gray-500 font-sans">{label}</span>
                <Info term=term />
            </div>
            <p class=format!("text-sm font-mono font-medium {}", cls)>{display}</p>
        </div>
    }
}

// ── Position row ──────────────────────────────────────────────────────────────

#[component]
fn PositionRow(
    auth: AuthState,
    position: Position,
    metrics: Option<PositionMetrics>,
    implied_tax: RwSignal<HashMap<Uuid, f64>>,
    on_delete: impl Fn() + 'static,
    #[prop(into)] on_update: Callback<Position>,
) -> impl IntoView {
    let is_option = position.kind == PositionKind::Option;
    let pid = position.id;
    let is_trade_log = position.entry_mode == PositionEntryMode::TradeLog;
    let show_trades = RwSignal::new(false);

    let kind_label = match &position.kind {
        PositionKind::Stock => "Stock".to_string(),
        PositionKind::Option => position.option_spec.as_ref().map(|s| {
            format!("{} ${:.0} {}", s.option_type.label(), s.strike, s.expiry.format("%d-%b-%y"))
        }).unwrap_or_else(|| "Option".to_string()),
    };

    // Spell the position out in a sentence — "sold 4 call contracts" beats "-4".
    let eff_qty_for_plain = position.effective_quantity();
    let plain_label = match &position.kind {
        PositionKind::Stock => format!(
            "{} {} shares",
            if eff_qty_for_plain >= 0 { "Holding" } else { "Short" },
            eff_qty_for_plain.abs()
        ),
        PositionKind::Option => position.option_spec.as_ref().map(|s| {
            let verb = if eff_qty_for_plain >= 0 { "Bought" } else { "Sold" };
            let n = eff_qty_for_plain.unsigned_abs();
            format!(
                "{} {} {} contract{} — the right to {} {} shares at ${:.0} until {}",
                verb, n,
                s.option_type.label().to_lowercase(),
                if n == 1 { "" } else { "s" },
                match s.option_type { OptionType::Call => "buy", OptionType::Put => "sell" },
                n * 100,
                s.strike,
                s.expiry.format("%-d %b %Y"),
            )
        }).unwrap_or_default(),
    };
    let term_for_kind = match &position.kind {
        PositionKind::Stock => None,
        PositionKind::Option => position.option_spec.as_ref().map(|s| match s.option_type {
            OptionType::Call => "call",
            OptionType::Put => "put",
        }),
    };

    let eff_qty = position.effective_quantity();
    let eff_cb  = position.effective_cost_basis();
    let qty_class = if eff_qty >= 0 { "text-green-400" } else { "text-red-400" };

    let (mark_str, pnl_class) = match &metrics {
        Some(m) => (format!("${:.2}", m.mark_price), if m.pnl >= 0.0 { "text-green-400" } else { "text-red-400" }),
        None => ("—".into(), "text-gray-500"),
    };
    let mark_price = metrics.as_ref().map(|m| m.mark_price);
    let mark_value = metrics.as_ref().map(|m| m.mark_value);
    let pnl_value  = metrics.as_ref().map(|m| m.pnl);

    let total_cost = eff_cb * eff_qty.abs() as f64
        * if is_option { 100.0 } else { 1.0 }
        * eff_qty.signum() as f64;

    view! {
        <div class="bg-panel border border-border rounded-lg p-3 space-y-2">
            <div class="flex items-center justify-between gap-4">
                <div class="flex items-center gap-4 min-w-0">
                    <span class="font-semibold text-sm w-14 shrink-0">{position.symbol.clone()}</span>
                    <span class="text-xs text-gray-400 truncate">{kind_label}</span>
                    <span class=format!("text-sm font-mono shrink-0 {}", qty_class)>
                        {format!("{:+}", eff_qty)}
                    </span>
                    <span class="text-xs text-gray-500 shrink-0 inline-flex items-center gap-1 font-sans">
                        "paid " <Info term="cost-basis" />
                        <span class="font-mono">{format!("${:.2}", eff_cb)}</span>
                    </span>
                </div>
                <div class="flex items-center gap-4 shrink-0">
                    <span class="text-xs text-gray-400 inline-flex items-center gap-1 font-sans">
                        "now " <Info term="mark-price" />
                        <span class="font-mono">{mark_str}</span>
                    </span>
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

            // Plain-English restatement of the position
            <div class="pl-14 flex items-center gap-1.5">
                <span class="text-xs text-gray-500 font-sans">{plain_label}</span>
                {term_for_kind.map(|t| view! { <Info term=t /> })}
            </div>

            // What it cost, and what leaving would cost
            <div class="flex flex-wrap gap-x-6 gap-y-1 pl-14 text-xs text-gray-500 font-sans">
                <span class="flex items-center gap-1.5">
                    "Total paid " <Info term="cost-basis" />
                    <span class={if total_cost >= 0.0 { "text-gray-300" } else { "text-green-400" }}>
                        {fmt_cash(total_cost)}
                    </span>
                </span>
                <span class="flex items-center gap-1.5">
                    "Tax if closed today " <Info term="implied-tax" />
                    {move || match implied_tax.get().get(&pid).copied() {
                        Some(t) if t > 0.0 => view! {
                            <span class="text-orange-300">{fmt_cash(-t)}</span>
                        }.into_any(),
                        _ => view! { <span class="text-gray-600">"—"</span> }.into_any(),
                    }}
                </span>
            </div>

            // Sensitivities — collapsed; most rows never need them open.
            {metrics.as_ref().map(|m| {
                let (d, v, t, r) = (m.delta, m.vega, m.theta, m.rho);
                view! {
                    <div class="pl-14">
                        <Disclosure
                            summary="What moves this position"
                            detail="Each number is the dollar change in this position for a one-unit \
                                    move in the thing named."
                        >
                            <div class="flex flex-wrap gap-x-5 gap-y-1 text-xs font-mono text-gray-400">
                                <span class="inline-flex items-center gap-1">
                                    "Per $1 move " <Info term="delta" /> <GreekVal v=d fmt="f1" />
                                </span>
                                {is_option.then(|| view! {
                                    <span class="inline-flex items-center gap-1">
                                        "Per 1% vol " <Info term="vega" /> <GreekVal v=v fmt="$" />
                                    </span>
                                    <span class="inline-flex items-center gap-1">
                                        "Per day " <Info term="theta" /> <GreekVal v=t fmt="$" />
                                    </span>
                                    <span class="inline-flex items-center gap-1">
                                        "Per 1% rate " <Info term="rho" /> <GreekVal v=r fmt="$" />
                                    </span>
                                })}
                            </div>
                        </Disclosure>
                    </div>
                }
            })}

            // Trade log toggle + panel
            {is_trade_log.then(|| {
                let pos2 = position.clone();
                view! {
                    <div>
                        <button
                            class="text-xs text-gray-500 hover:text-gray-300 transition-colors pl-14 font-sans"
                            on:click=move |_| show_trades.update(|v| *v = !*v)
                        >
                            {move || if show_trades.get() {
                                "▾ Individual purchases and sales"
                            } else {
                                "▸ Individual purchases and sales"
                            }}
                        </button>
                        {move || show_trades.get().then(|| view! {
                            <TradeLogPanel
                                auth=auth
                                position=pos2.clone()
                                mark_price=mark_price
                                on_update=on_update
                            />
                        })}
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

// ── Trade log panel ───────────────────────────────────────────────────────────

#[component]
fn TradeLogPanel(
    auth: AuthState,
    position: Position,
    mark_price: Option<f64>,
    #[prop(into)] on_update: Callback<Position>,
) -> impl IntoView {
    let trades_sig = RwSignal::new(position.trades.clone());
    let trade_date = RwSignal::new(Utc::now().date_naive().format("%Y-%m-%d").to_string());
    let trade_qty = RwSignal::new(String::new());
    let trade_price = RwSignal::new(String::new());
    let trade_err = RwSignal::new(Option::<String>::None);
    let is_option = position.kind == PositionKind::Option;

    // Store the base position so do_save_fn can be a Copy closure (Copy requires all captures to be Copy).
    // StoredValue<T> is Copy when T: Send + Sync + 'static.
    let base_pos = StoredValue::new(position);

    // All captures of do_save_fn are Copy + Send + Sync, making the closure itself Copy.
    let do_save_fn = move |new_trades: Vec<Trade>| {
        let mut pos = base_pos.get_value();
        pos.trades = new_trades.clone();
        trades_sig.set(new_trades);
        let Some(tok) = auth.token.get_untracked() else {
            trade_err.set(Some("Not signed in.".into()));
            return;
        };
        let Some(uid) = auth.user_id.get_untracked() else {
            trade_err.set(Some("Not signed in.".into()));
            return;
        };
        spawn_local(async move {
            match supabase::upsert_position(&tok, &uid, &pos).await {
                Ok(_) => on_update.run(pos),
                Err(e) => trade_err.set(Some(e)),
            }
        });
    };

    let on_add_trade = move |_| {
        let qty: i32 = match trade_qty.get_untracked().trim().parse() {
            Ok(v) if v != 0 => v,
            _ => { trade_err.set(Some("Quantity must be a non-zero integer.".into())); return; }
        };
        let price: f64 = match trade_price.get_untracked().trim().parse() {
            Ok(v) if v > 0.0 => v,
            _ => { trade_err.set(Some("Price must be a positive number.".into())); return; }
        };
        let date = match chrono::NaiveDate::parse_from_str(trade_date.get_untracked().trim(), "%Y-%m-%d") {
            Ok(d) => d,
            Err(_) => { trade_err.set(Some("Date must be YYYY-MM-DD.".into())); return; }
        };
        trade_err.set(None);
        let mut new_trades = trades_sig.get_untracked();
        new_trades.push(Trade { id: Uuid::new_v4(), date, quantity: qty, price });
        do_save_fn(new_trades);
        trade_qty.set(String::new());
        trade_price.set(String::new());
    };

    // on_del is Copy: captures only Copy+Send+Sync values.
    let on_del = move |trade_id: Uuid| {
        let new_trades: Vec<Trade> = trades_sig.with_untracked(|t| t.iter().filter(|x| x.id != trade_id).cloned().collect());
        do_save_fn(new_trades);
    };

    view! {
        <div class="mt-2 ml-14 space-y-4 border-t border-border pt-3">

            // ── Add trade form: amount, price, date ──────────────────────────
            <div class="space-y-2">
                <p class="text-xs font-medium text-gray-300">"Add trade"</p>
                <div class="flex flex-wrap gap-2 items-end">
                    <div>
                        <label class="block text-xs text-gray-400 mb-1">
                            {if is_option { "Amount (contracts, neg = sell)" } else { "Amount (shares, neg = sell)" }}
                        </label>
                        <input type="text" placeholder="100"
                            class="w-44 bg-surface border border-border rounded px-2 py-1 text-sm focus:outline-none focus:border-blue-500"
                            prop:value=move || trade_qty.get()
                            on:input=move |ev| trade_qty.set(event_target_value(&ev))
                        />
                    </div>
                    <div>
                        <label class="block text-xs text-gray-400 mb-1">
                            {if is_option { "Price (premium / share)" } else { "Price / share" }}
                        </label>
                        <input type="text" placeholder="0.00"
                            class="w-28 bg-surface border border-border rounded px-2 py-1 text-sm focus:outline-none focus:border-blue-500"
                            prop:value=move || trade_price.get()
                            on:input=move |ev| trade_price.set(event_target_value(&ev))
                        />
                    </div>
                    <div>
                        <label class="block text-xs text-gray-400 mb-1">"Date"</label>
                        <input type="date"
                            class="bg-surface border border-border rounded px-2 py-1 text-sm focus:outline-none focus:border-blue-500"
                            prop:value=move || trade_date.get()
                            on:input=move |ev| trade_date.set(event_target_value(&ev))
                        />
                    </div>
                    <button
                        class="text-sm px-3 py-1 rounded bg-blue-600 hover:bg-blue-500 transition-colors"
                        on:click=on_add_trade
                    >"Add"</button>
                </div>
                {move || trade_err.get().map(|e| view! { <p class="text-xs text-red-400">{e}</p> })}
            </div>

            // ── Open lots + realized P&L ─────────────────────────────────────
            {move || {
                let ts = trades_sig.get();
                let (open_lots, closed_lots) = match_trades(&ts, LotAllocation::Fifo);

                let st_pnl: f64 = closed_lots.iter().filter(|l| !l.is_long_term).map(|l| l.realized_pnl).sum();
                let lt_pnl: f64 = closed_lots.iter().filter(|l| l.is_long_term).map(|l| l.realized_pnl).sum();
                let has_closed = !closed_lots.is_empty();
                let has_open = !open_lots.is_empty();

                view! {
                    <div class="space-y-3">
                        {has_open.then(|| view! {
                            <div class="space-y-1">
                                <p class="text-xs font-medium text-gray-300">"Open lots"</p>
                                <div class="grid gap-x-4 text-xs text-gray-500"
                                    style="grid-template-columns: auto auto auto auto auto">
                                    <span>"Date"</span>
                                    <span class="text-right">"Qty"</span>
                                    <span class="text-right">"Cost"</span>
                                    <span class="text-right">"Days"</span>
                                    <span class="text-right">"Unreal. P&L"</span>
                                    {open_lots.iter().map(|lot| {
                                        let days = (Utc::now().date_naive() - lot.date).num_days();
                                        let holding = if days > 365 { format!("{}d LT", days) } else { format!("{}d", days) };
                                        let holding_cls = if days > 365 { "text-green-500" } else { "text-gray-400" };
                                        let unrealized = mark_price.map(|mp| {
                                            let mult = if is_option { 100.0 } else { 1.0 };
                                            (mp - lot.price) * lot.quantity.signum() as f64 * lot.quantity.abs() as f64 * mult
                                        });
                                        let pnl_cls = unrealized.map(|p| if p >= 0.0 { "text-green-400" } else { "text-red-400" }).unwrap_or("text-gray-500");
                                        view! {
                                            <span class="text-gray-400">{lot.date.format("%b %-d '%y").to_string()}</span>
                                            <span class="text-right font-mono">{format!("{:+}", lot.quantity)}</span>
                                            <span class="text-right">{format!("${:.2}", lot.price)}</span>
                                            <span class=format!("text-right {}", holding_cls)>{holding}</span>
                                            <span class=format!("text-right font-mono {}", pnl_cls)>
                                                {match unrealized {
                                                    Some(v) => fmt_cash(v),
                                                    None => "—".into(),
                                                }}
                                            </span>
                                        }
                                    }).collect_view()}
                                </div>
                            </div>
                        })}

                        {has_closed.then(|| view! {
                            <div class="text-xs space-y-0.5">
                                <p class="font-medium text-gray-300">"Realized P&L"</p>
                                {(st_pnl != 0.0).then(|| {
                                    let cls = if st_pnl >= 0.0 { "text-green-400" } else { "text-red-400" };
                                    view! {
                                        <div class="flex justify-between text-gray-400">
                                            <span>"Short-term"</span>
                                            <span class=cls>{fmt_cash(st_pnl)}</span>
                                        </div>
                                    }
                                })}
                                {(lt_pnl != 0.0).then(|| {
                                    let cls = if lt_pnl >= 0.0 { "text-green-400" } else { "text-red-400" };
                                    view! {
                                        <div class="flex justify-between text-gray-400">
                                            <span>"Long-term"</span>
                                            <span class=cls>{fmt_cash(lt_pnl)}</span>
                                        </div>
                                    }
                                })}
                            </div>
                        })}
                    </div>
                }
            }}

            // ── Trade history ────────────────────────────────────────────────
            {move || {
                let ts = trades_sig.get();
                if ts.is_empty() {
                    return view! { <p class="text-xs text-gray-600 italic">"No trades yet."</p> }.into_any();
                }
                let mut sorted = ts.clone();
                sorted.sort_by_key(|t| std::cmp::Reverse(t.date));
                view! {
                    <div class="space-y-1">
                        <p class="text-xs font-medium text-gray-300">"Trade history"</p>
                        {sorted.into_iter().map(|t| {
                            let tid = t.id;
                            let qty_cls = if t.quantity > 0 { "text-green-400" } else { "text-red-400" };
                            let action = if t.quantity > 0 { "Buy" } else { "Sell" };
                            view! {
                                <div class="flex items-center justify-between text-xs py-0.5 border-b border-border">
                                    <div class="flex items-center gap-3">
                                        <span class="text-gray-500 w-20">{t.date.format("%b %-d '%y").to_string()}</span>
                                        <span class=qty_cls>{format!("{} {:+}", action, t.quantity)}</span>
                                        <span class="text-gray-400">{format!("@ ${:.2}", t.price)}</span>
                                    </div>
                                    <button
                                        class="text-gray-600 hover:text-red-400 transition-colors"
                                        on:click=move |_| on_del(tid)
                                    >"✕"</button>
                                </div>
                            }
                        }).collect_view()}
                    </div>
                }.into_any()
            }}
        </div>
    }
}

// ── Add position form ─────────────────────────────────────────────────────────

#[component]
fn AddPositionForm(
    auth: AuthState,
    on_added: impl Fn(Position) + 'static,
) -> impl IntoView {
    let on_added = Rc::new(on_added);
    let symbol     = RwSignal::new(String::new());
    let kind       = RwSignal::new(PositionKind::Stock);
    let entry_mode = RwSignal::new(PositionEntryMode::Snapshot);
    let quantity   = RwSignal::new("1".to_string());
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
                // Prefer the pipeline-populated chain when it covers this symbol.
                if let Ok(meta) = market::fetch_option_meta(&tok, &sym).await {
                    if !meta.is_empty() {
                        store.option_meta.update(|map| { map.insert(sym.clone(), meta.clone()); });
                        option_meta.set(meta);
                        return;
                    }
                }

                // No pipeline data — fall back to the on-demand live chain, which
                // serves from the 15-min cache or fetches from the market data API
                // page-by-page. Merge each page into the dropdowns as it arrives.
                let mut acc: Vec<OptionMetaEntry> = Vec::new();
                let mut page_token: Option<String> = None;
                loop {
                    match market::fetch_option_chain_live(&tok, &sym, page_token.as_deref()).await {
                        Ok(page) => {
                            for e in &page.entries {
                                acc.push(OptionMetaEntry {
                                    expiry: e.expiry.clone(),
                                    option_type: e.option_type.clone(),
                                    strike: e.strike,
                                });
                            }
                            option_meta.set(acc.clone());
                            match page.next_page_token {
                                Some(t) => page_token = Some(t),
                                None => break,
                            }
                        }
                        Err(_) => break,
                    }
                }
                store.option_meta.update(|map| { map.insert(sym.clone(), acc); });
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

        let mode = entry_mode.get();

        let (qty, cb) = if mode == PositionEntryMode::TradeLog {
            (0i32, 0.0f64)
        } else {
            let q: i32 = match quantity.get().trim().parse() {
                Ok(v) => v,
                Err(_) => { err.set(Some("Invalid quantity.".into())); return; }
            };
            let c: f64 = match cost_basis.get().trim().parse() {
                Ok(v) => v,
                Err(_) => { err.set(Some("Invalid cost basis.".into())); return; }
            };
            (q, c)
        };

        let mut position = match kind.get() {
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
        position.entry_mode = mode;

        // Fire-and-forget: warm the quote + option-chain caches for a freshly
        // added stock so its mark and a future option entry have data ready.
        // Both fill in asynchronously — the Add itself doesn't block on them.
        if kind.get() == PositionKind::Stock {
            let warm_tok = auth.token.get().unwrap_or_default();
            let warm_sym = sym.clone();
            spawn_local(async move {
                let _ = market::fetch_latest_bar(&warm_tok, &warm_sym).await;
                let mut page_token: Option<String> = None;
                loop {
                    match market::fetch_option_chain_live(&warm_tok, &warm_sym, page_token.as_deref()).await {
                        Ok(page) => match page.next_page_token {
                            Some(t) => page_token = Some(t),
                            None => break,
                        },
                        Err(_) => break,
                    }
                }
            });
        }

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

            // Kind toggle
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

            // Entry mode toggle
            <div class="space-y-1.5">
                <div class="flex items-center gap-2 text-xs">
                    <span class="text-gray-500 font-sans">"How do you want to enter it?"</span>
                    <div class="flex rounded overflow-hidden border border-border">
                        <button type="button"
                            class=move || if entry_mode.get() == PositionEntryMode::Snapshot {
                                "px-2 py-0.5 bg-blue-600 text-white"
                            } else {
                                "px-2 py-0.5 text-gray-400 hover:text-gray-200"
                            }
                            on:click=move |_| entry_mode.set(PositionEntryMode::Snapshot)
                        >"One total"</button>
                        <button type="button"
                            class=move || if entry_mode.get() == PositionEntryMode::TradeLog {
                                "px-2 py-0.5 bg-blue-600 text-white"
                            } else {
                                "px-2 py-0.5 text-gray-400 hover:text-gray-200"
                            }
                            on:click=move |_| entry_mode.set(PositionEntryMode::TradeLog)
                        >"Purchase by purchase"</button>
                    </div>
                </div>
                <Hint>
                    {move || if entry_mode.get() == PositionEntryMode::TradeLog {
                        "You'll add each buy and sell after creating this, with its own date and price. \
                         Slower, but it tracks holding periods properly — which is what decides whether \
                         a gain is taxed at the long-term rate."
                    } else {
                        "Enter one quantity and one average price. Quick, but Martingale can't tell \
                         which shares are long-term, so tax estimates are rougher."
                    }}
                </Hint>
            </div>

            <div class="grid grid-cols-2 gap-3">
                <MiniInput label="Symbol" signal=symbol ph="AAPL" />
                {move || (entry_mode.get() == PositionEntryMode::Snapshot).then(|| view! {
                    <>
                        <MiniInput label="Quantity (neg=short)" signal=quantity  ph="1" />
                        <MiniInput label="Cost basis / share"  signal=cost_basis ph="0.00" />
                    </>
                })}

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
