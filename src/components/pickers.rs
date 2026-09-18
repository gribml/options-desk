//! Contract pickers that take a custom value as well as a listed one.
//!
//! The option chain we hold is a snapshot: it lags new listings, thins out at
//! far-dated expiries, and covers only the symbols the pipeline follows. A
//! `<select>` fed from it alone makes anything it lacks impossible to enter.
//! These keep the select — the market's expiries and strikes are the usual
//! answer — and end it with "Custom…", which opens a field for anything else.

use chrono::NaiveDate;
use leptos::prelude::*;

/// Accept the date formats people actually type and return the canonical
/// `YYYY-MM-DD` the rest of the app expects. Anything unrecognised is left
/// as typed so the field never fights the user mid-entry.
pub fn canonical_date(s: &str) -> String {
    let t = s.trim();
    // Two-digit-year forms go first: `%Y` would accept "28" as the year 28,
    // while `%y` rejects a four-digit year as trailing input.
    for fmt in ["%Y-%m-%d", "%m/%d/%y", "%m/%d/%Y", "%d-%b-%y", "%d-%b-%Y", "%d %b %Y", "%b %d %Y", "%Y%m%d"] {
        if let Ok(d) = NaiveDate::parse_from_str(t, fmt) {
            return d.format("%Y-%m-%d").to_string();
        }
    }
    t.to_string()
}

/// Sentinel `<option>` value for "Custom…". Can't collide with a date or a
/// price.
const CUSTOM: &str = "__custom__";

/// A ref for the custom field that puts the cursor in it as soon as it
/// appears, so choosing "Custom…" flows straight into typing. Fires only
/// when the element is (re)mounted, since the ref signal only changes then.
fn focus_when_mounted() -> NodeRef<leptos::html::Input> {
    let r = NodeRef::<leptos::html::Input>::new();
    Effect::new(move |_| {
        if let Some(el) = r.get() {
            let _ = el.focus();
        }
    });
    r
}

/// Expiry: a select of the market's expiries ending in "Custom…", which
/// reveals a field for any date. Normalised to `YYYY-MM-DD` when that field
/// loses focus. A value that isn't in the list — a saved custom one, or one
/// the chain has since dropped — shows as custom, so it is never blanked.
/// `on_set` fires after every change, for callers that reset a dependent
/// field (the strike, usually).
#[component]
pub fn ExpiryPicker(
    value: RwSignal<String>,
    #[prop(into)] options: Signal<Vec<String>>,
    #[prop(optional)] on_set: Option<Callback<()>>,
    #[prop(into)] class: String,
) -> impl IntoView {
    let chose_custom = RwSignal::new(false);
    let is_custom = Signal::derive(move || {
        let v = value.get();
        chose_custom.get() || (!v.is_empty() && !options.get().contains(&v))
    });
    let fire = move || { if let Some(cb) = on_set { cb.run(()); } };
    let input_cls = class.clone();
    let custom_ref = focus_when_mounted();
    view! {
        <span class="inline-flex items-center gap-1">
            <select
                class=class
                prop:value=move || if is_custom.get() { CUSTOM.to_string() } else { value.get() }
                on:change=move |ev| {
                    let v = event_target_value(&ev);
                    if v == CUSTOM {
                        chose_custom.set(true);
                        value.set(String::new());
                    } else {
                        chose_custom.set(false);
                        value.set(v);
                    }
                    fire();
                }
            >
                <option value="">"— expiry —"</option>
                {move || options.get().into_iter().map(|e| {
                    let label = e.clone();
                    view! { <option value=e>{label}</option> }
                }).collect_view()}
                <option value=CUSTOM>"Custom…"</option>
            </select>
            {move || is_custom.get().then(|| view! {
                <input
                    type="text"
                    node_ref=custom_ref
                    class=input_cls.clone()
                    placeholder="YYYY-MM-DD"
                    autocomplete="off"
                    prop:value=move || value.get()
                    on:input=move |ev| { value.set(event_target_value(&ev)); fire(); }
                    on:change=move |ev| {
                        let canon = canonical_date(&event_target_value(&ev));
                        if canon != value.get_untracked() { value.set(canon); fire(); }
                    }
                />
            })}
        </span>
    }
}

/// Strike: a select of the market's strikes ending in "Custom…", which
/// reveals a field for any price.
#[component]
pub fn StrikePicker(
    value: RwSignal<String>,
    #[prop(into)] options: Signal<Vec<f64>>,
    #[prop(optional)] on_set: Option<Callback<()>>,
    #[prop(into)] class: String,
) -> impl IntoView {
    let chose_custom = RwSignal::new(false);
    // Compare as numbers: "150" and "150.0" are the same strike.
    let listed = move |v: &str| -> Option<String> {
        let x: f64 = v.trim().parse().ok()?;
        options.get().into_iter().find(|s| (s - x).abs() < 1e-9).map(|s| format!("{}", s))
    };
    let is_custom = Signal::derive(move || {
        let v = value.get();
        chose_custom.get() || (!v.is_empty() && listed(&v).is_none())
    });
    let fire = move || { if let Some(cb) = on_set { cb.run(()); } };
    let input_cls = class.clone();
    let custom_ref = focus_when_mounted();
    view! {
        <span class="inline-flex items-center gap-1">
            <select
                class=class
                prop:value=move || {
                    if is_custom.get() { CUSTOM.to_string() } else { listed(&value.get()).unwrap_or_default() }
                }
                on:change=move |ev| {
                    let v = event_target_value(&ev);
                    if v == CUSTOM {
                        chose_custom.set(true);
                        value.set(String::new());
                    } else {
                        chose_custom.set(false);
                        value.set(v);
                    }
                    fire();
                }
            >
                <option value="">"— strike —"</option>
                {move || options.get().into_iter().map(|s| {
                    view! { <option value=format!("{}", s)>{format!("${:.0}", s)}</option> }
                }).collect_view()}
                <option value=CUSTOM>"Custom…"</option>
            </select>
            {move || is_custom.get().then(|| view! {
                <input
                    type="text"
                    inputmode="decimal"
                    node_ref=custom_ref
                    class=input_cls.clone()
                    placeholder="0.00"
                    autocomplete="off"
                    prop:value=move || value.get()
                    on:input=move |ev| { value.set(event_target_value(&ev)); fire(); }
                />
            })}
        </span>
    }
}

#[cfg(test)]
mod tests {
    use super::canonical_date;

    #[test]
    fn common_date_spellings_become_iso() {
        for s in ["2028-01-21", "01/21/2028", "1/21/28", "21-Jan-2028", "21-jan-28", "21 Jan 2028", "Jan 21 2028", "20280121"] {
            assert_eq!(canonical_date(s), "2028-01-21", "{s}");
        }
    }

    #[test]
    fn unparseable_input_is_left_alone() {
        assert_eq!(canonical_date("2028-01"), "2028-01");
        assert_eq!(canonical_date("  soon "), "soon");
        assert_eq!(canonical_date(""), "");
    }
}
