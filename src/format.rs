use leptos::prelude::*;

/// Formats a currency value, abbreviating large numbers for compact display.
pub fn fmt_currency(v: f64) -> String {
    fmt_signed(v, false)
}

/// Like `fmt_currency` but prefixes positive values with `+`.
pub fn fmt_cash(v: f64) -> String {
    fmt_signed(v, true)
}

/// Full-precision currency — never abbreviated.
pub fn fmt_currency_full(v: f64) -> String {
    if v < 0.0 { format!("-${:.2}", v.abs()) } else { format!("${:.2}", v) }
}

/// Full-precision cash with +/- prefix — never abbreviated.
pub fn fmt_cash_full(v: f64) -> String {
    format!("{}{:.2}", if v >= 0.0 { "+$" } else { "-$" }, v.abs())
}

fn fmt_signed(v: f64, show_plus: bool) -> String {
    let abs = v.abs();
    let (scaled, suffix) = if abs >= 1_000_000_000.0 {
        (v / 1_000_000_000.0, "B")
    } else if abs >= 1_000_000.0 {
        (v / 1_000_000.0, "M")
    } else if abs >= 10_000.0 {
        (v / 1_000.0, "K")
    } else {
        let prefix = if show_plus && v >= 0.0 { "+$" } else if v < 0.0 { "-$" } else { "$" };
        return format!("{}{:.2}", prefix, abs);
    };
    let prefix = if show_plus && scaled >= 0.0 { "+$" } else if scaled < 0.0 { "-$" } else { "$" };
    format!("{}{:.1}{}", prefix, scaled.abs(), suffix)
}

/// Renders a currency value: full precision on desktop (≥640px), abbreviated on mobile.
#[component]
pub fn Num(value: f64, #[prop(default = false)] signed: bool) -> impl IntoView {
    let full = if signed { fmt_cash_full(value) } else { fmt_currency_full(value) };
    let abbr = if signed { fmt_cash(value) } else { fmt_currency(value) };
    view! {
        <span class="hidden sm:inline">{full}</span>
        <span class="sm:hidden">{abbr}</span>
    }
}
