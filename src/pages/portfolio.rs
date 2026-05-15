use std::rc::Rc;

use chrono::NaiveDate;
use leptos::prelude::*;
use uuid::Uuid;
use wasm_bindgen_futures::spawn_local;

use crate::api::supabase;
use crate::app::AuthState;
use crate::models::{
    option::{OptionSpec, OptionType},
    position::{Position, PositionKind},
};

#[component]
pub fn PortfolioPage() -> impl IntoView {
    let auth = use_context::<AuthState>().expect("AuthState missing");
    let positions = RwSignal::new(Vec::<Position>::new());
    let loading = RwSignal::new(true);
    let error = RwSignal::new(Option::<String>::None);
    let show_add = RwSignal::new(false);

    let auth_for_load = auth.clone();
    Effect::new(move |_| {
        let token = auth_for_load.token.get();
        let user_id = auth_for_load.user_id.get();
        if let (Some(tok), Some(uid)) = (token, user_id) {
            spawn_local(async move {
                match supabase::fetch_positions(&tok, &uid).await {
                    Ok(ps) => positions.set(ps),
                    Err(e) => error.set(Some(e)),
                }
                loading.set(false);
            });
        } else {
            loading.set(false);
        }
    });

    let delete_position = {
        let auth = auth.clone();
        move |id: Uuid| {
            let token = auth.token.get().unwrap_or_default();
            spawn_local(async move {
                if supabase::delete_position(&token, &id.to_string()).await.is_ok() {
                    positions.update(|ps| ps.retain(|p| p.id != id));
                }
            });
        }
    };

    view! {
        <div class="space-y-6">
            <div class="flex items-center justify-between">
                <h1 class="text-xl font-semibold">"Portfolio"</h1>
                <button
                    class="bg-blue-600 hover:bg-blue-500 px-4 py-2 rounded text-sm font-medium transition-colors"
                    on:click=move |_| show_add.update(|v| *v = !*v)
                >
                    {move || if show_add.get() { "Cancel" } else { "+ Add position" }}
                </button>
            </div>

            {move || show_add.get().then(|| {
                let auth2 = auth.clone();
                view! {
                    <AddPositionForm
                        auth=auth2
                        on_added=move |p: Position| {
                            positions.update(|ps| ps.push(p));
                            show_add.set(false);
                        }
                    />
                }
            })}

            {move || error.get().map(|e| view! {
                <p class="text-red-400 text-sm">{e}</p>
            })}

            {move || loading.get().then(|| view! {
                <p class="text-gray-400 text-sm">"Loading…"</p>
            })}

            {move || {
                let ps = positions.get();
                (!loading.get() && ps.is_empty()).then(|| view! {
                    <p class="text-gray-500 text-sm">"No positions yet. Add one above."</p>
                })
            }}

            <div class="space-y-2">
                {move || positions.get().into_iter().map(|p| {
                    let id = p.id;
                    let del = delete_position.clone();
                    view! {
                        <PositionRow
                            position=p
                            on_delete=move || del(id)
                        />
                    }
                }).collect_view()}
            </div>
        </div>
    }
}

#[component]
fn PositionRow(position: Position, on_delete: impl Fn() + 'static) -> impl IntoView {
    let kind_label = match position.kind {
        PositionKind::Stock => "Stock".to_string(),
        PositionKind::Option => {
            if let Some(spec) = &position.option_spec {
                format!(
                    "{} ${:.0} {}",
                    spec.option_type.label(),
                    spec.strike,
                    spec.expiry.format("%d-%b-%y")
                )
            } else {
                "Option".to_string()
            }
        }
    };

    let qty_class = if position.quantity >= 0 { "text-green-400" } else { "text-red-400" };

    view! {
        <div class="bg-panel border border-border rounded-lg px-4 py-3 flex items-center justify-between">
            <div class="flex gap-6 items-center">
                <span class="font-semibold text-sm w-16">{position.symbol.clone()}</span>
                <span class="text-xs text-gray-400 w-40">{kind_label}</span>
                <span class=format!("text-sm font-mono {}", qty_class)>
                    {format!("{:+}", position.quantity)}
                </span>
                <span class="text-xs text-gray-400">
                    "@ " {format!("{:.2}", position.cost_basis)}
                </span>
                <span class="text-xs text-gray-500">
                    "cost " {format!("${:.2}", position.total_cost().abs())}
                </span>
            </div>
            <button
                class="text-gray-500 hover:text-red-400 text-xs transition-colors"
                on:click=move |_| on_delete()
            >
                "remove"
            </button>
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
    let symbol = RwSignal::new(String::new());
    let kind = RwSignal::new(PositionKind::Stock);
    let quantity = RwSignal::new("1".to_string());
    let cost_basis = RwSignal::new(String::new());
    let opt_type = RwSignal::new(OptionType::Call);
    let strike = RwSignal::new(String::new());
    let expiry = RwSignal::new(String::new());
    let err = RwSignal::new(Option::<String>::None);
    let saving = RwSignal::new(false);

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
                let spec = OptionSpec {
                    symbol: sym.clone(),
                    option_type: opt_type.get(),
                    strike: s,
                    expiry: exp,
                };
                Position::new_option(&sym, qty, cb, spec)
            }
        };

        saving.set(true);
        let token = auth.token.get().unwrap_or_default();
        let user_id = auth.user_id.get().unwrap_or_default();
        let pos_clone = position.clone();
        let cb_fn = Rc::clone(&on_added);
        spawn_local(async move {
            match supabase::upsert_position(&token, &user_id, &pos_clone).await {
                Ok(_) => cb_fn(pos_clone),
                Err(e) => { err.set(Some(e)); saving.set(false); }
            }
        });
    };

    view! {
        <form on:submit=on_submit class="bg-panel border border-border rounded-xl p-6 space-y-4">
            <h2 class="text-sm font-medium text-gray-300">"Add position"</h2>

            <div class="flex gap-2">
                {[
                    (PositionKind::Stock, "Stock"),
                    (PositionKind::Option, "Option"),
                ].map(|(k, label)| {
                    let k_cmp = k.clone();
                    let k_set = k.clone();
                    view! {
                        <button
                            type="button"
                            class=move || {
                                let active = kind.get() == k_cmp;
                                format!(
                                    "px-4 py-1 rounded text-xs border transition-colors {}",
                                    if active { "bg-blue-600 border-blue-600 text-white" }
                                    else { "bg-surface border-border text-gray-400 hover:border-gray-500" }
                                )
                            }
                            on:click={
                                let k = k_set.clone();
                                move |_| kind.set(k.clone())
                            }
                        >
                            {label}
                        </button>
                    }
                })}
            </div>

            <div class="grid grid-cols-2 gap-3">
                <SmallInput label="Symbol" signal=symbol placeholder="AAPL" />
                <SmallInput label="Quantity (neg=short)" signal=quantity placeholder="1" />
                <SmallInput label="Cost basis / share" signal=cost_basis placeholder="0.00" />

                {move || (kind.get() == PositionKind::Option).then(|| view! {
                    <>
                        <div class="col-span-2 flex gap-2">
                            {[OptionType::Call, OptionType::Put].map(|t| {
                                let label = t.label();
                                view! {
                                    <button
                                        type="button"
                                        class=move || {
                                            let active = opt_type.get() == t;
                                            format!(
                                                "px-4 py-1 rounded text-xs border transition-colors {}",
                                                if active { "bg-blue-600 border-blue-600 text-white" }
                                                else { "bg-surface border-border text-gray-400" }
                                            )
                                        }
                                        on:click=move |_| opt_type.set(t)
                                    >
                                        {label}
                                    </button>
                                }
                            })}
                        </div>
                        <SmallInput label="Strike" signal=strike placeholder="150.00" />
                        <SmallInput label="Expiry (YYYY-MM-DD)" signal=expiry placeholder="2025-01-17" />
                    </>
                })}
            </div>

            {move || err.get().map(|e| view! {
                <p class="text-red-400 text-xs">{e}</p>
            })}

            <button
                type="submit"
                class="bg-blue-600 hover:bg-blue-500 disabled:opacity-50 px-4 py-2 rounded text-sm font-medium transition-colors"
                prop:disabled=move || saving.get()
            >
                {move || if saving.get() { "Saving…" } else { "Add" }}
            </button>
        </form>
    }
}

#[component]
fn SmallInput(label: &'static str, signal: RwSignal<String>, placeholder: &'static str) -> impl IntoView {
    view! {
        <div>
            <label class="block text-xs text-gray-400 mb-1">{label}</label>
            <input
                class="w-full bg-surface border border-border rounded px-3 py-1.5 text-sm focus:outline-none focus:border-blue-500"
                prop:value=move || signal.get()
                on:input=move |ev| signal.set(event_target_value(&ev))
                placeholder=placeholder
            />
        </div>
    }
}
