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

    let initial_seed = existing.as_ref().and_then(|p| p.current().cloned()).unwrap_or_default();

    let filing_status = RwSignal::new(initial_seed.filing_status);
    let deduction_choice = RwSignal::new(initial_seed.deduction_choice);
    let w2 = RwSignal::new(money_str(initial_seed.w2_income));
    let interest = RwSignal::new(money_str(initial_seed.interest_income));
    let non_qual_div = RwSignal::new(money_str((initial_seed.ordinary_dividends - initial_seed.qualified_dividends).max(0.0)));
    let qual_div = RwSignal::new(money_str(initial_seed.qualified_dividends));
    let st_gains = RwSignal::new(money_str(initial_seed.st_capital_gains));
    let lt_gains = RwSignal::new(money_str(initial_seed.lt_capital_gains));
    let rental = RwSignal::new(money_str(initial_seed.rental_income));
    let itemized = RwSignal::new(money_str(initial_seed.itemized_deductions));
    let cf_st = RwSignal::new(money_str(initial_seed.carryforward_st_loss));
    let cf_lt = RwSignal::new(money_str(initial_seed.carryforward_lt_loss));

    let err = RwSignal::new(Option::<String>::None);
    let saving = RwSignal::new(false);

    // Derives the latest persisted revision from `revisions` so is_dirty always
    // compares against what's actually saved, not the snapshot from component creation.
    let seed = Memo::new(move |_| revisions.get().last().cloned().unwrap_or_default());

    // True when any form field differs from the last saved revision.
    let is_dirty = Memo::new(move |_| {
        let s = seed.get();
        let parse_f = |s: &str| -> f64 { s.trim().parse().unwrap_or(0.0) };
        filing_status.get() != s.filing_status
            || deduction_choice.get() != s.deduction_choice
            || (parse_f(&w2.get())       - s.w2_income).abs()            > 0.005
            || (parse_f(&interest.get()) - s.interest_income).abs()      > 0.005
            || (parse_f(&non_qual_div.get()) - (s.ordinary_dividends - s.qualified_dividends).max(0.0)).abs() > 0.005
            || (parse_f(&qual_div.get())     - s.qualified_dividends).abs() > 0.005
            || (parse_f(&st_gains.get()) - s.st_capital_gains).abs()     > 0.005
            || (parse_f(&lt_gains.get()) - s.lt_capital_gains).abs()     > 0.005
            || (parse_f(&rental.get())   - s.rental_income).abs()        > 0.005
            || (parse_f(&itemized.get()) - s.itemized_deductions).abs()  > 0.005
            || (parse_f(&cf_st.get())    - s.carryforward_st_loss).abs() > 0.005
            || (parse_f(&cf_lt.get())    - s.carryforward_lt_loss).abs() > 0.005
    });

    let apply_revision_to_form = move |cur: TaxRevision| {
        filing_status.set(cur.filing_status);
        deduction_choice.set(cur.deduction_choice);
        w2.set(money_str(cur.w2_income));
        interest.set(money_str(cur.interest_income));
        non_qual_div.set(money_str((cur.ordinary_dividends - cur.qualified_dividends).max(0.0)));
        qual_div.set(money_str(cur.qualified_dividends));
        st_gains.set(money_str(cur.st_capital_gains));
        lt_gains.set(money_str(cur.lt_capital_gains));
        rental.set(money_str(cur.rental_income));
        itemized.set(money_str(cur.itemized_deductions));
        cf_st.set(money_str(cur.carryforward_st_loss));
        cf_lt.set(money_str(cur.carryforward_lt_loss));
    };

    let on_delete_revision = move |entered_at: chrono::DateTime<Utc>| {
        let token = auth.token.get().unwrap_or_default();
        let user_id = auth.user_id.get().unwrap_or_default();
        let id = profile_id.get().unwrap_or_else(Uuid::new_v4);
        revisions.update(|rs| rs.retain(|r| r.entered_at != entered_at));
        let cur = revisions.get_untracked().last().cloned().unwrap_or_default();
        apply_revision_to_form(cur);
        let profile = TaxProfile { id, tax_year: year, revisions: revisions.get_untracked() };
        spawn_local(async move {
            let _ = supabase::upsert_tax_profile(&token, &user_id, &profile).await;
        });
    };

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
                let qual = parse(qual_div, "Qual dividends")?;
                let non_qual = parse(non_qual_div, "Non-qual dividends")?;
                Ok(TaxRevision {
                    entered_at: Utc::now(),
                    filing_status: filing_status.get(),
                    w2_income: parse(w2, "W-2 income")?,
                    interest_income: parse(interest, "Interest")?,
                    ordinary_dividends: non_qual + qual,
                    qualified_dividends: qual,
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
                        <MoneyField label="Non-qual dividends" signal=non_qual_div />
                        <MoneyField label="Qual dividends" signal=qual_div />
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
                        "Non-qual dividends are taxed at ordinary rates; qual dividends at long-term rates. Enter each independently. Carryforward losses are entered as positive numbers."
                    </p>

                    {move || err.get().map(|e| view! { <p class="text-sm text-red-400">{e}</p> })}

                    <button
                        class="text-sm px-4 py-1.5 rounded bg-blue-600 hover:bg-blue-500 transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
                        prop:disabled=move || saving.get() || !is_dirty.get()
                        on:click=on_save
                    >
                        {move || if saving.get() { "Saving…" } else { "Save" }}
                    </button>

                    <RevisionHistory
                        revisions=revisions
                        on_restore=move |r: TaxRevision| apply_revision_to_form(r)
                        on_delete=on_delete_revision
                    />
                </div>
            })}
        </div>
    }
}

#[component]
fn RevisionHistory(
    revisions: RwSignal<Vec<TaxRevision>>,
    #[prop(into)] on_restore: Callback<TaxRevision>,
    #[prop(into)] on_delete: Callback<chrono::DateTime<Utc>>,
) -> impl IntoView {
    let show_history = RwSignal::new(false);

    view! {
        {move || {
            let revs = revisions.get();
            // Need at least 2 revisions to have any history to show.
            if revs.len() <= 1 {
                return None;
            }
            // Pairs: (newer, older) — diff newer against older to show what changed.
            // We also keep the older revision so the user can restore to it.
            let pairs: Vec<(TaxRevision, TaxRevision)> = revs
                .windows(2)
                .map(|w| (w[1].clone(), w[0].clone()))
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();

            Some(view! {
                <div class="pt-2">
                    <button
                        class="text-xs text-gray-500 hover:text-gray-300 transition-colors"
                        on:click=move |_| show_history.update(|v| *v = !*v)
                    >
                        {move || if show_history.get() { "▾ Hide history" } else { "▸ Edit history" }}
                    </button>
                    {move || show_history.get().then(|| view! {
                        <div class="mt-2 space-y-2">
                            {pairs.clone().into_iter().map(|(newer, older)| {
                                let when = newer.entered_at.format("%d %b %Y %H:%M").to_string();
                                let changes = revision_diff(&older, &newer);
                                let restore_to = older.clone();
                                let delete_at = newer.entered_at;
                                view! {
                                    <div class="text-xs text-gray-500 border-l border-border pl-2">
                                        <div class="flex items-baseline justify-between gap-2">
                                            <span class="text-gray-400">{when}</span>
                                            <div class="flex gap-2 shrink-0">
                                                <button
                                                    class="text-gray-600 hover:text-blue-400 transition-colors"
                                                    title="Restore these values to the form"
                                                    on:click=move |_| on_restore.run(restore_to.clone())
                                                >"↩ restore"</button>
                                                <button
                                                    class="text-gray-600 hover:text-red-400 transition-colors"
                                                    title="Delete this revision"
                                                    on:click=move |_| on_delete.run(delete_at)
                                                >"✕"</button>
                                            </div>
                                        </div>
                                        {if changes.is_empty() {
                                            view! { <span class="italic">"no changes"</span> }.into_any()
                                        } else {
                                            view! {
                                                <ul class="mt-0.5 space-y-0">
                                                    {changes.into_iter().map(|c| view! {
                                                        <li>{c}</li>
                                                    }).collect_view()}
                                                </ul>
                                            }.into_any()
                                        }}
                                    </div>
                                }
                            }).collect_view()}
                        </div>
                    })}
                </div>
            })
        }}
    }
}

/// Returns a list of human-readable change descriptions between two revisions.
fn revision_diff(older: &TaxRevision, newer: &TaxRevision) -> Vec<String> {
    let mut changes = Vec::new();

    if older.filing_status != newer.filing_status {
        changes.push(format!("Filing: {} → {}", older.filing_status.label(), newer.filing_status.label()));
    }
    if older.deduction_choice != newer.deduction_choice {
        changes.push(format!("Deduction: {} → {}", older.deduction_choice.as_str(), newer.deduction_choice.as_str()));
    }

    let money_fields: &[(&str, f64, f64)] = &[
        ("W-2",              older.w2_income,            newer.w2_income),
        ("Interest",         older.interest_income,      newer.interest_income),
        ("Non-qual div.",    older.ordinary_dividends - older.qualified_dividends, newer.ordinary_dividends - newer.qualified_dividends),
        ("Qual div.",        older.qualified_dividends,  newer.qualified_dividends),
        ("ST gains",         older.st_capital_gains,     newer.st_capital_gains),
        ("LT gains",         older.lt_capital_gains,     newer.lt_capital_gains),
        ("Rental",           older.rental_income,        newer.rental_income),
        ("Itemized ded.",    older.itemized_deductions,  newer.itemized_deductions),
        ("CF ST loss",       older.carryforward_st_loss, newer.carryforward_st_loss),
        ("CF LT loss",       older.carryforward_lt_loss, newer.carryforward_lt_loss),
    ];
    for (label, old_v, new_v) in money_fields {
        if (old_v - new_v).abs() > 0.005 {
            changes.push(format!("{}: {} → {}", label, fmt_cash(*old_v), fmt_cash(*new_v)));
        }
    }

    changes
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
