/// Formats a currency value, abbreviating large numbers for compact display.
///
/// Examples:
///   999.50      →  "$999.50"
///   12_345.67   →  "$12.3K"
///   1_234_567.0 →  "$1.2M"
///   1_500_000_000.0 → "$1.5B"
pub fn fmt_currency(v: f64) -> String {
    fmt_currency_signed(v, false)
}

/// Like `fmt_currency` but prefixes positive values with `+`.
pub fn fmt_cash(v: f64) -> String {
    fmt_currency_signed(v, true)
}

fn fmt_currency_signed(v: f64, show_plus: bool) -> String {
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
