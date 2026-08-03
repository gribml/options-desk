//! Plain-English definitions for every piece of jargon the UI shows.
//!
//! The audience is someone comfortable with money but not with derivatives, so
//! each entry answers "what is this and why do I care?" in one or two sentences
//! and avoids defining jargon with more jargon. Surfaced by the `Info` component
//! (`components::ui`) via the term key.

pub struct Definition {
    pub key: &'static str,
    pub title: &'static str,
    pub body: &'static str,
}

pub fn lookup(key: &str) -> Option<&'static Definition> {
    TERMS.iter().find(|d| d.key == key)
}

pub const TERMS: &[Definition] = &[
    // ── Options, the basics ──────────────────────────────────────────────────
    Definition {
        key: "call",
        title: "Call option",
        body: "A contract giving its owner the right to buy 100 shares at a fixed price before a set date. If you sell one, you collect cash now and take on the obligation to deliver those shares if the buyer wants them.",
    },
    Definition {
        key: "put",
        title: "Put option",
        body: "A contract giving its owner the right to sell 100 shares at a fixed price before a set date. If you sell one, you collect cash now and take on the obligation to buy those shares if the owner wants out.",
    },
    Definition {
        key: "strike",
        title: "Strike price",
        body: "The fixed price written into the contract. A call is worth exercising once the stock trades above it; a put, once the stock trades below it.",
    },
    Definition {
        key: "spot",
        title: "Spot price",
        body: "What the stock costs right now. Everything else on this page is calculated relative to it.",
    },
    Definition {
        key: "expiry",
        title: "Expiry",
        body: "The date the contract dies. After it, the option is either settled or worth nothing — there is no third outcome.",
    },
    Definition {
        key: "premium",
        title: "Premium",
        body: "The price of the option itself, quoted per share. One contract covers 100 shares, so a $3.50 premium means $350 of cash changes hands.",
    },
    Definition {
        key: "contract",
        title: "Contract",
        body: "One option contract controls 100 shares. Selling 4 contracts against 400 shares covers your whole holding.",
    },
    Definition {
        key: "covered-call",
        title: "Covered call",
        body: "Selling a call against stock you already own. You keep the premium as income; in exchange you give up any gain above the strike price if the stock rises past it.",
    },
    Definition {
        key: "assignment",
        title: "Assignment",
        body: "When the option you sold gets exercised against you. For a covered call it means your shares are sold at the strike price, whether or not you wanted to sell — which is a taxable event.",
    },
    Definition {
        key: "roll",
        title: "Rolling",
        body: "Closing an option you sold and opening another one further out in time, usually to keep collecting premium without letting your shares get called away this month.",
    },

    // ── Prices and value ─────────────────────────────────────────────────────
    Definition {
        key: "cost-basis",
        title: "Cost basis",
        body: "What you originally paid per share, including any adjustments. Your taxable gain is measured from this number, not from what the position is worth today.",
    },
    Definition {
        key: "mark-price",
        title: "Mark price",
        body: "The current fair value of one share or one contract. For stock this is the live quote; for options it is a calculated value, since options often trade thinly.",
    },
    Definition {
        key: "market-value",
        title: "Market value",
        body: "What everything is worth right now — each holding's current price times how much of it you have. An option you've sold counts against you here: it's an obligation you'd have to buy back, not an asset, so it subtracts. A position that's mostly sold options can therefore show a value below its gain, or even a negative one, while still being well ahead.",
    },
    Definition {
        key: "unrealised-pnl",
        title: "Unrealised gain or loss",
        body: "What everything is worth today, minus what it cost you to get there — the gap between this and the portfolio value is exactly that cost. Things you bought cost positive money, so their gain comes out below their value. But an option you sold paid you to open it, so its cost is negative: that's why a gain can be larger than the portfolio value, and it isn't an error when it is. Paper only either way — nothing is owed until you close out.",
    },
    Definition {
        key: "realized-pnl",
        title: "Realised gain or loss",
        body: "Profit or loss on shares you've already sold. Unlike unrealised gains, this one is real and shows up on your tax return for the year it happened.",
    },
    Definition {
        key: "net-cash",
        title: "Net cash",
        body: "The cash that actually lands in or leaves your account if this plays out — premium collected, shares sold, contracts bought back, all netted together.",
    },
    Definition {
        key: "open-lot",
        title: "Lot",
        body: "One batch of shares bought on one date at one price. Lots matter because each one has its own holding-period clock, and selling the older ones can be taxed at a lower rate.",
    },

    // ── Risk measures ────────────────────────────────────────────────────────
    Definition {
        key: "delta",
        title: "Delta — price sensitivity",
        body: "Roughly how many dollars the position gains if the stock rises by $1. Owning 200 shares gives a delta of 200. A delta near zero means the stock's direction barely affects you.",
    },
    // Retained but currently unreferenced: gamma is still computed throughout,
    // just not shown in the UI. Delete this entry only if that changes for good.
    Definition {
        key: "gamma",
        title: "Gamma — how fast delta shifts",
        body: "How much your price sensitivity itself changes as the stock moves. High gamma means the position's behaviour changes quickly, so today's hedge may be wrong tomorrow.",
    },
    Definition {
        key: "vega",
        title: "Vega — sensitivity to volatility",
        body: "How many dollars you gain or lose if the market's expectation of future swings rises by one percentage point. Sellers of options are usually hurt when volatility jumps.",
    },
    Definition {
        key: "theta",
        title: "Theta — time decay",
        body: "How much value the position gains or loses per day just from time passing. If you've sold options this is normally positive: waiting pays you.",
    },
    Definition {
        key: "rho",
        title: "Rho — sensitivity to interest rates",
        body: "How much the position moves if interest rates change by one percentage point. It is usually the smallest of these numbers and rarely drives a decision.",
    },
    Definition {
        key: "greeks",
        title: "Sensitivities",
        body: "A set of numbers describing what actually moves this position — the stock price, time passing, volatility, or interest rates. Traders call them 'the Greeks' because each has a Greek letter.",
    },

    // ── Volatility and models ────────────────────────────────────────────────
    Definition {
        key: "volatility",
        title: "Volatility",
        body: "How much the stock is expected to swing, as a yearly percentage. 25% means a typical year moves the price about a quarter either way. Higher volatility makes every option more expensive.",
    },
    Definition {
        key: "implied-vol",
        title: "Implied volatility",
        body: "The volatility figure that makes the model agree with the option's actual market price. It's the market's own forecast of future swings, read backwards out of the price.",
    },
    Definition {
        key: "forward-vol",
        title: "Forward volatility",
        body: "Expected volatility over a future window — from your evaluation date out to expiry, rather than starting today. It's the right input when you're modelling a trade you haven't placed yet.",
    },
    Definition {
        key: "vol-surface",
        title: "Volatility surface",
        body: "A map of implied volatility across every strike and expiry for one stock. It shows that the market doesn't expect the same swings for a far-out-of-the-money option as for an at-the-money one.",
    },
    Definition {
        key: "risk-free-rate",
        title: "Risk-free rate",
        body: "The return on cash held safely, roughly the Treasury yield. It sets the value of deferring payment, so it nudges option prices slightly. The default here is fine unless rates have moved a lot.",
    },
    Definition {
        key: "black-scholes",
        title: "Black-Scholes",
        body: "The standard formula for pricing an option from the stock price, strike, time left, volatility, and interest rate. It's a model, not a guarantee — its answer is a reference point, not the price you'll get filled at.",
    },
    Definition {
        key: "combo",
        title: "Combination",
        body: "Two or more option contracts tracked as a single trade, such as the pair of contracts in a roll. Only the combined price matters, so it's worth watching as one number.",
    },

    // ── Scenarios ────────────────────────────────────────────────────────────
    Definition {
        key: "scenario",
        title: "Scenario",
        body: "A what-if: a stock price you name, a date, and the trades you're considering. The result shows the cash, the taxable gain, and how your risk changes — before you commit to anything.",
    },
    Definition {
        key: "evaluation-date",
        title: "Evaluation date",
        body: "The date you're imagining this playing out on. It decides how much time value the options still have and whether any gains count as long-term.",
    },
    Definition {
        key: "closes-position",
        title: "Closing an existing position",
        body: "Link this trade to something you already hold and the gain or loss is worked out from your real cost basis and purchase date, instead of being treated as a brand-new trade.",
    },
    Definition {
        key: "upside-cap",
        title: "Capped upside",
        body: "The most your shares can be sold for if every call you've sold is exercised. Selling calls at a higher strike raises this ceiling; selling more calls at a lower strike lowers it.",
    },
    Definition {
        key: "uncovered-shares",
        title: "Uncovered shares",
        body: "Shares you own that no sold call is written against. They're free to keep rising — you just aren't collecting premium on them.",
    },
    Definition {
        key: "excess-short-calls",
        title: "More calls than shares",
        body: "You've sold more call contracts than you have shares to deliver. The surplus is uncovered, which means losses on it are open-ended if the stock climbs. Usually a mistake worth fixing.",
    },

    // ── Tax ──────────────────────────────────────────────────────────────────
    Definition {
        key: "short-term-gain",
        title: "Short-term gain",
        body: "Profit on something held a year or less. It's taxed at your ordinary income rate — the same rate as your salary, and the higher of the two rates.",
    },
    Definition {
        key: "long-term-gain",
        title: "Long-term gain",
        body: "Profit on something held more than a year. It's taxed at preferential rates (0%, 15%, or 20%), which is why holding past the one-year mark can be worth real money.",
    },
    Definition {
        key: "holding-period",
        title: "Holding period",
        body: "How long you've owned the shares. Cross one year and the gain is taxed at the lower long-term rate, so the date you bought matters as much as the price.",
    },
    Definition {
        key: "implied-tax",
        title: "Tax if you closed this one on its own",
        body: "What closing just this position would add to your federal tax — selling it if you're long, buying it back if you've sold it short. It deliberately ignores your other positions, so if you've sold a call against this stock, the loss on that call isn't counted here. A losing position shows a saving instead, because it shelters gains elsewhere. For the number that nets everything together, use the portfolio total at the top.",
    },
    Definition {
        key: "portfolio-tax",
        title: "Tax if you closed everything today",
        body: "What you'd owe if you sold up completely. Gains and losses are added together before the tax is worked out — which is how your return actually does it — so this is not the sum of the per-position figures above. That matters most for a covered call: as the stock climbs, its gain grows without limit while the call you sold runs an equally growing loss, and the two cancel. This total stays roughly flat, which is the truth of it; the individual rows are the ones that mislead.",
    },
    Definition {
        key: "baseline-tax",
        title: "Baseline tax",
        body: "The federal tax you'd owe this year from your income profile alone, with none of these trades. It's the reference point the scenario's tax impact is measured against.",
    },
    Definition {
        key: "filing-status",
        title: "Filing status",
        body: "How you file your return — single, married filing jointly, and so on. It sets the width of every tax bracket, so it changes the answer more than almost any other field here.",
    },
    Definition {
        key: "qualified-dividends",
        title: "Qualified dividends",
        body: "Dividends that qualify for the lower long-term capital-gains rates rather than ordinary income rates. Most dividends from US companies you've held a while are qualified; your 1099-DIV splits them out.",
    },
    Definition {
        key: "non-qualified-dividends",
        title: "Non-qualified dividends",
        body: "Dividends taxed at your ordinary income rate. Enter these separately from qualified ones, because the two are taxed under completely different schedules.",
    },
    Definition {
        key: "standard-deduction",
        title: "Standard vs itemised",
        body: "A flat amount everyone can subtract from income, or the sum of your actual deductible expenses if that's larger. Pick whichever is bigger — most people take the standard deduction.",
    },
    Definition {
        key: "carryforward-loss",
        title: "Carried-forward loss",
        body: "Investment losses from earlier years you haven't used yet. They offset this year's gains before any tax is due, so entering them can cut the estimate substantially. Enter as a positive number.",
    },
    Definition {
        key: "tax-line-items",
        title: "Line items vs snapshot",
        body: "Snapshot means entering one total per income category. Line items means logging each amount as it happens and letting the totals build up. Both feed the same calculation — pick whichever you'll actually keep current.",
    },
];
