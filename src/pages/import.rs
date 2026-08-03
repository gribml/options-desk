//! Bring existing holdings in as dated lots.
//!
//! The point of this page is the acquisition date. A position entered as a
//! single snapshot has no purchase date, so every gain on it reads as
//! short-term — which for long-held stock overstates the tax badly. Importing
//! lots gives each parcel of shares its real date, and the holding period falls
//! out correctly.
//!
//! Three steps: choose a source, say which column is which, then review and fix.
//! A CSV only ever *pre-fills* the review table — the table itself is editable
//! from nothing, so someone without an export can type their lots in by hand.

use std::collections::HashMap;

use leptos::prelude::*;
use uuid::Uuid;
use wasm_bindgen_futures::spawn_local;

use crate::api::supabase;
use crate::app::AuthState;
use crate::components::ui::{Callout, Hint, Info, Label, Tone};
use crate::format::fmt_cash;
use crate::models::import::{
    draft_lots, guess_column, parse_date, parse_money, parse_quantity, split_csv, BasisKind,
    DraftLot, Role,
};
use crate::models::position::{Position, PositionEntryMode, Trade};

#[derive(Clone, Copy, PartialEq)]
enum Step {
    Source,
    Map,
    Review,
}

/// What to do about a ticker that's already in the portfolio.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Conflict {
    Replace,
    Append,
    Skip,
}

impl Conflict {
    fn label(self) -> &'static str {
        match self {
            Conflict::Replace => "Replace it",
            Conflict::Append => "Add to it",
            Conflict::Skip => "Skip",
        }
    }
}

const INPUT: &str = "w-full bg-surface border border-border rounded px-2 py-1 text-sm \
                     focus:outline-none focus:border-blue-500";

#[component]
pub fn ImportPage() -> impl IntoView {
    let auth = use_context::<AuthState>().expect("AuthState missing");
    let step = RwSignal::new(Step::Source);

    // Raw CSV state
    let rows = RwSignal::new(Vec::<Vec<String>>::new());
    let file_name = RwSignal::new(String::new());
    let read_err = RwSignal::new(Option::<String>::None);
    let has_header = RwSignal::new(true);

    // Mapping state
    let date_col = RwSignal::new(0usize);
    let qty_col = RwSignal::new(0usize);
    let price_col = RwSignal::new(0usize);
    let symbol_col = RwSignal::new(Option::<usize>::None);
    let one_symbol = RwSignal::new(String::new());
    let basis = RwSignal::new(BasisKind::PerShare);

    // Review state
    let lots = RwSignal::new(Vec::<RwSignal<DraftLot>>::new());
    let existing = RwSignal::new(Vec::<Position>::new());
    let conflicts = RwSignal::new(HashMap::<String, Conflict>::new());
    let saving = RwSignal::new(false);
    let save_err = RwSignal::new(Option::<String>::None);

    // Existing positions, to spot ticker collisions in the review step.
    Effect::new(move |_| {
        let (Some(tok), Some(uid)) = (auth.token.get(), auth.user_id.get()) else { return };
        spawn_local(async move {
            if let Ok(ps) = supabase::fetch_positions(&tok, &uid).await {
                existing.set(ps);
            }
        });
    });

    let headers = Memo::new(move |_| {
        let r = rows.get();
        match (has_header.get(), r.first()) {
            (true, Some(h)) => h.clone(),
            (_, Some(h)) => (1..=h.len()).map(|i| format!("Column {i}")).collect(),
            _ => vec![],
        }
    });

    // Read the chosen file, split it, and pre-select columns by header name.
    let on_file = move |ev: web_sys::Event| {
        use wasm_bindgen::JsCast as _;
        let input: web_sys::HtmlInputElement = match ev.target().and_then(|t| t.dyn_into().ok()) {
            Some(i) => i,
            None => return,
        };
        let Some(file) = input.files().and_then(|f| f.get(0)) else { return };
        file_name.set(file.name());
        read_err.set(None);
        spawn_local(async move {
            match gloo_file::futures::read_as_text(&gloo_file::Blob::from(file)).await {
                Ok(text) => {
                    let parsed = split_csv(&text);
                    if parsed.is_empty() {
                        read_err.set(Some("That file has no rows in it.".into()));
                        return;
                    }
                    let h = parsed[0].clone();
                    date_col.set(guess_column(&h, Role::Date).unwrap_or(0));
                    qty_col.set(guess_column(&h, Role::Quantity).unwrap_or(0));
                    price_col.set(guess_column(&h, Role::Price).unwrap_or(0));
                    symbol_col.set(guess_column(&h, Role::Symbol));
                    rows.set(parsed);
                    step.set(Step::Map);
                }
                Err(e) => read_err.set(Some(format!("Couldn’t read that file: {e}"))),
            }
        });
    };

    // Mapping → editable rows.
    let build_lots = move || {
        let drafts = draft_lots(
            &rows.get_untracked(),
            has_header.get_untracked(),
            date_col.get_untracked(),
            qty_col.get_untracked(),
            price_col.get_untracked(),
            symbol_col.get_untracked(),
            &one_symbol.get_untracked(),
            basis.get_untracked(),
        );
        lots.set(drafts.into_iter().map(RwSignal::new).collect());
        step.set(Step::Review);
    };

    let start_blank = move |_| {
        lots.set(vec![RwSignal::new(DraftLot::blank())]);
        step.set(Step::Review);
    };

    // Rows that are complete enough to import, grouped by ticker.
    let grouped = Memo::new(move |_| {
        let mut map: HashMap<String, Vec<(chrono::NaiveDate, i32, f64)>> = HashMap::new();
        for l in lots.get() {
            if let Some((sym, d, q, p)) = l.get().resolved() {
                map.entry(sym).or_default().push((d, q, p));
            }
        }
        let mut out: Vec<(String, Vec<(chrono::NaiveDate, i32, f64)>)> = map.into_iter().collect();
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    });

    let blocked = Memo::new(move |_| {
        lots.get().iter().filter(|l| l.get().issues().blocking()).count()
    });

    let do_import = move |_| {
        let groups = grouped.get_untracked();
        if groups.is_empty() {
            save_err.set(Some("Nothing to import yet.".into()));
            return;
        }
        let (Some(tok), Some(uid)) = (auth.token.get_untracked(), auth.user_id.get_untracked())
        else {
            save_err.set(Some("Not signed in.".into()));
            return;
        };
        save_err.set(None);
        saving.set(true);

        let prior = existing.get_untracked();
        let choices = conflicts.get_untracked();

        spawn_local(async move {
            for (sym, parcels) in groups {
                let clash = prior.iter().find(|p| {
                    p.symbol == sym && p.option_spec.is_none()
                });
                let choice = choices.get(&sym).copied().unwrap_or(Conflict::Replace);
                if clash.is_some() && choice == Conflict::Skip {
                    continue;
                }

                let new_trades: Vec<Trade> = parcels
                    .iter()
                    .map(|(d, q, p)| Trade {
                        id: Uuid::new_v4(),
                        date: *d,
                        quantity: *q,
                        price: *p,
                    })
                    .collect();

                let mut pos = match (clash, choice) {
                    // Keep the existing row's id so anything referencing it survives.
                    (Some(p), Conflict::Append) if p.entry_mode == PositionEntryMode::TradeLog => {
                        let mut p = p.clone();
                        p.trades.extend(new_trades);
                        p
                    }
                    (Some(p), _) => {
                        let mut p = p.clone();
                        p.trades = new_trades;
                        p.entry_mode = PositionEntryMode::TradeLog;
                        // Snapshot fields would otherwise double-count.
                        p.quantity = 0;
                        p.cost_basis = 0.0;
                        p
                    }
                    (None, _) => {
                        let mut p = Position::new_stock(&sym, 0, 0.0);
                        p.entry_mode = PositionEntryMode::TradeLog;
                        p.trades = new_trades;
                        p
                    }
                };
                pos.entry_mode = PositionEntryMode::TradeLog;

                if let Err(e) = supabase::upsert_position(&tok, &uid, &pos).await {
                    save_err.set(Some(e));
                    saving.set(false);
                    return;
                }
            }
            saving.set(false);
            // Full navigation rather than a router push: the portfolio reloads
            // its positions from scratch, so the imported lots are there.
            if let Some(w) = web_sys::window() {
                let _ = w
                    .location()
                    .set_href(&format!("{}/portfolio", crate::config::APP_BASE));
            }
        });
    };

    view! {
        <div class="max-w-4xl mx-auto space-y-6">
            <div>
                <h1 class="text-xl font-semibold">"Import holdings"</h1>
                <p class="text-xs text-gray-500 mt-1 font-sans">
                    "Bring in each parcel of shares with the date you bought it, so gains held over \
                     a year are taxed at the lower long-term rate instead of as income."
                </p>
            </div>

            <StepBar step=step />

            {move || match step.get() {
                Step::Source => view! {
                    <SourceStep on_file=on_file on_blank=start_blank
                        file_name=file_name read_err=read_err />
                }.into_any(),
                Step::Map => view! {
                    <MapStep
                        headers=headers rows=rows has_header=has_header
                        date_col=date_col qty_col=qty_col price_col=price_col
                        symbol_col=symbol_col one_symbol=one_symbol basis=basis
                        on_back=move |_| step.set(Step::Source)
                        on_next=move |_| build_lots()
                    />
                }.into_any(),
                Step::Review => view! {
                    <ReviewStep
                        lots=lots grouped=grouped blocked=blocked
                        existing=existing conflicts=conflicts
                        saving=saving save_err=save_err
                        on_back=move |_| step.set(if rows.get_untracked().is_empty() {
                            Step::Source
                        } else {
                            Step::Map
                        })
                        on_import=do_import
                    />
                }.into_any(),
            }}
        </div>
    }
}

#[component]
fn StepBar(step: RwSignal<Step>) -> impl IntoView {
    let item = move |s: Step, n: &'static str, label: &'static str| {
        let active = move || step.get() == s;
        view! {
            <div class="flex items-center gap-2">
                <span class=move || format!(
                    "w-5 h-5 rounded-full text-[10px] flex items-center justify-center font-sans {}",
                    if active() { "bg-blue-600 text-white" } else { "bg-surface border border-border text-gray-500" },
                )>{n}</span>
                <span class=move || format!(
                    "text-xs font-sans {}",
                    if active() { "text-gray-200" } else { "text-gray-500" },
                )>{label}</span>
            </div>
        }
    };
    view! {
        <div class="flex items-center gap-4 border-b border-border pb-3">
            {item(Step::Source, "1", "Source")}
            <span class="text-gray-700 text-xs">"→"</span>
            {item(Step::Map, "2", "Match columns")}
            <span class="text-gray-700 text-xs">"→"</span>
            {item(Step::Review, "3", "Check and fix")}
        </div>
    }
}

#[component]
fn SourceStep(
    on_file: impl Fn(web_sys::Event) + 'static,
    on_blank: impl Fn(web_sys::MouseEvent) + 'static,
    file_name: RwSignal<String>,
    read_err: RwSignal<Option<String>>,
) -> impl IntoView {
    view! {
        <div class="space-y-4">
            <div class="bg-panel border border-border rounded-xl p-6 space-y-3">
                <p class="text-sm font-medium text-gray-200 font-sans">"Upload a CSV"</p>
                <Hint>
                    "Most brokers can export your holdings with a cost-basis or realised-gain \
                     report. Any file with a date, a quantity and a price will do — you'll say \
                     which column is which next."
                </Hint>
                <input
                    type="file"
                    accept=".csv,text/csv"
                    class="block w-full text-sm text-gray-400 font-sans file:mr-3 file:py-2 file:px-4 \
                           file:rounded file:border-0 file:text-sm file:font-medium \
                           file:bg-blue-600 file:text-white hover:file:bg-blue-500"
                    on:change=on_file
                />
                {move || (!file_name.get().is_empty()).then(|| view! {
                    <p class="text-xs text-gray-500 font-sans">"Read " {file_name.get()}</p>
                })}
                {move || read_err.get().map(|e| view! {
                    <Callout tone=Tone::Warn>{e}</Callout>
                })}
            </div>

            <div class="bg-panel border border-border rounded-xl p-6 space-y-3">
                <p class="text-sm font-medium text-gray-200 font-sans">"Or enter them yourself"</p>
                <Hint>
                    "No export handy? Start with an empty table and type each purchase in. \
                     You get the same result — it's the same table the CSV fills."
                </Hint>
                <button
                    class="px-4 py-2 rounded text-sm font-medium border border-border text-gray-200 hover:border-gray-500 transition-colors"
                    on:click=on_blank
                >"Enter lots by hand"</button>
            </div>
        </div>
    }
}

#[component]
fn MapStep(
    headers: Memo<Vec<String>>,
    rows: RwSignal<Vec<Vec<String>>>,
    has_header: RwSignal<bool>,
    date_col: RwSignal<usize>,
    qty_col: RwSignal<usize>,
    price_col: RwSignal<usize>,
    symbol_col: RwSignal<Option<usize>>,
    one_symbol: RwSignal<String>,
    basis: RwSignal<BasisKind>,
    on_back: impl Fn(web_sys::MouseEvent) + 'static,
    on_next: impl Fn(web_sys::MouseEvent) + 'static,
) -> impl IntoView {
    let picker = move |label: &'static str, term: Option<&'static str>, sig: RwSignal<usize>| {
        view! {
            <div>
                <Label text=label term=term />
                <select
                    class=INPUT
                    prop:value=move || sig.get().to_string()
                    on:change=move |ev| {
                        if let Ok(i) = event_target_value(&ev).parse::<usize>() { sig.set(i); }
                    }
                >
                    {move || headers.get().into_iter().enumerate()
                        .map(|(i, h)| view! { <option value=i.to_string()>{h}</option> })
                        .collect_view()}
                </select>
            </div>
        }
    };

    view! {
        <div class="space-y-5">
            <div class="bg-panel border border-border rounded-xl p-5 space-y-4">
                <div class="flex items-center gap-2">
                    <input type="checkbox" id="hdr" class="accent-blue-600"
                        prop:checked=move || has_header.get()
                        on:change=move |ev| has_header.set(event_target_checked(&ev)) />
                    <label for="hdr" class="text-xs text-gray-400 font-sans">
                        "The first row is column names, not data"
                    </label>
                </div>

                <div class="grid grid-cols-1 sm:grid-cols-3 gap-3">
                    {picker("Date acquired", Some("holding-period"), date_col)}
                    {picker("Quantity", None, qty_col)}
                    {picker("Cost", Some("cost-basis"), price_col)}
                </div>

                <div class="space-y-1.5">
                    <span class="text-xs text-gray-400 font-sans">"That cost column holds…"</span>
                    <div class="flex rounded overflow-hidden border border-border text-xs w-fit">
                        {[(BasisKind::PerShare, "Price of one share"), (BasisKind::TotalCost, "Total paid for the lot")]
                            .map(|(k, l)| view! {
                                <button type="button"
                                    class=move || if basis.get() == k {
                                        "px-3 py-1 bg-blue-600 text-white"
                                    } else {
                                        "px-3 py-1 text-gray-400 hover:text-gray-200"
                                    }
                                    on:click=move |_| basis.set(k)
                                >{l}</button>
                            })}
                    </div>
                    <Hint>
                        "Brokers differ. If a lot of 100 shares shows about the same number as one \
                         share's price, it's per share; if it's roughly a hundred times bigger, \
                         it's the total."
                    </Hint>
                </div>
            </div>

            <div class="bg-panel border border-border rounded-xl p-5 space-y-3">
                <span class="text-xs text-gray-400 font-sans">"Which stock are these lots for?"</span>
                <div class="flex rounded overflow-hidden border border-border text-xs w-fit">
                    <button type="button"
                        class=move || if symbol_col.get().is_some() {
                            "px-3 py-1 bg-blue-600 text-white"
                        } else { "px-3 py-1 text-gray-400 hover:text-gray-200" }
                        on:click=move |_| symbol_col.set(Some(0))
                    >"A column in the file"</button>
                    <button type="button"
                        class=move || if symbol_col.get().is_none() {
                            "px-3 py-1 bg-blue-600 text-white"
                        } else { "px-3 py-1 text-gray-400 hover:text-gray-200" }
                        on:click=move |_| symbol_col.set(None)
                    >"All the same stock"</button>
                </div>
                {move || match symbol_col.get() {
                    Some(cur) => view! {
                        <select
                            class=format!("{INPUT} max-w-xs")
                            prop:value=cur.to_string()
                            on:change=move |ev| {
                                if let Ok(i) = event_target_value(&ev).parse::<usize>() {
                                    symbol_col.set(Some(i));
                                }
                            }
                        >
                            {move || headers.get().into_iter().enumerate()
                                .map(|(i, h)| view! { <option value=i.to_string()>{h}</option> })
                                .collect_view()}
                        </select>
                    }.into_any(),
                    None => view! {
                        <input class=format!("{INPUT} max-w-xs") placeholder="e.g. PLTR"
                            prop:value=move || one_symbol.get()
                            on:input=move |ev| one_symbol.set(event_target_value(&ev).to_uppercase()) />
                    }.into_any(),
                }}
            </div>

            // A few real rows, so a wrong mapping is obvious before going on.
            {move || {
                let r = rows.get();
                let body: Vec<Vec<String>> = if has_header.get() { r.iter().skip(1).take(3).cloned().collect() }
                                             else { r.iter().take(3).cloned().collect() };
                (!body.is_empty()).then(|| view! {
                    <div class="bg-panel border border-border rounded-xl p-5 space-y-2 overflow-x-auto">
                        <p class="text-xs text-gray-400 font-sans">"What that gives you"</p>
                        <table class="text-xs font-mono w-full">
                            <thead>
                                <tr class="text-gray-500 text-left">
                                    <th class="pr-4 pb-1 font-sans font-normal">"Ticker"</th>
                                    <th class="pr-4 pb-1 font-sans font-normal">"Acquired"</th>
                                    <th class="pr-4 pb-1 font-sans font-normal">"Qty"</th>
                                    <th class="pb-1 font-sans font-normal">"Per share"</th>
                                </tr>
                            </thead>
                            <tbody>
                                {body.into_iter().map(|row| {
                                    let g = |i: usize| row.get(i).cloned().unwrap_or_default();
                                    let sym = match symbol_col.get() {
                                        Some(i) => g(i).to_uppercase(),
                                        None => one_symbol.get(),
                                    };
                                    let qty = g(qty_col.get());
                                    let raw = g(price_col.get());
                                    let per = match (basis.get(), parse_money(&raw), parse_quantity(&qty)) {
                                        (BasisKind::TotalCost, Some(v), Some(q)) if q.abs() > 1e-9 =>
                                            format!("{:.4}", v / q),
                                        _ => parse_money(&raw).map(|v| format!("{v:.4}")).unwrap_or(raw),
                                    };
                                    let d = g(date_col.get());
                                    let shown = parse_date(&d)
                                        .map(|x| x.format("%-d %b %Y").to_string())
                                        .unwrap_or_else(|| format!("? {d}"));
                                    view! {
                                        <tr class="text-gray-300">
                                            <td class="pr-4 py-0.5">{if sym.is_empty() { "—".into() } else { sym }}</td>
                                            <td class="pr-4 py-0.5">{shown}</td>
                                            <td class="pr-4 py-0.5">{qty}</td>
                                            <td class="py-0.5">{per}</td>
                                        </tr>
                                    }
                                }).collect_view()}
                            </tbody>
                        </table>
                    </div>
                })
            }}

            <div class="flex gap-2">
                <button class="px-4 py-2 rounded text-sm border border-border text-gray-300 hover:border-gray-500 transition-colors"
                    on:click=on_back>"Back"</button>
                <button class="bg-blue-600 hover:bg-blue-500 px-4 py-2 rounded text-sm font-medium transition-colors"
                    on:click=on_next>"Check the rows →"</button>
            </div>
        </div>
    }
}

/// One editable cell in the review table, outlined red with the reason beneath
/// when the value can't be read.
#[component]
fn LotCell<V, S, E>(value: V, set: S, err: E, ph: &'static str) -> impl IntoView
where
    V: Fn() -> String + Send + Sync + 'static,
    S: Fn(String) + Send + Sync + 'static,
    E: Fn() -> Option<&'static str> + Copy + Send + Sync + 'static,
{
    view! {
        <div>
            <input
                class=move || format!(
                    "{INPUT} {}",
                    if err().is_some() { "border-red-500/70" } else { "" },
                )
                prop:value=move || value()
                on:input=move |ev| set(event_target_value(&ev))
                placeholder=ph
            />
            {move || err().map(|e| view! {
                <span class="block text-[10px] text-red-400 font-sans mt-0.5">{e}</span>
            })}
        </div>
    }
}

#[component]
fn ReviewStep(
    lots: RwSignal<Vec<RwSignal<DraftLot>>>,
    grouped: Memo<Vec<(String, Vec<(chrono::NaiveDate, i32, f64)>)>>,
    blocked: Memo<usize>,
    existing: RwSignal<Vec<Position>>,
    conflicts: RwSignal<HashMap<String, Conflict>>,
    saving: RwSignal<bool>,
    save_err: RwSignal<Option<String>>,
    on_back: impl Fn(web_sys::MouseEvent) + 'static,
    on_import: impl Fn(web_sys::MouseEvent) + 'static,
) -> impl IntoView {
    view! {
        <div class="space-y-5">
            <Hint>
                "Fix anything flagged, and delete rows for shares you've already sold elsewhere. \
                 Nothing is saved until you press Import."
            </Hint>

            {move || (blocked.get() > 0).then(|| view! {
                <Callout tone=Tone::Warn>
                    {format!(
                        "{} row{} still need attention and won't be imported until fixed.",
                        blocked.get(),
                        if blocked.get() == 1 { "" } else { "s" },
                    )}
                </Callout>
            })}

            <div class="bg-panel border border-border rounded-xl p-4 space-y-2 overflow-x-auto">
                <div class="grid grid-cols-[6rem_9rem_7rem_8rem_auto] gap-2 text-xs text-gray-500 font-sans">
                    <span>"Ticker"</span>
                    <span class="flex items-center gap-1">"Acquired" <Info term="holding-period" /></span>
                    <span>"Quantity"</span>
                    <span class="flex items-center gap-1">"Per share" <Info term="cost-basis" /></span>
                    <span></span>
                </div>

                {move || lots.get().into_iter().enumerate().map(|(i, lot)| {
                    let iss = Memo::new(move |_| lot.get().issues());
                    view! {
                        <div class="grid grid-cols-[6rem_9rem_7rem_8rem_auto] gap-2 items-start">
                            <LotCell
                                value=move || lot.get().symbol
                                set=move |v: String| lot.update(|l| l.symbol = v.to_uppercase())
                                err=move || iss.get().symbol
                                ph="PLTR"
                            />
                            <LotCell
                                value=move || lot.get().date
                                set=move |v: String| lot.update(|l| l.date = v)
                                err=move || iss.get().date
                                ph="2021-03-15"
                            />
                            <LotCell
                                value=move || lot.get().quantity
                                set=move |v: String| lot.update(|l| l.quantity = v)
                                err=move || iss.get().quantity
                                ph="100"
                            />
                            <LotCell
                                value=move || lot.get().price
                                set=move |v: String| lot.update(|l| l.price = v)
                                err=move || iss.get().price
                                ph="15.25"
                            />
                            <div class="flex items-center gap-2 pt-1">
                                {move || iss.get().warning.map(|w| view! {
                                    <span class="text-[10px] text-amber-400 font-sans">{w}</span>
                                })}
                                <button
                                    class="text-gray-600 hover:text-red-400 text-xs ml-auto"
                                    title="Remove this lot"
                                    on:click=move |_| lots.update(|v| { v.remove(i); })
                                >"✕"</button>
                            </div>
                        </div>
                    }
                }).collect_view()}

                <button
                    class="text-xs text-blue-400 hover:text-blue-300 font-sans"
                    on:click=move |_| lots.update(|v| v.push(RwSignal::new(DraftLot::blank())))
                >"+ add a lot"</button>
            </div>

            // Per-ticker outcome, including what happens to anything already held.
            {move || {
                let groups = grouped.get();
                (!groups.is_empty()).then(|| view! {
                    <div class="bg-panel border border-border rounded-xl p-4 space-y-3">
                        <p class="text-xs font-medium text-gray-300 font-sans">"What you'll end up with"</p>
                        {groups.into_iter().map(|(sym, parcels)| {
                            let shares: i32 = parcels.iter().map(|(_, q, _)| *q).sum();
                            let cost: f64 = parcels.iter().map(|(_, q, p)| *q as f64 * p).sum();
                            let avg = if shares != 0 { cost / shares as f64 } else { 0.0 };
                            let n = parcels.len();
                            let sym2 = sym.clone();
                            let clash = existing.get().into_iter().find(|p| {
                                p.symbol == sym2 && p.option_spec.is_none()
                            });
                            view! {
                                <div class="border-t border-border pt-3 first:border-0 first:pt-0 space-y-1.5">
                                    <div class="flex items-baseline justify-between gap-3 text-sm">
                                        <span class="font-semibold">{sym.clone()}</span>
                                        <span class="text-gray-400 font-mono text-xs">
                                            {format!("{n} lot{} · {shares} shares · avg ", if n == 1 { "" } else { "s" })}
                                            {format!("${avg:.2}")}
                                            {format!(" · {} total", fmt_cash(cost))}
                                        </span>
                                    </div>
                                    {clash.map(|c| {
                                        let sym3 = sym.clone();
                                        let held = c.effective_quantity();
                                        let was_snapshot = c.entry_mode == PositionEntryMode::Snapshot;
                                        view! {
                                            <div class="space-y-1.5">
                                                <p class="text-xs text-amber-400 font-sans">
                                                    {format!(
                                                        "Already in your portfolio: {held} shares{}.",
                                                        if was_snapshot { " entered as a single total" } else { " as lots" },
                                                    )}
                                                </p>
                                                <div class="flex rounded overflow-hidden border border-border text-xs w-fit">
                                                    // "Add to it" is withheld for a snapshot position:
                                                    // its quantity is a standalone total, so appending
                                                    // lots would count the same shares twice.
                                                    {if was_snapshot {
                                                        vec![Conflict::Replace, Conflict::Skip]
                                                    } else {
                                                        vec![Conflict::Replace, Conflict::Append, Conflict::Skip]
                                                    }.into_iter().map(|c| {
                                                        let sym4 = sym3.clone();
                                                        let sym5 = sym3.clone();
                                                        view! {
                                                            <button type="button"
                                                                class=move || {
                                                                    let cur = conflicts.get().get(&sym4).copied()
                                                                        .unwrap_or(Conflict::Replace);
                                                                    if cur == c { "px-2.5 py-1 bg-blue-600 text-white" }
                                                                    else { "px-2.5 py-1 text-gray-400 hover:text-gray-200" }
                                                                }
                                                                on:click=move |_| conflicts.update(|m| {
                                                                    m.insert(sym5.clone(), c);
                                                                })
                                                            >{c.label()}</button>
                                                        }
                                                    }).collect_view()}
                                                </div>
                                                {was_snapshot.then(|| view! {
                                                    <Hint>
                                                        "Replacing is normally right here: the existing entry has no \
                                                         purchase dates, so its gain is being treated as short-term. \
                                                         Adding to it would count the same shares twice."
                                                    </Hint>
                                                })}
                                            </div>
                                        }
                                    })}
                                </div>
                            }
                        }).collect_view()}
                    </div>
                })
            }}

            {move || save_err.get().map(|e| view! { <Callout tone=Tone::Warn>{e}</Callout> })}

            <div class="flex gap-2 items-center">
                <button class="px-4 py-2 rounded text-sm border border-border text-gray-300 hover:border-gray-500 transition-colors"
                    on:click=on_back>"Back"</button>
                <button
                    class="bg-blue-600 hover:bg-blue-500 disabled:opacity-50 px-4 py-2 rounded text-sm font-medium transition-colors"
                    prop:disabled=move || saving.get() || grouped.get().is_empty()
                    on:click=on_import
                >
                    {move || if saving.get() { "Importing…" } else { "Import" }}
                </button>
                {move || {
                    let n: usize = grouped.get().iter().map(|(_, v)| v.len()).sum();
                    (n > 0).then(|| view! {
                        <span class="text-xs text-gray-500 font-sans">
                            {format!("{n} lot{} ready", if n == 1 { "" } else { "s" })}
                        </span>
                    })
                }}
            </div>
        </div>
    }
}
