//! Contract pickers that take a typed value as readily as a listed one.
//!
//! The option chain we hold is a snapshot: it lags new listings, thins out at
//! far-dated expiries, and covers only the symbols the pipeline follows. A
//! `<select>` fed from it makes anything it lacks impossible to enter. These
//! are text inputs backed by a `<datalist>` instead — the market's expiries
//! and strikes are one keystroke away, and anything else can simply be typed.

use chrono::NaiveDate;
use leptos::prelude::*;
use uuid::Uuid;

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

/// Expiry: type a date or pick one the market lists. Normalised to
/// `YYYY-MM-DD` when the field loses focus. `on_set` fires after every
/// change, for callers that reset a dependent field (the strike, usually).
#[component]
pub fn ExpiryInput(
    value: RwSignal<String>,
    #[prop(into)] options: Signal<Vec<String>>,
    #[prop(optional)] on_set: Option<Callback<()>>,
    #[prop(into)] class: String,
) -> impl IntoView {
    let list_id = format!("exp-{}", Uuid::new_v4().simple());
    let list_for = list_id.clone();
    let fire = move || { if let Some(cb) = on_set { cb.run(()); } };
    view! {
        <input
            type="text"
            class=class
            list=list_for
            placeholder="YYYY-MM-DD"
            autocomplete="off"
            title="Pick a listed expiry or type any date"
            prop:value=move || value.get()
            on:input=move |ev| { value.set(event_target_value(&ev)); fire(); }
            on:change=move |ev| {
                let canon = canonical_date(&event_target_value(&ev));
                if canon != value.get_untracked() { value.set(canon); fire(); }
            }
        />
        <datalist id=list_id>
            {move || options.get().into_iter().map(|e| view! { <option value=e /> }).collect_view()}
        </datalist>
    }
}

/// Strike: type a price or pick one the market lists.
#[component]
pub fn StrikeInput(
    value: RwSignal<String>,
    #[prop(into)] options: Signal<Vec<f64>>,
    #[prop(optional)] on_set: Option<Callback<()>>,
    #[prop(into)] class: String,
) -> impl IntoView {
    let list_id = format!("strike-{}", Uuid::new_v4().simple());
    let list_for = list_id.clone();
    view! {
        <input
            type="text"
            inputmode="decimal"
            class=class
            list=list_for
            placeholder="strike"
            autocomplete="off"
            title="Pick a listed strike or type any price"
            prop:value=move || value.get()
            on:input=move |ev| {
                value.set(event_target_value(&ev));
                if let Some(cb) = on_set { cb.run(()); }
            }
        />
        <datalist id=list_id>
            {move || options.get().into_iter().map(|s| view! { <option value=format!("{}", s) /> }).collect_view()}
        </datalist>
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
