//! CSV → lots. Pure parsing and coercion; no Leptos, no web APIs.
//!
//! Broker exports are messy in predictable ways: quoted fields containing
//! commas, thousands separators inside numbers, currency symbols, parenthesised
//! negatives, and half a dozen date orderings. Everything here is written to be
//! forgiving on input and explicit about what it could not read, so the review
//! step can show the user exactly which cells need a human.

use chrono::NaiveDate;

// ── CSV splitting ────────────────────────────────────────────────────────────

/// Splits CSV text into rows of fields, honouring RFC-4180 quoting: fields may
/// be wrapped in `"`, a quoted field may contain commas and newlines, and `""`
/// inside a quoted field is a literal quote. Handles LF and CRLF line endings.
///
/// Rows that are entirely empty are dropped, so trailing newlines and the blank
/// lines brokers like to pad exports with don't become phantom lots.
pub fn split_csv(text: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut in_quotes = false;
    let mut chars = text.chars().peekable();

    while let Some(c) = chars.next() {
        if in_quotes {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    field.push('"');
                } else {
                    in_quotes = false;
                }
            } else {
                field.push(c);
            }
            continue;
        }
        match c {
            '"' => in_quotes = true,
            ',' => row.push(std::mem::take(&mut field)),
            '\r' => {} // swallow; the \n that follows ends the row
            '\n' => {
                row.push(std::mem::take(&mut field));
                rows.push(std::mem::take(&mut row));
            }
            _ => field.push(c),
        }
    }
    // Final field/row when the file doesn't end in a newline.
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }

    rows.retain(|r| r.iter().any(|f| !f.trim().is_empty()));
    rows
}

// ── Field coercion ───────────────────────────────────────────────────────────

/// Reads a money-ish string: `$1,234.56`, `1 234,00` style separators are *not*
/// assumed, but `$`, `,`, whitespace and a trailing `USD` are stripped, and both
/// `-123` and `(123)` are read as negative. Returns `None` if what's left isn't
/// a number.
pub fn parse_money(s: &str) -> Option<f64> {
    let t = s.trim();
    if t.is_empty() {
        return None;
    }
    let parenthesised = t.starts_with('(') && t.ends_with(')');
    let cleaned: String = t
        .trim_start_matches('(')
        .trim_end_matches(')')
        .chars()
        .filter(|c| !matches!(c, '$' | ',' | ' ' | '\u{a0}'))
        .collect();
    let cleaned = cleaned.trim_end_matches("USD").trim_end_matches("usd");
    let v: f64 = cleaned.parse().ok()?;
    if !v.is_finite() {
        return None;
    }
    Some(if parenthesised { -v.abs() } else { v })
}

/// Share counts, as a float so fractional shares survive long enough to be
/// reported. `Trade::quantity` is an integer, so the caller has to decide what
/// to do about a fraction — see [`DraftLot::issues`].
pub fn parse_quantity(s: &str) -> Option<f64> {
    parse_money(s)
}

/// Date orderings seen in broker exports, most-specific first. Ambiguous
/// `d/m/Y` is deliberately absent: it cannot be told apart from `m/d/Y` for the
/// first twelve days of a month, and guessing silently would put lots on the
/// wrong side of the one-year line. US ordering is assumed and the review step
/// shows the interpreted date so a mistake is visible.
const DATE_FORMATS: &[&str] = &[
    "%Y-%m-%d",
    "%Y/%m/%d",
    "%m/%d/%Y",
    "%m-%d-%Y",
    "%m/%d/%y",
    "%m-%d-%y",
    "%d-%b-%Y",
    "%d-%b-%y",
    "%d %b %Y",
    "%b %d, %Y",
    "%b %d %Y",
    "%B %d, %Y",
];

pub fn parse_date(s: &str) -> Option<NaiveDate> {
    use chrono::Datelike as _;
    let t = s.trim();
    if t.is_empty() {
        return None;
    }
    // Some exports carry a time component; the date alone decides holding period.
    let head = t.split_whitespace().next().unwrap_or(t);

    // A result is only accepted if the year is plausible. Without this, `%m/%d/%Y`
    // matches "3/15/21" as year 21 — two millennia early, and silently long-term —
    // before the two-digit `%m/%d/%y` form is ever reached.
    for f in DATE_FORMATS {
        for candidate in [t, head] {
            if let Ok(d) = NaiveDate::parse_from_str(candidate, f) {
                if (1900..=2100).contains(&d.year()) {
                    return Some(d);
                }
            }
        }
    }
    None
}

/// Whether the mapped cost column holds one share's price or the whole lot's cost.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BasisKind {
    PerShare,
    TotalCost,
}

impl BasisKind {
    /// Per-share basis, given the raw column value and the lot's share count.
    pub fn per_share(self, value: f64, quantity: f64) -> Option<f64> {
        match self {
            BasisKind::PerShare => Some(value),
            BasisKind::TotalCost => {
                if quantity.abs() < 1e-9 {
                    None
                } else {
                    Some(value / quantity)
                }
            }
        }
    }
}

// ── Draft lots ───────────────────────────────────────────────────────────────

/// One editable row in the review step. Fields stay as strings so the user can
/// fix a cell the parser choked on without the value being destroyed first —
/// the same reason every other form in this app holds strings.
#[derive(Debug, Clone, PartialEq)]
pub struct DraftLot {
    pub symbol: String,
    pub date: String,
    pub quantity: String,
    /// Always per-share by the time it lands here; the mapping step divides a
    /// total-cost column through by quantity.
    pub price: String,
}

/// What's wrong with a row, in words fit to show the user.
#[derive(Debug, Clone, PartialEq)]
pub struct LotIssues {
    pub symbol: Option<&'static str>,
    pub date: Option<&'static str>,
    pub quantity: Option<&'static str>,
    pub price: Option<&'static str>,
    /// Non-blocking: the row imports, but something was changed to make it fit.
    pub warning: Option<String>,
}

impl LotIssues {
    pub fn blocking(&self) -> bool {
        self.symbol.is_some()
            || self.date.is_some()
            || self.quantity.is_some()
            || self.price.is_some()
    }
}

impl DraftLot {
    pub fn blank() -> Self {
        Self {
            symbol: String::new(),
            date: String::new(),
            quantity: String::new(),
            price: String::new(),
        }
    }

    /// Validated view of the row, plus anything the user should know about it.
    pub fn issues(&self) -> LotIssues {
        let qty = parse_quantity(&self.quantity);
        let mut warning = None;

        // Share counts must land on an integer to fit `Trade::quantity`. Rounding
        // is nearly always right (brokers print 100.0000) but must never be silent.
        if let Some(q) = qty {
            if (q - q.round()).abs() > 1e-6 {
                warning = Some(format!(
                    "{} shares rounded to {}",
                    trim_float(q),
                    q.round() as i64
                ));
            }
        }

        LotIssues {
            symbol: if self.symbol.trim().is_empty() {
                Some("needs a ticker")
            } else {
                None
            },
            date: match parse_date(&self.date) {
                Some(_) => None,
                None => Some("unreadable date"),
            },
            quantity: match qty {
                Some(q) if q.round() as i64 != 0 => None,
                Some(_) => Some("can't be zero"),
                None => Some("not a number"),
            },
            price: match parse_money(&self.price) {
                Some(p) if p >= 0.0 => None,
                Some(_) => Some("can't be negative"),
                None => Some("not a number"),
            },
            warning,
        }
    }

    /// `(symbol, date, quantity, per-share price)` when the row is complete.
    pub fn resolved(&self) -> Option<(String, NaiveDate, i32, f64)> {
        if self.issues().blocking() {
            return None;
        }
        Some((
            self.symbol.trim().to_uppercase(),
            parse_date(&self.date)?,
            parse_quantity(&self.quantity)?.round() as i32,
            parse_money(&self.price)?,
        ))
    }
}

fn trim_float(v: f64) -> String {
    format!("{v}")
}

/// Builds draft rows from split CSV data under a chosen column mapping.
/// `symbol_col` of `None` means every row takes `fallback_symbol`.
#[allow(clippy::too_many_arguments)]
pub fn draft_lots(
    rows: &[Vec<String>],
    skip_header: bool,
    date_col: usize,
    qty_col: usize,
    price_col: usize,
    symbol_col: Option<usize>,
    fallback_symbol: &str,
    basis: BasisKind,
) -> Vec<DraftLot> {
    let body = if skip_header && !rows.is_empty() {
        &rows[1..]
    } else {
        rows
    };
    let cell = |r: &Vec<String>, i: usize| r.get(i).map(|s| s.trim().to_string()).unwrap_or_default();

    body.iter()
        .map(|r| {
            let qty_raw = cell(r, qty_col);
            let price_raw = cell(r, price_col);

            // A total-cost column has to be divided through here, while the
            // quantity for this row is still in hand.
            let price = match (basis, parse_money(&price_raw), parse_quantity(&qty_raw)) {
                (BasisKind::PerShare, _, _) => price_raw,
                (BasisKind::TotalCost, Some(v), Some(q)) => match basis.per_share(v, q) {
                    Some(p) => format!("{p:.4}"),
                    None => price_raw,
                },
                (BasisKind::TotalCost, _, _) => price_raw,
            };

            DraftLot {
                symbol: match symbol_col {
                    Some(i) => cell(r, i).to_uppercase(),
                    None => fallback_symbol.trim().to_uppercase(),
                },
                date: cell(r, date_col),
                quantity: qty_raw,
                price,
            }
        })
        .collect()
}

/// Best-guess column index for a role, from the header text. Saves the user the
/// mapping step entirely on well-formed exports; they can always override.
pub fn guess_column(headers: &[String], role: Role) -> Option<usize> {
    let needles: &[&str] = match role {
        Role::Date => &["date acquired", "acquired", "open date", "trade date", "purchase date", "date"],
        Role::Quantity => &["quantity", "shares", "qty", "amount", "units"],
        Role::Price => &["cost per share", "price per share", "unit cost", "cost basis", "basis", "price", "cost"],
        Role::Symbol => &["symbol", "ticker", "security"],
    };
    let lower: Vec<String> = headers.iter().map(|h| h.trim().to_lowercase()).collect();
    // Exact-ish match first, then substring, so "cost basis" doesn't lose to "cost".
    for n in needles {
        if let Some(i) = lower.iter().position(|h| h == n) {
            return Some(i);
        }
    }
    for n in needles {
        if let Some(i) = lower.iter().position(|h| h.contains(n)) {
            return Some(i);
        }
    }
    None
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Date,
    Quantity,
    Price,
    Symbol,
}
