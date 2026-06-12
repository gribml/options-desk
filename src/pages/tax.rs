use chrono::{Datelike, Utc};
use leptos::prelude::*;
use uuid::Uuid;
use wasm_bindgen_futures::spawn_local;

use crate::api::supabase;
use crate::app::AuthState;
use crate::format::fmt_cash;
use crate::models::tax::{DeductionChoice, FilingStatus, TaxProfile, TaxRevision};

#[component]
pub fn TaxPage() -> impl IntoView {
    let auth = use_context::<AuthState>().expect("AuthState missing");
    let profiles = RwSignal::new(Vec::<TaxProfile>::new());
    let loading = RwSignal::new(true);
    let fetch_err = RwSignal::new(Option::<String>::None);
    let added_years = RwSignal::new(Vec::<u16>::new());
    let new_year_input = RwSignal::new(String::new());

    Effect::new(move |_| {
        let token = auth.token.get();
        let user_id = auth.user_id.get();
        if let (Some(tok), Some(uid)) = (token, user_id) {
            fetch_err.set(None);
            spawn_local(async move {
                match supabase::fetch_tax_profiles(&tok, &uid).await {
                    Ok(p) => profiles.set(p),
                    Err(e) => fetch_err.set(Some(e)),
                }
                loading.set(false);
            });
        } else {
            loading.set(false);
        }
    });

    let current_year = Utc::now().year() as u16;

    // Years to display = current year ∪ years with a profile ∪ user-added years.
    let years = move || {
        let mut ys: Vec<u16> = vec![current_year];
        for p in profiles.get() {
            ys.push(p.tax_year);
        }
        ys.extend(added_years.get());
        ys.sort_unstable();
        ys.dedup();
        ys.reverse();
        ys
    };

    let add_year = move |_| {
        if let Ok(y) = new_year_input.get().trim().parse::<u16>() {
            if y >= 1990 && y <= 2100 {
                added_years.update(|v| {
                    if !v.contains(&y) {
                        v.push(y);
                    }
                });
                new_year_input.set(String::new());
            }
        }
    };

    view! {
        <div class="space-y-6">
            <div class="flex items-center justify-between">
                <h1 class="text-lg font-semibold">"Taxes"</h1>
                <div class="flex items-center gap-2">
                    <input
                        class="w-24 bg-surface border border-border rounded px-2 py-1 text-sm focus:outline-none focus:border-blue-500"
                        prop:value=move || new_year_input.get()
                        on:input=move |ev| new_year_input.set(event_target_value(&ev))
                        placeholder="Year"
                    />
                    <button
                        class="text-xs px-3 py-1.5 rounded bg-surface border border-border hover:border-blue-500 transition-colors"
                        on:click=add_year
                    >
                        "Add year"
                    </button>
                </div>
            </div>

            <p class="text-xs text-gray-500">
                "Enter your federal income profile for each tax year. Scenario and portfolio tax estimates use the latest values you save here."
            </p>

            {move || loading.get().then(|| view! { <p class="text-sm text-gray-400">"Loading…"</p> })}
            {move || fetch_err.get().map(|e| view! { <p class="text-sm text-red-400">{e}</p> })}

            {move || {
                years().into_iter().map(|year| {
                    let existing = profiles.get().into_iter().find(|p| p.tax_year == year);
                    let default_expanded = year == current_year;
                    view! {
                        <YearSection
                            auth=auth
                            year=year
                            existing=existing
                            default_expanded=default_expanded
                            on_saved=move |saved: TaxProfile| {
                                profiles.update(|ps| {
                                    if let Some(slot) = ps.iter_mut().find(|p| p.tax_year == saved.tax_year) {
                                        *slot = saved;
                                    } else {
                                        ps.push(saved);
                                    }
                                });
                            }
                        />
                    }
                }).collect_view()
            }}
        </div>
    }
}

#[component]
fn YearSection(
    auth: AuthState,
    year: u16,
    existing: Option<TaxProfile>,
    default_expanded: bool,
    #[prop(into)] on_saved: Callback<TaxProfile>,
) -> impl IntoView {
    let expanded = RwSignal::new(default_expanded);
    let profile_id = RwSignal::new(existing.as_ref().map(|p| p.id));
    let revisions = RwSignal::new(existing.as_ref().map(|p| p.revisions.clone()).unwrap_or_default());

    let seed = existing.as_ref().and_then(|p| p.current().cloned()).unwrap_or_default();

    let filing_status = RwSignal::new(seed.filing_status);
    let deduction_choice = RwSignal::new(seed.deduction_choice);
    let w2 = RwSignal::new(money_str(seed.w2_income));
    let interest = RwSignal::new(money_str(seed.interest_income));
    let ord_div = RwSignal::new(money_str(seed.ordinary_dividends));
    let qual_div = RwSignal::new(money_str(seed.qualified_dividends));
    let st_gains = RwSignal::new(money_str(seed.st_capital_gains));
    let lt_gains = RwSignal::new(money_str(seed.lt_capital_gains));
    let rental = RwSignal::new(money_str(seed.rental_income));
    let itemized = RwSignal::new(money_str(seed.itemized_deductions));
    let cf_st = RwSignal::new(money_str(seed.carryforward_st_loss));
    let cf_lt = RwSignal::new(money_str(seed.carryforward_lt_loss));

    let err = RwSignal::new(Option::<String>::None);
    let saving = RwSignal::new(false);

    let on_save = move |_| {
            // Parse all money fields; empty → 0.0, invalid → error.
            let parse = |sig: RwSignal<String>, name: &str| -> Result<f64, String> {
                let s = sig.get();
                let t = s.trim();
                if t.is_empty() {
                    return Ok(0.0);
                }
                t.parse::<f64>().map_err(|_| format!("{} must be a number", name))
            };

            let rev = (|| -> Result<TaxRevision, String> {
                Ok(TaxRevision {
                    entered_at: Utc::now(),
                    filing_status: filing_status.get(),
                    w2_income: parse(w2, "W-2 income")?,
                    interest_income: parse(interest, "Interest")?,
                    ordinary_dividends: parse(ord_div, "Ordinary dividends")?,
                    qualified_dividends: parse(qual_div, "Qualified dividends")?,
                    st_capital_gains: parse(st_gains, "Short-term gains")?,
                    lt_capital_gains: parse(lt_gains, "Long-term gains")?,
                    rental_income: parse(rental, "Rental income")?,
                    deduction_choice: deduction_choice.get(),
                    itemized_deductions: parse(itemized, "Itemized deductions")?,
                    carryforward_st_loss: parse(cf_st, "ST carryforward loss")?,
                    carryforward_lt_loss: parse(cf_lt, "LT carryforward loss")?,
                })
            })();

            let rev = match rev {
                Ok(r) => r,
                Err(e) => {
                    err.set(Some(e));
                    return;
                }
            };
            err.set(None);

            let id = profile_id.get().unwrap_or_else(Uuid::new_v4);
            let mut all = revisions.get();
            all.push(rev.clone());
            let profile = TaxProfile { id, tax_year: year, revisions: all };

            let token = auth.token.get().unwrap_or_default();
            let user_id = auth.user_id.get().unwrap_or_default();
            saving.set(true);
            spawn_local(async move {
                match supabase::upsert_tax_profile(&token, &user_id, &profile).await {
                    Ok(()) => {
                        profile_id.set(Some(id));
                        revisions.update(|r| r.push(rev));
                        on_saved.run(profile);
                    }
                    Err(e) => err.set(Some(e)),
                }
                saving.set(false);
            });
    };

    view! {
        <div class="rounded-xl border border-border bg-panel">
            <button
                class="w-full flex items-center justify-between px-4 py-3 text-left"
                on:click=move |_| expanded.update(|e| *e = !*e)
            >
                <span class="font-medium text-sm">
                    {format!("Tax year {}", year)}
                    {move || (year > Utc::now().year() as u16).then(|| view! {
                        <span class="ml-2 text-xs text-blue-400">"(projection)"</span>
                    })}
                </span>
                <span class="text-xs text-gray-500">
                    {move || if expanded.get() { "▾" } else { "▸" }}
                </span>
            </button>

            {move || expanded.get().then(|| view! {
                <div class="px-4 pb-4 space-y-4 border-t border-border pt-4">
                    <div class="grid grid-cols-2 sm:grid-cols-3 gap-3">
                        <div>
                            <label class="block text-xs text-gray-400 mb-1">"Filing status"</label>
                            <select
                                class=SELECT_CLS
                                prop:value=move || filing_status.get().as_str()
                                on:change=move |ev| {
                                    if let Some(f) = FilingStatus::from_str(&event_target_value(&ev)) {
                                        filing_status.set(f);
                                    }
                                }
                            >
                                {FilingStatus::all().into_iter().map(|f| view! {
                                    <option value=f.as_str()>{f.label()}</option>
                                }).collect_view()}
                            </select>
                        </div>
                        <MoneyField label="W-2 income" signal=w2 />
                        <MoneyField label="Interest income" signal=interest />
                        <MoneyField label="Ordinary dividends" signal=ord_div />
                        <MoneyField label="Qualified dividends" signal=qual_div />
                        <MoneyField label="Rental income" signal=rental />
                        <MoneyField label="Short-term gains" signal=st_gains />
                        <MoneyField label="Long-term gains" signal=lt_gains />
                        <div>
                            <label class="block text-xs text-gray-400 mb-1">"Deduction"</label>
                            <select
                                class=SELECT_CLS
                                prop:value=move || deduction_choice.get().as_str()
                                on:change=move |ev| {
                                    if let Some(d) = DeductionChoice::from_str(&event_target_value(&ev)) {
                                        deduction_choice.set(d);
                                    }
                                }
                            >
                                <option value="standard">"Standard"</option>
                                <option value="itemized">"Itemized"</option>
                            </select>
                        </div>
                        <MoneyField label="Itemized deductions" signal=itemized />
                        <MoneyField label="ST carryforward loss" signal=cf_st />
                        <MoneyField label="LT carryforward loss" signal=cf_lt />
                    </div>

                    <p class="text-xs text-gray-500">
                        "Qualified dividends are a subset of ordinary dividends (taxed at long-term rates). Carryforward losses are entered as positive numbers."
                    </p>

                    {move || err.get().map(|e| view! { <p class="text-sm text-red-400">{e}</p> })}

                    <button
                        class="text-sm px-4 py-1.5 rounded bg-blue-600 hover:bg-blue-500 transition-colors disabled:opacity-50"
                        prop:disabled=move || saving.get()
                        on:click=on_save
                    >
                        {move || if saving.get() { "Saving…" } else { "Save" }}
                    </button>

                    <RevisionHistory revisions=revisions />
                </div>
            })}
        </div>
    }
}

#[component]
fn RevisionHistory(revisions: RwSignal<Vec<TaxRevision>>) -> impl IntoView {
    view! {
        {move || {
            let revs = revisions.get();
            // Skip the current (last) revision; show prior edits, newest first.
            if revs.len() <= 1 {
                return None;
            }
            let prior: Vec<TaxRevision> = revs.iter().rev().skip(1).cloned().collect();
            Some(view! {
                <div class="pt-2">
                    <h3 class="text-xs font-medium text-gray-400 mb-2">"Edit history"</h3>
                    <div class="space-y-1">
                        {prior.into_iter().map(|r| {
                            let when = r.entered_at.format("%d %b %Y %H:%M").to_string();
                            let summary = format!(
                                "{} · W-2 {} · ST {} · LT {}",
                                r.filing_status.label(),
                                fmt_cash(r.w2_income),
                                fmt_cash(r.st_capital_gains),
                                fmt_cash(r.lt_capital_gains),
                            );
                            view! {
                                <div class="flex items-baseline justify-between text-xs text-gray-500 gap-3">
                                    <span class="whitespace-nowrap">{when}</span>
                                    <span class="text-right">{summary}</span>
                                </div>
                            }
                        }).collect_view()}
                    </div>
                </div>
            })
        }}
    }
}

const SELECT_CLS: &str =
    "w-full bg-surface border border-border rounded px-2 py-1.5 text-sm focus:outline-none focus:border-blue-500";

/// Formats a stored amount for an input field — blank for zero, plain number otherwise.
fn money_str(v: f64) -> String {
    if v == 0.0 {
        String::new()
    } else {
        format!("{}", v)
    }
}

#[component]
fn MoneyField(label: &'static str, signal: RwSignal<String>) -> impl IntoView {
    view! {
        <div>
            <label class="block text-xs text-gray-400 mb-1">{label}</label>
            <input
                class="w-full bg-surface border border-border rounded px-3 py-1.5 text-sm focus:outline-none focus:border-blue-500"
                prop:value=move || signal.get()
                on:input=move |ev| signal.set(event_target_value(&ev))
                placeholder="0"
            />
        </div>
    }
}
