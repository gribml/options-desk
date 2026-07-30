use gloo_timers::callback::Timeout;
use leptos::prelude::*;
use uuid::Uuid;
use wasm_bindgen_futures::spawn_local;

use crate::api::{market, supabase};
use crate::app::AuthState;
use crate::charts::{self, LinePlot, Series};
use crate::components::ui::{Callout, Disclosure, Hint, Info, Label, Tone};
use crate::models::combo::{Combo, ComboLegSpec};
use crate::models::market::OptionMetaEntry;
use crate::models::option::OptionType;
use crate::pricing::black_scholes::{implied_vol, BsInputs};
use crate::pricing::combo::{self, ComboLeg};
use crate::store::MarketStore;

fn parse_f64(s: &str) -> Option<f64> {
    let v: f64 = s.trim().parse().ok()?;
    if v.is_finite() && v > 0.0 { Some(v) } else { None }
}

// ── Page shell with tabs ────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    Pricer,
    Surface,
    Combos,
}

#[component]
pub fn PricerPage() -> impl IntoView {
    let tab = RwSignal::new(Tab::Pricer);

    let tab_btn = move |t: Tab, label: &'static str| {
        view! {
            <button
                class=move || format!(
                    "px-4 py-2 text-sm font-medium border-b-2 transition-colors {}",
                    if tab.get() == t { "border-blue-500 text-white" }
                    else { "border-transparent text-gray-400 hover:text-gray-200" }
                )
                on:click=move |_| tab.set(t)
            >{label}</button>
        }
    };

    view! {
        <div class="max-w-4xl mx-auto space-y-6">
            <div>
                <h1 class="text-xl font-semibold">"Pricer"</h1>
                <p class="text-xs text-gray-500 mt-1 font-sans">
                    "Work out what an option contract is worth, and what would have to happen for that to change."
                </p>
            </div>

            <div class="flex gap-2 border-b border-border">
                {tab_btn(Tab::Pricer, "Price an option")}
                {tab_btn(Tab::Surface, "Volatility map")}
                {tab_btn(Tab::Combos, "Track a combination")}
            </div>

            {move || match tab.get() {
                Tab::Pricer  => view! { <BsPricer /> }.into_any(),
                Tab::Surface => view! { <VolSurfaceTab /> }.into_any(),
                Tab::Combos  => view! { <CombosTab /> }.into_any(),
            }}
        </div>
    }
}

// ── Tab 1: Black-Scholes pricer (unchanged behaviour) ───────────────────────

#[component]
fn BsPricer() -> impl IntoView {
    let spot = RwSignal::new("100".to_string());
    let strike = RwSignal::new("100".to_string());
    let vol = RwSignal::new("25".to_string());
    let rate = RwSignal::new("3.75".to_string());
    let expiry_days = RwSignal::new("30".to_string());
    let opt_type = RwSignal::new(OptionType::Call);
    let market_price = RwSignal::new("".to_string());

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
            vol: 0.25,
            rate: parse_f64(&rate.get())? / 100.0,
            expiry_years: parse_f64(&expiry_days.get())? / 365.0,
        };
        implied_vol(mp, inputs, opt_type.get())
    });

    view! {
        <div class="max-w-2xl space-y-6">
            <div class="space-y-2">
                <div class="flex items-center gap-3">
                    <span class="text-xs text-gray-400 font-sans">"Which kind of contract?"</span>
                    <span class="flex items-center gap-1.5 text-xs text-gray-600 font-sans">
                        "call" <Info term="call" /> "put" <Info term="put" />
                    </span>
                </div>
                <div class="flex gap-2">
                    {[OptionType::Call, OptionType::Put].map(|t| {
                        let label = t.label();
                        let sub = match t {
                            OptionType::Call => "right to buy",
                            OptionType::Put => "right to sell",
                        };
                        view! {
                            <button
                                class=move || format!(
                                    "px-6 py-2 rounded text-sm font-medium border transition-colors text-left {}",
                                    if opt_type.get() == t { "bg-blue-600 border-blue-600 text-white" }
                                    else { "bg-panel border-border text-gray-400 hover:border-gray-500" }
                                )
                                on:click=move |_| opt_type.set(t)
                            >
                                {label}
                                <span class="block text-[10px] opacity-70 font-sans font-normal">{sub}</span>
                            </button>
                        }
                    })}
                </div>
            </div>

            <div class="grid grid-cols-2 gap-4">
                <InputField label="Stock price today" term="spot" signal=spot placeholder="100" />
                <InputField label="Strike price" term="strike" signal=strike placeholder="100" />
                <InputField label="Expected swing (% a year)" term="volatility" signal=vol placeholder="25" />
                <InputField label="Days until it expires" term="expiry" signal=expiry_days placeholder="30" />
            </div>

            <Disclosure
                summary="Advanced input"
                detail="Interest rates barely move an option's value compared with the four fields \
                        above. The default tracks short-term Treasury yields — leave it unless rates \
                        have shifted a lot."
            >
                <div class="grid grid-cols-2 gap-4">
                    <InputField label="Interest rate (%)" term="risk-free-rate" signal=rate placeholder="3.75" />
                </div>
            </Disclosure>

            {move || result.get().map(|r| view! {
                <div class="bg-panel border border-blue-900 rounded-xl p-6 space-y-4">
                    <div>
                        <div class="flex items-center gap-1.5 mb-1">
                            <span class="text-xs text-gray-400 font-sans">"What one contract is worth"</span>
                            <Info term="premium" />
                        </div>
                        <p class="text-3xl font-semibold">{format!("${:.2}", r.price * 100.0)}</p>
                        <p class="text-xs text-gray-500 mt-1 font-sans">
                            {format!("${:.4} per share × 100 shares per contract", r.price)}
                        </p>
                    </div>

                    <div class="border-t border-border pt-3">
                        <Disclosure
                            summary="What would change this price"
                            detail="Each row is how much the per-share price moves when one thing \
                                    changes and everything else stays put."
                        >
                            <div class="grid grid-cols-1 sm:grid-cols-2 gap-x-6">
                                <GreekRow label="If the stock rises $1" term="delta" value=format!("{:+.4}", r.delta) />
                                <GreekRow label="How fast that shifts" term="gamma" value=format!("{:+.4}", r.gamma) />
                                <GreekRow label="If swings rise 1 point" term="vega" value=format!("{:+.4}", r.vega) />
                                <GreekRow label="Each day that passes" term="theta" value=format!("{:+.4}", r.theta) />
                                <GreekRow label="If rates rise 1 point" term="rho" value=format!("{:+.4}", r.rho) />
                            </div>
                        </Disclosure>
                    </div>
                </div>
            })}

            <div class="bg-panel border border-border rounded-xl p-6 space-y-3">
                <div class="flex items-center gap-1.5">
                    <h2 class="text-sm font-medium text-gray-300 font-sans">"Work backwards from a real price"</h2>
                    <Info term="implied-vol" />
                </div>
                <Hint>
                    "If you know what the contract actually trades for, this tells you the expected \
                     swing the market is pricing in — useful for judging whether an option looks \
                     cheap or expensive."
                </Hint>
                <div class="flex gap-3 items-end">
                    <div class="flex-1">
                        <Label text="Price you were quoted (per share)" term=Some("premium") />
                        <input
                            class="w-full bg-surface border border-border rounded px-3 py-2 text-sm focus:outline-none focus:border-blue-500"
                            prop:value=move || market_price.get()
                            on:input=move |ev| market_price.set(event_target_value(&ev))
                            placeholder="e.g. 3.50"
                        />
                    </div>
                </div>
                {move || iv.get().map(|v| view! {
                    <p class="text-blue-300 text-sm font-sans">
                        "The market is pricing in swings of about "
                        <span class="font-semibold font-mono">{format!("{:.1}%", v * 100.0)}</span>
                        " a year."
                    </p>
                })}
                {move || {
                    if market_price.get().trim().is_empty() {
                        None
                    } else if iv.get().is_none() {
                        Some(view! {
                            <Callout tone=Tone::Warn>
                                "No volatility can produce that price. It's usually below the contract's \
                                 minimum possible value — check the strike, the stock price, and the days left."
                            </Callout>
                        })
                    } else {
                        None
                    }
                }}
            </div>
        </div>
    }
}

// ── Tab 2: Vol surface (placeholder — pipeline not yet connected) ────────────

#[derive(Clone, PartialEq)]
enum SurfaceState {
    Empty,
    NotAvailable(String),
    Building(String),
}

#[component]
fn VolSurfaceTab() -> impl IntoView {
    let symbol = RwSignal::new(String::new());
    let state = RwSignal::new(SurfaceState::Empty);

    let load = move |_| {
        let sym = symbol.get_untracked().trim().to_uppercase();
        if sym.is_empty() { return; }
        // No surface data exists yet (pipeline not connected), so the lookup
        // always reports "not available" and offers a Build action.
        state.set(SurfaceState::NotAvailable(sym));
    };

    let build = move |_| {
        if let SurfaceState::NotAvailable(sym) = state.get_untracked() {
            // Placeholder: this will POST to a Modal backend that pulls the
            // option chain and calibrates a surface, then we poll for the
            // result and plot it. Not wired yet.
            state.set(SurfaceState::Building(sym));
        }
    };

    view! {
        <div class="space-y-4">
            <div class="flex items-center gap-1.5">
                <h2 class="text-sm font-medium text-gray-300 font-sans">"Volatility map"</h2>
                <Info term="vol-surface" />
            </div>
            <Hint>
                "The market doesn't expect the same size of swing for every strike and every expiry \
                 on the same stock. This maps that out, so you can see which contracts are priced \
                 richly relative to the rest."
            </Hint>

            <div class="flex gap-2 items-end">
                <div class="flex-1 max-w-xs">
                    <Label text="Stock symbol" />
                    <input
                        class="w-full bg-surface border border-border rounded px-3 py-2 text-sm focus:outline-none focus:border-blue-500"
                        prop:value=move || symbol.get()
                        on:input=move |ev| symbol.set(event_target_value(&ev))
                        placeholder="AAPL"
                    />
                </div>
                <button
                    class="bg-blue-600 hover:bg-blue-500 px-4 py-2 rounded text-sm font-medium transition-colors"
                    on:click=load
                >"Load map"</button>
            </div>

            <div class="bg-panel border border-border rounded-xl p-8 min-h-[280px] flex items-center justify-center text-center">
                {move || match state.get() {
                    SurfaceState::Empty => view! {
                        <p class="text-gray-500 text-sm font-sans max-w-sm">
                            "Enter a stock symbol above to see its volatility map."
                        </p>
                    }.into_any(),
                    SurfaceState::NotAvailable(sym) => view! {
                        <div class="space-y-3 max-w-sm">
                            <p class="text-gray-400 text-sm font-sans">
                                "Nothing built for " <span class="font-semibold text-gray-200">{sym}</span>
                                " yet. Building one pulls its full option chain and fits a curve through it."
                            </p>
                            <button
                                class="bg-blue-600 hover:bg-blue-500 px-4 py-2 rounded text-sm font-medium transition-colors"
                                on:click=build
                            >"Build it"</button>
                        </div>
                    }.into_any(),
                    SurfaceState::Building(sym) => view! {
                        <div class="max-w-sm">
                            <Callout>
                                "Requested for " <span class="font-semibold">{sym}</span>
                                ". This feature isn't finished — the calculation service behind it \
                                 isn't connected yet, so nothing will come back. Everything else on \
                                 this page works normally."
                            </Callout>
                        </div>
                    }.into_any(),
                }}
            </div>
        </div>
    }
}

// ── Tab 3: Combos ────────────────────────────────────────────────────────────

#[derive(Clone, Copy)]
struct LegForm {
    option_type: RwSignal<OptionType>,
    strike: RwSignal<String>,
    expiry: RwSignal<String>,
    quantity: RwSignal<String>,
}

impl LegForm {
    fn new(quantity: &str, opt: OptionType) -> Self {
        Self {
            option_type: RwSignal::new(opt),
            strike: RwSignal::new(String::new()),
            expiry: RwSignal::new(String::new()),
            quantity: RwSignal::new(quantity.to_string()),
        }
    }
}

#[derive(Clone, PartialEq)]
enum HistStatus {
    Idle,
    Loading,
    Empty,
    Loaded,
    Error(String),
}

#[derive(Clone, Copy)]
struct ComboTrack {
    id: Uuid,
    label: RwSignal<String>,
    symbol: RwSignal<String>,
    legs: RwSignal<Vec<LegForm>>,
    meta: RwSignal<Vec<OptionMetaEntry>>,
    spot: RwSignal<String>,
    vol: RwSignal<String>,
    rate: RwSignal<String>,
    /// Metric toggle for all plots: false = price difference, true = vol.
    vol_mode: RwSignal<bool>,
    collapsed: RwSignal<bool>,
    hist_price: RwSignal<Vec<(f64, f64)>>,
    hist_vol: RwSignal<Vec<(f64, f64)>>,
    hist_status: RwSignal<HistStatus>,
}

impl ComboTrack {
    fn new() -> Self {
        // Default to a two-leg roll shape (sell near / buy far).
        Self {
            id: Uuid::new_v4(),
            label: RwSignal::new(String::new()),
            symbol: RwSignal::new(String::new()),
            legs: RwSignal::new(vec![
                LegForm::new("-1", OptionType::Call),
                LegForm::new("1", OptionType::Call),
            ]),
            meta: RwSignal::new(vec![]),
            spot: RwSignal::new("100".to_string()),
            vol: RwSignal::new("25".to_string()),
            rate: RwSignal::new("3.75".to_string()),
            vol_mode: RwSignal::new(false),
            collapsed: RwSignal::new(false),
            hist_price: RwSignal::new(vec![]),
            hist_vol: RwSignal::new(vec![]),
            hist_status: RwSignal::new(HistStatus::Idle),
        }
    }

    /// Rebuild an in-memory combo from a saved record (collapsed by default).
    fn from_combo(c: &Combo) -> Self {
        let legs: Vec<LegForm> = c.legs.iter().map(|s| {
            let lf = LegForm::new(&s.quantity.to_string(), s.option_type);
            lf.strike.set(format!("{}", s.strike));
            lf.expiry.set(s.expiry.clone());
            lf
        }).collect();
        Self {
            id: c.id,
            label: RwSignal::new(c.label.clone()),
            symbol: RwSignal::new(c.symbol.clone()),
            legs: RwSignal::new(if legs.is_empty() { vec![LegForm::new("1", OptionType::Call)] } else { legs }),
            meta: RwSignal::new(vec![]),
            spot: RwSignal::new("100".to_string()),
            vol: RwSignal::new("25".to_string()),
            rate: RwSignal::new("3.75".to_string()),
            vol_mode: RwSignal::new(c.vol_mode),
            collapsed: RwSignal::new(true),
            hist_price: RwSignal::new(vec![]),
            hist_vol: RwSignal::new(vec![]),
            hist_status: RwSignal::new(HistStatus::Idle),
        }
    }

    /// Serialize the current state for persistence (drops incomplete legs).
    fn to_combo(&self) -> Combo {
        let legs = self.legs.get_untracked().iter().filter_map(|l| {
            let strike = l.strike.get_untracked().trim().parse::<f64>().ok()?;
            let quantity = l.quantity.get_untracked().trim().parse::<i32>().ok().filter(|v| *v != 0)?;
            Some(ComboLegSpec {
                option_type: l.option_type.get_untracked(),
                strike,
                expiry: l.expiry.get_untracked(),
                quantity,
            })
        }).collect();
        Combo {
            id: self.id,
            label: self.label.get_untracked(),
            symbol: self.symbol.get_untracked(),
            legs,
            vol_mode: self.vol_mode.get_untracked(),
        }
    }
}

/// The ATM contract to quote for a vol-slider default: the call whose strike is
/// nearest `spot` in the earliest non-expired expiry present in the metadata.
fn atm_contract(meta: &[OptionMetaEntry], spot: f64) -> Option<(String, &'static str, f64)> {
    let today = chrono::Local::now().date_naive();
    let mut expiries: Vec<String> = meta.iter().map(|e| e.expiry.clone()).collect();
    expiries.sort();
    expiries.dedup();
    let first = expiries.into_iter().find(|e| {
        chrono::NaiveDate::parse_from_str(e, "%Y-%m-%d").map(|d| d >= today).unwrap_or(false)
    })?;
    let entry = meta.iter()
        .filter(|e| e.expiry == first && e.option_type == "call")
        .min_by(|a, b| {
            (a.strike - spot).abs()
                .partial_cmp(&(b.strike - spot).abs())
                .unwrap_or(std::cmp::Ordering::Equal)
        })?;
    Some((first, "call", entry.strike))
}

fn parse_legs(legs: &[LegForm], today: chrono::NaiveDate) -> Vec<ComboLeg> {
    legs.iter()
        .filter_map(|l| {
            let strike = l.strike.get().trim().parse::<f64>().ok().filter(|v| *v > 0.0)?;
            let quantity = l.quantity.get().trim().parse::<i32>().ok().filter(|v| *v != 0)?;
            let expiry = chrono::NaiveDate::parse_from_str(l.expiry.get().trim(), "%Y-%m-%d").ok()?;
            let expiry_years = (expiry - today).num_days().max(0) as f64 / 365.0;
            Some(ComboLeg { option_type: l.option_type.get(), strike, expiry_years, quantity })
        })
        .collect()
}

fn date_to_unix(d: &str) -> Option<f64> {
    let nd = chrono::NaiveDate::parse_from_str(d.trim(), "%Y-%m-%d").ok()?;
    Some(nd.and_hms_opt(0, 0, 0)?.and_utc().timestamp() as f64)
}

/// Defer a plot call to the next tick so the target div is laid out (uPlot
/// sizes to `clientWidth`).
fn defer_plot(id: String, plot: LinePlot) {
    Timeout::new(0, move || charts::line_plot(&id, &plot)).forget();
}

#[component]
fn CombosTab() -> impl IntoView {
    let auth = use_context::<AuthState>().expect("AuthState missing");
    let combos = RwSignal::new(Vec::<ComboTrack>::new());
    let loaded = RwSignal::new(false);

    // Load saved combos once (falls back to a single blank combo).
    Effect::new(move |_| {
        if loaded.get_untracked() { return; }
        let (tok, uid) = match (auth.token.get(), auth.user_id.get()) {
            (Some(t), Some(u)) => (t, u),
            _ => return,
        };
        loaded.set(true);
        spawn_local(async move {
            match supabase::fetch_combos(&tok, &uid).await {
                Ok(saved) if !saved.is_empty() => {
                    combos.set(saved.iter().map(ComboTrack::from_combo).collect());
                }
                _ => combos.set(vec![ComboTrack::new()]),
            }
        });
    });

    view! {
        <div class="space-y-4">
            <div class="flex items-start justify-between gap-4">
                <div class="space-y-1">
                    <div class="flex items-center gap-1.5">
                        <h2 class="text-sm font-medium text-gray-300 font-sans">"Track a combination"</h2>
                        <Info term="combo" />
                    </div>
                    <Hint>
                        "When you roll a covered call you're really doing two things at once — buying \
                         back one contract and selling another. What matters is the net price of the \
                         pair, not either leg on its own. Set the legs up here and track that number."
                    </Hint>
                </div>
                <button
                    class="bg-blue-600 hover:bg-blue-500 px-4 py-2 rounded text-sm font-medium transition-colors shrink-0"
                    on:click=move |_| combos.update(|v| v.push(ComboTrack::new()))
                >"+ Add combination"</button>
            </div>

            {move || combos.get().into_iter().enumerate().map(|(i, c)| view! {
                <ComboCard
                    combo=c
                    auth=auth
                    on_remove=move || {
                        let id = c.id;
                        let tok = auth.token.get_untracked().unwrap_or_default();
                        spawn_local(async move { let _ = supabase::delete_combo(&tok, &id.to_string()).await; });
                        combos.update(|v| { v.remove(i); });
                    }
                />
            }).collect_view()}
        </div>
    }
}

#[component]
fn ComboCard(combo: ComboTrack, auth: AuthState, on_remove: impl Fn() + 'static) -> impl IntoView {
    let today = chrono::Local::now().date_naive();
    let idp = combo.id.simple().to_string();
    let id_price = format!("cs-price-{idp}");
    let id_vol = format!("cs-vol-{idp}");
    let id_rate = format!("cs-rate-{idp}");
    let id_time = format!("cs-time-{idp}");
    let id_hist = format!("cs-hist-{idp}");

    let legs_parsed = Memo::new(move |_| parse_legs(&combo.legs.get(), today));
    let center = Memo::new(move |_| {
        let strikes: Vec<f64> = combo.legs.get().iter()
            .filter_map(|l| l.strike.get().trim().parse::<f64>().ok())
            .filter(|v| *v > 0.0)
            .collect();
        if strikes.is_empty() { 100.0 } else { strikes.iter().sum::<f64>() / strikes.len() as f64 }
    });

    // Current slider values (Copy closures reused across the sensitivity effects).
    let cur_spot = move || combo.spot.get().trim().parse::<f64>().unwrap_or(100.0);
    let cur_vol = move || combo.vol.get().trim().parse::<f64>().unwrap_or(25.0) / 100.0;
    let cur_rate = move || combo.rate.get().trim().parse::<f64>().unwrap_or(3.75) / 100.0;

    // Distinct expiries from the loaded metadata (for the leg dropdowns).
    let expiries = Memo::new(move |_| {
        let mut v: Vec<String> = combo.meta.get().iter().map(|e| e.expiry.clone()).collect();
        v.sort();
        v.dedup();
        v
    });

    let store = use_context::<MarketStore>().expect("MarketStore missing");

    // On symbol change: default spot to the latest close, load the option-chain
    // metadata for the dropdowns (same flow as the portfolio's add form — the
    // pipeline snapshot first, then the on-demand live chain if the DB has none),
    // and default vol to the ATM implied vol.
    Effect::new(move |_| {
        let sym = combo.symbol.get().trim().to_uppercase();
        if sym.is_empty() { return; }
        // Serve dropdowns from cache immediately if we have it.
        if let Some(cached) = store.option_meta.get_untracked().get(&sym).cloned() {
            combo.meta.set(cached);
        }
        let tok = auth.token.get_untracked().unwrap_or_default();
        spawn_local(async move {
            let mut spot = combo.spot.get_untracked().trim().parse::<f64>().unwrap_or(0.0);
            if let Ok(bar) = market::fetch_latest_bar(&tok, &sym).await {
                spot = bar.close;
                combo.spot.set(format!("{:.2}", bar.close));
            }

            let meta = if let Some(cached) = store.option_meta.get_untracked().get(&sym).cloned() {
                cached
            } else {
                // Prefer the pipeline-populated chain when it covers this symbol.
                let mut m = market::fetch_option_meta(&tok, &sym).await.unwrap_or_default();
                if m.is_empty() {
                    // No pipeline data — fall back to the on-demand live chain
                    // (15-min cache or page-by-page fetch), merging as it arrives.
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
                                combo.meta.set(acc.clone());
                                match page.next_page_token {
                                    Some(t) => page_token = Some(t),
                                    None => break,
                                }
                            }
                            Err(_) => break,
                        }
                    }
                    m = acc;
                }
                store.option_meta.update(|map| { map.insert(sym.clone(), m.clone()); });
                m
            };
            combo.meta.set(meta.clone());

            // Default vol to the ATM contract's implied vol.
            if spot > 0.0 {
                if let Some((exp, ty, strike)) = atm_contract(&meta, spot) {
                    if let Ok(q) = market::fetch_option_quote(&tok, &sym, &exp, ty, strike).await {
                        if let Some(iv) = q.implied_vol {
                            combo.vol.set(format!("{:.1}", iv * 100.0));
                        }
                    }
                }
            }
        });
    });

    // ── Sensitivity plots ───────────────────────────────────────────────
    let value_series = |ys: Vec<f64>| vec![Series { label: "Net".into(), color: "#60a5fa".into(), y: ys }];

    // vs Underlying
    {
        let id = id_price.clone();
        Effect::new(move |_| {
            if combo.collapsed.get() || combo.vol_mode.get() { charts::destroy_plot(&id); return; }
            let legs = legs_parsed.get();
            if legs.is_empty() { charts::destroy_plot(&id); return; }
            let c = center.get();
            let (xs, ys) = combo::sweep_spot(&legs, cur_vol(), cur_rate(), (c * 0.5).max(0.5), c * 1.5, 61);
            defer_plot(id.clone(), LinePlot {
                title: "Price vs Underlying".into(),
                x_label: "Underlying".into(),
                x_fmt: "usd".into(), y_fmt: "usd".into(),
                x: xs, series: value_series(ys),
            });
        });
    }
    // vs Vol
    {
        let id = id_vol.clone();
        Effect::new(move |_| {
            if combo.collapsed.get() || combo.vol_mode.get() { charts::destroy_plot(&id); return; }
            let legs = legs_parsed.get();
            if legs.is_empty() { charts::destroy_plot(&id); return; }
            let (xs, ys) = combo::sweep_vol(&legs, cur_spot(), cur_rate(), 0.01, 1.5, 61);
            defer_plot(id.clone(), LinePlot {
                title: "Price vs Volatility".into(),
                x_label: "Vol".into(),
                x_fmt: "pct".into(), y_fmt: "usd".into(),
                x: xs, series: value_series(ys),
            });
        });
    }
    // vs Rate
    {
        let id = id_rate.clone();
        Effect::new(move |_| {
            if combo.collapsed.get() || combo.vol_mode.get() { charts::destroy_plot(&id); return; }
            let legs = legs_parsed.get();
            if legs.is_empty() { charts::destroy_plot(&id); return; }
            let (xs, ys) = combo::sweep_rate(&legs, cur_spot(), cur_vol(), 0.0, 0.12, 61);
            defer_plot(id.clone(), LinePlot {
                title: "Price vs Rate".into(),
                x_label: "Rate".into(),
                x_fmt: "pct".into(), y_fmt: "usd".into(),
                x: xs, series: value_series(ys),
            });
        });
    }
    // vs Time to maturity (decay curve)
    {
        let id = id_time.clone();
        Effect::new(move |_| {
            if combo.collapsed.get() || combo.vol_mode.get() { charts::destroy_plot(&id); return; }
            let legs = legs_parsed.get();
            if legs.is_empty() { charts::destroy_plot(&id); return; }
            // Only meaningful up to the nearest expiry — past that the closest
            // leg has settled and the combo is a different instrument.
            let horizon = legs.iter()
                .map(|l| l.expiry_years)
                .filter(|&t| t > 0.0)
                .fold(f64::INFINITY, f64::min);
            if !horizon.is_finite() { charts::destroy_plot(&id); return; }
            let (xs, ys) = combo::sweep_time(&legs, cur_spot(), cur_vol(), cur_rate(), horizon, 61);
            defer_plot(id.clone(), LinePlot {
                title: "Price vs Time".into(),
                x_label: "Days forward".into(),
                x_fmt: "num0".into(), y_fmt: "usd".into(),
                x: xs, series: value_series(ys),
            });
        });
    }

    // ── Historical plot ─────────────────────────────────────────────────
    {
        let id = id_hist.clone();
        Effect::new(move |_| {
            if combo.collapsed.get() { charts::destroy_plot(&id); return; }
            let vol_mode = combo.vol_mode.get();
            let series = if vol_mode { combo.hist_vol.get() } else { combo.hist_price.get() };
            if series.is_empty() { charts::destroy_plot(&id); return; }
            let xs: Vec<f64> = series.iter().map(|(t, _)| *t).collect();
            let ys: Vec<f64> = series.iter().map(|(_, v)| *v).collect();
            defer_plot(id.clone(), LinePlot {
                title: if vol_mode { "Historical vol difference".into() } else { "Historical price difference".into() },
                x_label: "Date".into(),
                x_fmt: "time".into(),
                y_fmt: if vol_mode { "pct".into() } else { "usd".into() },
                x: xs,
                series: vec![Series {
                    label: "Combo".into(),
                    color: if vol_mode { "#c084fc".into() } else { "#34d399".into() },
                    y: ys,
                }],
            });
        });
    }

    // Fetch historical series for every leg and combine.
    let load_history = move |_| {
        let sym = combo.symbol.get_untracked().trim().to_uppercase();
        let legs = combo.legs.get_untracked();
        if sym.is_empty() || legs.is_empty() {
            combo.hist_status.set(HistStatus::Error("Set a symbol and at least one leg.".into()));
            return;
        }
        combo.hist_status.set(HistStatus::Loading);
        let tok = auth.token.get_untracked().unwrap_or_default();
        spawn_local(async move {
            let mut price_legs: Vec<(i32, Vec<(String, f64)>)> = vec![];
            let mut vol_legs: Vec<(i32, Vec<(String, f64)>)> = vec![];
            for l in &legs {
                let strike = match l.strike.get_untracked().trim().parse::<f64>() { Ok(v) => v, Err(_) => continue };
                let qty: i32 = l.quantity.get_untracked().trim().parse().unwrap_or(0);
                if qty == 0 { continue; }
                let expiry = l.expiry.get_untracked();
                let ty = match l.option_type.get_untracked() { OptionType::Call => "call", OptionType::Put => "put" };
                if let Ok(points) = market::fetch_option_history(&tok, &sym, &expiry, ty, strike).await {
                    let ps = points.iter().filter_map(|p| p.mid.map(|m| (p.t.clone(), m))).collect();
                    let vs = points.iter().filter_map(|p| p.implied_vol.map(|iv| (p.t.clone(), iv))).collect();
                    price_legs.push((qty, ps));
                    vol_legs.push((qty, vs));
                }
            }
            let to_pts = |v: Vec<(String, f64)>| -> Vec<(f64, f64)> {
                v.into_iter().filter_map(|(t, val)| date_to_unix(&t).map(|u| (u, val))).collect()
            };
            let pp = to_pts(combo::combine_series(&price_legs, true));
            let vp = to_pts(combo::combine_series(&vol_legs, false));
            let empty = pp.is_empty() && vp.is_empty();
            combo.hist_price.set(pp);
            combo.hist_vol.set(vp);
            combo.hist_status.set(if empty { HistStatus::Empty } else { HistStatus::Loaded });
        });
    };

    // Persist the combo to Supabase.
    let save_status = RwSignal::new(Option::<&'static str>::None);
    let save = move |_| {
        let record = combo.to_combo();
        let tok = auth.token.get_untracked().unwrap_or_default();
        let uid = auth.user_id.get_untracked().unwrap_or_default();
        save_status.set(Some("Saving…"));
        spawn_local(async move {
            match supabase::upsert_combo(&tok, &uid, &record).await {
                Ok(_) => save_status.set(Some("Saved ✓")),
                Err(_) => save_status.set(Some("Save failed")),
            }
        });
    };

    // Clear the saved indicator whenever a persisted field is edited (the first
    // run just establishes subscriptions, so it doesn't clear on load).
    Effect::new(move |prev: Option<()>| {
        combo.label.track();
        combo.symbol.track();
        combo.vol_mode.track();
        for l in combo.legs.get().iter() {
            l.option_type.track();
            l.strike.track();
            l.expiry.track();
            l.quantity.track();
        }
        if prev.is_some() {
            save_status.set(None);
        }
    });

    view! {
        <div class="bg-panel border border-border rounded-xl p-4 space-y-4">
            // ── Header ────────────────────────────────────────────────────
            <div class="flex items-center gap-3">
                <button
                    class="text-gray-500 hover:text-gray-300 text-sm"
                    on:click=move |_| combo.collapsed.update(|c| *c = !*c)
                >
                    {move || if combo.collapsed.get() { "▸" } else { "▾" }}
                </button>
                <input
                    class="flex-1 bg-surface border border-border rounded px-2 py-1 text-sm focus:outline-none focus:border-blue-500"
                    prop:value=move || combo.label.get()
                    on:input=move |ev| combo.label.set(event_target_value(&ev))
                    placeholder="Combo name (e.g. AAPL Jun→Sep roll)"
                />
                {move || save_status.get().map(|s| view! { <span class="text-xs text-gray-500 shrink-0">{s}</span> })}
                <button
                    class="text-xs text-blue-400 hover:text-blue-300 shrink-0"
                    on:click=save
                >"Save"</button>
                <button class="text-gray-600 hover:text-red-400 text-sm" on:click=move |_| on_remove()>"✕"</button>
            </div>

            {move || (!combo.collapsed.get()).then(|| {
                let id_price = id_price.clone();
                let id_vol = id_vol.clone();
                let id_rate = id_rate.clone();
                let id_time = id_time.clone();
                let id_hist = id_hist.clone();
                view! {
                    <div class="space-y-4 pl-6">
                        // ── Symbol + legs ─────────────────────────────────
                        <div class="flex gap-2 items-end">
                            <div class="w-32">
                                <Label text="Stock symbol" />
                                <input
                                    class="w-full bg-surface border border-border rounded px-2 py-1 text-sm focus:outline-none focus:border-blue-500"
                                    prop:value=move || combo.symbol.get()
                                    on:input=move |ev| combo.symbol.set(event_target_value(&ev).to_uppercase())
                                    placeholder="AAPL"
                                />
                            </div>
                        </div>

                        <div class="space-y-2">
                            <Hint>
                                "One row per contract. Use a negative quantity for a contract you're \
                                 selling — a roll is one negative row and one positive row."
                            </Hint>
                            <div class="grid grid-cols-[auto_1fr_1fr_auto_auto] gap-2 text-xs text-gray-500 font-sans">
                                <span>"Type"</span>
                                <span class="flex items-center gap-1">"Expires" <Info term="expiry" /></span>
                                <span class="flex items-center gap-1">"Strike" <Info term="strike" /></span>
                                <span class="flex items-center gap-1">"Qty" <Info term="contract" /></span>
                                <span></span>
                            </div>
                            {move || combo.legs.get().into_iter().enumerate().map(|(i, leg)| view! {
                                <div class="grid grid-cols-[auto_1fr_1fr_auto_auto] gap-2 items-center">
                                    <div class="flex rounded overflow-hidden border border-border text-xs">
                                        {[OptionType::Call, OptionType::Put].map(|t| view! {
                                            <button type="button"
                                                class=move || if leg.option_type.get() == t { "px-2 py-1 bg-blue-600 text-white" } else { "px-2 py-1 text-gray-400" }
                                                on:click=move |_| { leg.option_type.set(t); leg.strike.set(String::new()); }
                                            >{t.label()}</button>
                                        })}
                                    </div>
                                    <select class="bg-surface border border-border rounded px-2 py-1 text-sm focus:outline-none focus:border-blue-500"
                                        prop:value=move || leg.expiry.get()
                                        on:change=move |ev| { leg.expiry.set(event_target_value(&ev)); leg.strike.set(String::new()); }
                                    >
                                        <option value="">"— expiry —"</option>
                                        {move || expiries.get().into_iter()
                                            .map(|e| view! { <option value=e.clone()>{e.clone()}</option> })
                                            .collect_view()}
                                    </select>
                                    <select class="bg-surface border border-border rounded px-2 py-1 text-sm focus:outline-none focus:border-blue-500"
                                        prop:value=move || leg.strike.get()
                                        on:change=move |ev| leg.strike.set(event_target_value(&ev))
                                    >
                                        <option value="">"— strike —"</option>
                                        {move || {
                                            let ts = if leg.option_type.get() == OptionType::Call { "call" } else { "put" };
                                            let sel = leg.expiry.get();
                                            let mut ks: Vec<f64> = combo.meta.get().iter()
                                                .filter(|e| e.expiry == sel && e.option_type == ts)
                                                .map(|e| e.strike)
                                                .collect();
                                            ks.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                                            ks.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
                                            ks.into_iter().map(|s| {
                                                let val = format!("{}", s);
                                                view! { <option value=val.clone()>{format!("${:.0}", s)}</option> }
                                            }).collect_view()
                                        }}
                                    </select>
                                    <input class="w-16 bg-surface border border-border rounded px-2 py-1 text-sm focus:outline-none focus:border-blue-500"
                                        prop:value=move || leg.quantity.get()
                                        on:input=move |ev| leg.quantity.set(event_target_value(&ev))
                                        placeholder="1" />
                                    <button class="text-gray-600 hover:text-red-400 text-xs"
                                        on:click=move |_| combo.legs.update(|v| { if v.len() > 1 { v.remove(i); } })
                                    >"✕"</button>
                                </div>
                            }).collect_view()}
                            <button
                                class="text-xs text-blue-400 hover:text-blue-300"
                                on:click=move |_| combo.legs.update(|v| v.push(LegForm::new("1", OptionType::Call)))
                            >"+ Add leg"</button>
                        </div>

                        // ── Metric toggle (applies to all plots) ──────────
                        <div class="flex items-center gap-2">
                            <span class="text-xs text-gray-500 font-sans">"Chart the combination's"</span>
                            <div class="flex rounded overflow-hidden border border-border text-xs">
                                <button type="button"
                                    class=move || if !combo.vol_mode.get() { "px-3 py-1 bg-blue-600 text-white" } else { "px-3 py-1 text-gray-400" }
                                    on:click=move |_| combo.vol_mode.set(false)
                                >"Price"</button>
                                <button type="button"
                                    class=move || if combo.vol_mode.get() { "px-3 py-1 bg-blue-600 text-white" } else { "px-3 py-1 text-gray-400" }
                                    on:click=move |_| combo.vol_mode.set(true)
                                >"Vol"</button>
                            </div>
                        </div>

                        // ── Sliders ───────────────────────────────────────
                        <Hint>
                            "Drag these to ask what-if. The charts below redraw against whatever you set here."
                        </Hint>
                        <div class="grid grid-cols-3 gap-4 bg-surface border border-border rounded-lg p-3">
                            <div>
                                <label class="flex items-center gap-1 text-xs text-gray-400 mb-1 font-sans">
                                    "Stock price" <Info term="spot" />
                                    <span class="text-gray-200 font-mono">{move || combo.spot.get()}</span>
                                </label>
                                <input type="range"
                                    min=move || format!("{:.2}", (center.get() * 0.3).max(1.0))
                                    max=move || format!("{:.2}", center.get() * 1.7)
                                    step=move || format!("{:.2}", (center.get() / 200.0).max(0.01))
                                    class="w-full"
                                    prop:value=move || combo.spot.get()
                                    on:input=move |ev| combo.spot.set(event_target_value(&ev))
                                />
                            </div>
                            <div>
                                <label class="flex items-center gap-1 text-xs text-gray-400 mb-1 font-sans">
                                    "Expected swing" <Info term="volatility" />
                                    <span class="text-gray-200 font-mono">{move || format!("{}%", combo.vol.get())}</span>
                                </label>
                                <input type="range" min="1" max="150" step="1" class="w-full"
                                    prop:value=move || combo.vol.get()
                                    on:input=move |ev| combo.vol.set(event_target_value(&ev))
                                />
                            </div>
                            <div>
                                <label class="flex items-center gap-1 text-xs text-gray-400 mb-1 font-sans">
                                    "Interest rate" <Info term="risk-free-rate" />
                                    <span class="text-gray-200 font-mono">{move || format!("{}%", combo.rate.get())}</span>
                                </label>
                                <input type="range" min="0" max="12" step="0.25" class="w-full"
                                    prop:value=move || combo.rate.get()
                                    on:input=move |ev| combo.rate.set(event_target_value(&ev))
                                />
                            </div>
                        </div>

                        // ── Sensitivity plots (Price metric only) ─────────
                        {move || if combo.vol_mode.get() {
                            view! {
                                <Callout>
                                    "What-if charts aren't available in volatility mode — they need the \
                                     volatility map, which isn't built yet. The historical chart below \
                                     still works. Switch back to Price for the what-if charts."
                                </Callout>
                            }.into_any()
                        } else {
                            let (a, b, c, d) = (id_price.clone(), id_vol.clone(), id_rate.clone(), id_time.clone());
                            view! {
                                <div>
                                    {move || legs_parsed.get().is_empty().then(|| view! {
                                        <Hint>
                                            "Choose an expiry and strike for each row above and the what-if \
                                             charts will appear here."
                                        </Hint>
                                    })}
                                    <div class="grid grid-cols-1 sm:grid-cols-2 gap-3">
                                        {plot_box(a.clone())}
                                        {plot_box(b.clone())}
                                        {plot_box(c.clone())}
                                        {plot_box(d.clone())}
                                    </div>
                                </div>
                            }.into_any()
                        }}

                        // ── Historical ────────────────────────────────────
                        <div class="space-y-2">
                            <div class="flex items-center gap-3">
                                <button
                                    class="bg-blue-600 hover:bg-blue-500 px-3 py-1.5 rounded text-xs font-medium transition-colors"
                                    on:click=load_history
                                >"Show past prices"</button>
                                {move || match combo.hist_status.get() {
                                    HistStatus::Loading => view! { <span class="text-xs text-gray-400 font-sans">"Loading…"</span> }.into_any(),
                                    HistStatus::Empty   => view! {
                                        <span class="text-xs text-amber-400 font-sans">
                                            "No past prices recorded for these contracts yet."
                                        </span>
                                    }.into_any(),
                                    HistStatus::Error(e) => view! { <span class="text-xs text-red-400 font-sans">{e}</span> }.into_any(),
                                    _ => ().into_any(),
                                }}
                            </div>
                            {plot_box(id_hist.clone())}
                        </div>
                    </div>
                }
            })}
        </div>
    }
}

// ── Shared small components ──────────────────────────────────────────────────

/// A bordered container holding a uPlot target div, addressed by `id`.
fn plot_box(id: String) -> impl IntoView {
    view! {
        <div class="bg-panel border border-border rounded-lg p-2">
            <div id=id class="w-full"></div>
        </div>
    }
}

#[component]
fn InputField(
    label: &'static str,
    #[prop(optional)] term: Option<&'static str>,
    signal: RwSignal<String>,
    placeholder: &'static str,
) -> impl IntoView {
    view! {
        <div>
            <Label text=label term=term />
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
fn GreekRow(label: &'static str, term: &'static str, value: String) -> impl IntoView {
    view! {
        <div class="flex justify-between items-center border-b border-border py-2 gap-3">
            <span class="flex items-center gap-1.5 text-xs text-gray-400 font-sans">
                {label} <Info term=term />
            </span>
            <span class="text-sm font-medium text-blue-200 font-mono shrink-0">{value}</span>
        </div>
    }
}
