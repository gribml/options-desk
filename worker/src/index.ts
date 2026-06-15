// ── Shared types ──────────────────────────────────────────────────────────────

interface Quote {
  symbol: string;
  price: number;
  change: number;
  change_pct: number;
}

interface OptionQuote {
  price: number;
  implied_vol: number | null;
}

interface OptionChainEntry {
  symbol: string;
  underlying: string;
  expiry: string;
  type: 'call' | 'put';
  strike: number;
  bid: number | null;
  ask: number | null;
  mid: number | null;
  implied_vol: number | null;
  delta: number | null;
  gamma: number | null;
  theta: number | null;
  vega: number | null;
  open_interest: number | null;
  volume: number | null;
}

// ── Worker env ────────────────────────────────────────────────────────────────

interface Env {
  SUPABASE_URL: string;
  SUPABASE_ANON_KEY: string;
  ALPACA_KEY: string;
  ALPACA_SECRET: string;
  DB: D1Database;
  ALLOWED_ORIGIN: string; // e.g. https://options-desk.pages.dev
}

// ── Helpers ───────────────────────────────────────────────────────────────────

function corsHeaders(origin: string): Record<string, string> {
  return {
    'Access-Control-Allow-Origin': origin,
    'Access-Control-Allow-Methods': 'GET, POST, OPTIONS',
    'Access-Control-Allow-Headers': 'Content-Type, Authorization',
  };
}

// Plain JSON — CORS is added by the entry point after origin validation.
function jsonResp(data: unknown, status = 200): Response {
  return new Response(JSON.stringify(data), {
    status,
    headers: { 'Content-Type': 'application/json' },
  });
}

function withCors(response: Response, cors: Record<string, string>): Response {
  const headers = new Headers(response.headers);
  for (const [k, v] of Object.entries(cors)) headers.set(k, v);
  return new Response(response.body, { status: response.status, headers });
}

function supabaseBase(env: Env): string {
  return env.SUPABASE_URL.replace(/\/rest\/v1\/?$/, '').replace(/\/$/, '');
}

interface SupabaseUser {
  id: string;
  email?: string;
}

// Verifies the Supabase JWT by calling /auth/v1/user, which also returns the
// user record — so callers get the authenticated user_id for free. Returns null
// when the token is missing or invalid.
async function verifyToken(request: Request, env: Env): Promise<SupabaseUser | null> {
  const auth = request.headers.get('Authorization');
  if (!auth?.startsWith('Bearer ')) return null;
  const resp = await fetch(`${supabaseBase(env)}/auth/v1/user`, {
    headers: { apikey: env.SUPABASE_ANON_KEY, Authorization: auth },
  });
  if (!resp.ok) return null;
  return await resp.json<SupabaseUser>();
}

// ── Route handlers ────────────────────────────────────────────────────────────

// GET /quote?symbol=AAPL
// Returns the latest price and day-over-day change from bars_1min.
async function handleQuote(symbol: string, env: Env): Promise<Response> {
  const { results } = await env.DB.prepare(`
    SELECT date(timestamp) AS day, close
    FROM bars_1min
    WHERE symbol = ?
      AND timestamp IN (SELECT MAX(timestamp) FROM bars_1min WHERE symbol = ? GROUP BY date(timestamp))
    ORDER BY day DESC
    LIMIT 2
  `).bind(symbol, symbol).all<{ day: string; close: number }>();

  if (!results.length) return jsonResp({ error: `No data for ${symbol}` }, 404);

  const price = results[0].close;
  const prevClose = results[1]?.close ?? price;
  const change = price - prevClose;
  const change_pct = prevClose !== 0 ? (change / prevClose) * 100 : 0;

  return jsonResp({ symbol, price, change, change_pct });
}

// GET /option-quote?symbol=AAPL&expiry=2025-01-17&type=call&strike=150
async function handleOptionQuote(url: URL, env: Env): Promise<Response> {
  const symbol = url.searchParams.get('symbol')?.toUpperCase();
  const expiry = url.searchParams.get('expiry');
  const type = url.searchParams.get('type') as 'call' | 'put' | null;
  const strikeStr = url.searchParams.get('strike');

  if (!symbol || !expiry || !type || !strikeStr) {
    return jsonResp({ error: 'symbol, expiry, type, and strike are required' }, 400);
  }
  if (type !== 'call' && type !== 'put') {
    return jsonResp({ error: 'type must be "call" or "put"' }, 400);
  }
  const strike = parseFloat(strikeStr);
  if (isNaN(strike)) return jsonResp({ error: 'strike must be a number' }, 400);

  const row = await env.DB.prepare(`
    SELECT (bid + ask) / 2.0 AS mid, implied_volatility AS implied_vol
    FROM option_chain
    WHERE underlying = ?
      AND expiration = ?
      AND option_type = ?
      AND strike = ?
    ORDER BY snapshot_date DESC
    LIMIT 1
  `).bind(symbol, expiry, type, strike)
    .first<{ mid: number | null; implied_vol: number | null }>();

  if (!row) return jsonResp({ error: 'No data for option contract' }, 404);

  return jsonResp({ price: row.mid ?? 0, implied_vol: row.implied_vol });
}

// GET /option-chain?symbol=AAPL
// Returns all contracts from the latest snapshot for the underlying.
async function handleOptionChain(url: URL, env: Env): Promise<Response> {
  const symbol = url.searchParams.get('symbol')?.toUpperCase();
  if (!symbol) return jsonResp({ error: 'symbol required' }, 400);

  const { results } = await env.DB.prepare(`
    SELECT symbol, underlying, expiration AS expiry, option_type AS type, strike,
           bid, ask, (bid + ask) / 2.0 AS mid, implied_volatility AS implied_vol,
           delta, gamma, theta, vega,
           NULL AS open_interest, NULL AS volume
    FROM option_chain
    WHERE underlying = ?
      AND snapshot_date = (SELECT MAX(snapshot_date) FROM option_chain WHERE underlying = ?)
    ORDER BY expiration, option_type, strike
  `).bind(symbol, symbol).all<OptionChainEntry>();

  return jsonResp(results);
}

// GET /option-meta?symbol=AAPL
// Returns distinct (expiry, option_type, strike) tuples from the latest snapshot.
// Lighter than /option-chain — used to populate expiry/strike dropdowns in the UI.
async function handleOptionMeta(url: URL, env: Env): Promise<Response> {
  const symbol = url.searchParams.get('symbol')?.toUpperCase();
  if (!symbol) return jsonResp({ error: 'symbol required' }, 400);

  const { results } = await env.DB.prepare(`
    SELECT DISTINCT expiration AS expiry, option_type, strike
    FROM option_chain
    WHERE underlying = ?
      AND snapshot_date = (SELECT MAX(snapshot_date) FROM option_chain WHERE underlying = ?)
    ORDER BY expiration, option_type, strike
  `).bind(symbol, symbol).all<{ expiry: string; option_type: string; strike: number }>();

  return jsonResp(results);
}

// GET /close-prices?symbol=AAPL&limit=31
// Returns daily close prices (most recent first) derived from bars_1min.
async function handleClosePrices(url: URL, env: Env): Promise<Response> {
  const symbol = url.searchParams.get('symbol')?.toUpperCase();
  if (!symbol) return jsonResp({ error: 'symbol required' }, 400);

  const limit = Math.min(parseInt(url.searchParams.get('limit') ?? '31') || 31, 500);

  const { results } = await env.DB.prepare(`
    SELECT close
    FROM bars_1min
    WHERE symbol = ?
      AND timestamp IN (SELECT MAX(timestamp) FROM bars_1min WHERE symbol = ? GROUP BY date(timestamp))
    ORDER BY date(timestamp) DESC
    LIMIT ?
  `).bind(symbol, symbol, limit).all<{ close: number }>();

  return jsonResp(results.map(r => r.close));
}

// ── Forward volatility ────────────────────────────────────────────────────────

function yearsUntil(today: Date, target: Date): number {
  return (target.getTime() - today.getTime()) / (365.25 * 24 * 3600 * 1000);
}

// Linear interpolation of total variance V(T) = atm_vol² × T.
// Extrapolates flat vol outside the observed range.
function interpVariance(curve: Array<{ T: number; V: number }>, T: number): number {
  if (curve.length === 0 || T <= 0) return 0;
  if (T <= curve[0].T) return curve[0].V * (T / curve[0].T);
  const last = curve[curve.length - 1];
  if (T >= last.T) return last.V + (last.V / last.T) * (T - last.T);
  for (let i = 0; i < curve.length - 1; i++) {
    const lo = curve[i], hi = curve[i + 1];
    if (T >= lo.T && T <= hi.T) {
      const w = (T - lo.T) / (hi.T - lo.T);
      return lo.V + w * (hi.V - lo.V);
    }
  }
  return last.V;
}

// GET /forward-vol?symbol=AAPL&eval_date=2025-06-15&expiry=2025-09-19
// Returns the forward volatility from eval_date to expiry derived from the
// SABR ATM variance curve.  Variance is additive:
//   σ_fwd² × (T2−T1) = σ_atm(T2)² × T2 − σ_atm(T1)² × T1
async function handleForwardVol(url: URL, env: Env): Promise<Response> {
  const symbol      = url.searchParams.get('symbol')?.toUpperCase();
  const evalDateStr = url.searchParams.get('eval_date');
  const expiryStr   = url.searchParams.get('expiry');

  if (!symbol || !evalDateStr || !expiryStr) {
    return jsonResp({ error: 'symbol, eval_date, and expiry are required' }, 400);
  }

  const today    = new Date(); today.setUTCHours(0, 0, 0, 0);
  const evalDate = new Date(evalDateStr + 'T00:00:00Z');
  const expiry   = new Date(expiryStr   + 'T00:00:00Z');

  if (evalDate >= expiry) return jsonResp({ error: 'eval_date must be before expiry' }, 400);

  const T1 = yearsUntil(today, evalDate);
  const T2 = yearsUntil(today, expiry);

  if (T1 <= 0) return jsonResp({ error: 'eval_date must be in the future' }, 400);
  if (T2 <= 0) return jsonResp({ error: 'expiry must be in the future' }, 400);

  const { results } = await env.DB.prepare(`
    SELECT expiry, atm_vol
    FROM vol_surface
    WHERE underlying = ?
      AND snapshot_date = (SELECT MAX(snapshot_date) FROM vol_surface WHERE underlying = ?)
    ORDER BY expiry
  `).bind(symbol, symbol).all<{ expiry: string; atm_vol: number }>();

  if (!results.length) {
    return jsonResp({ error: `No vol surface data for ${symbol}` }, 404);
  }

  const curve = results
    .map(r => {
      const T = yearsUntil(today, new Date(r.expiry + 'T00:00:00Z'));
      return { T, V: r.atm_vol * r.atm_vol * T };
    })
    .filter(p => p.T > 0);

  if (!curve.length) return jsonResp({ error: 'All vol surface expiries have passed' }, 400);

  const V1 = interpVariance(curve, T1);
  const V2 = interpVariance(curve, T2);

  if (V2 <= V1) {
    return jsonResp({ error: 'Non-positive forward variance — surface may be stale' }, 400);
  }

  const forward_vol = Math.sqrt((V2 - V1) / (T2 - T1));
  const atm_vol_t1  = T1 > 0 ? Math.sqrt(V1 / T1) : 0;
  const atm_vol_t2  = Math.sqrt(V2 / T2);

  return jsonResp({ forward_vol, atm_vol_t1, atm_vol_t2, t1_years: T1, t2_years: T2 });
}

// GET /term-rates?symbol=AAPL
// Returns implied risk-free rates per expiry from the latest pipeline snapshot.
async function handleTermRates(url: URL, env: Env): Promise<Response> {
  const symbol = url.searchParams.get('symbol')?.toUpperCase();
  if (!symbol) return jsonResp({ error: 'symbol required' }, 400);

  const { results } = await env.DB.prepare(`
    SELECT expiry, rate, num_contracts
    FROM implied_rates
    WHERE underlying = ?
      AND snapshot_date = (SELECT MAX(snapshot_date) FROM implied_rates WHERE underlying = ?)
    ORDER BY expiry
  `).bind(symbol, symbol).all<{ expiry: string; rate: number; num_contracts: number | null }>();

  if (!results.length) return jsonResp({ error: `No term rate data for ${symbol}` }, 404);

  return jsonResp(results);
}

function isStock(symbol: string): boolean {
  return !/\d/.test(symbol);
}

// GET /latest-bar?symbol=AAPL
// Fetches the latest bar from Alpaca and caches the result in D1 for 15 minutes.
async function handleLatestBar(url: URL, env: Env): Promise<Response> {
  const symbol = url.searchParams.get('symbol')?.toUpperCase();
  if (!symbol) return jsonResp({ error: 'symbol required' }, 400);

  const cached = await env.DB.prepare(`
    SELECT symbol, bar_time, fetched_at, open, high, low, close, volume, trade_count, vwap
    FROM latest_bars_cache
    WHERE symbol = ?
      AND fetched_at >= datetime('now', '-15 minutes')
  `).bind(symbol).first<{
    symbol: string; bar_time: string; fetched_at: string;
    open: number; high: number; low: number; close: number;
    volume: number; trade_count: number; vwap: number;
  }>();

  if (cached) {
    return jsonResp({ ...cached, cached: true });
  }

  let api_endpoint: string = "";
  if (isStock(symbol)) {
    api_endpoint = `v2/stocks/${encodeURIComponent(symbol)}/bars/latest`
  } else {
    api_endpoint = `v1beta1/options/quotes/latest?symbols=${encodeURIComponent(symbol)}&feed=indicative`
  }

  const alpacaResp = await fetch(
    `https://data.alpaca.markets/${api_endpoint}`,
    { headers: { 'APCA-API-KEY-ID': env.ALPACA_KEY, 'APCA-API-SECRET-KEY': env.ALPACA_SECRET } },
  );

  if (!alpacaResp.ok) {
    const text = await alpacaResp.text();
    return jsonResp({ error: `Alpaca error: ${text}` }, alpacaResp.status);
  }

  const data = await alpacaResp.json<{ bar: { o: number; h: number; l: number; c: number; v: number; vw: number; n: number; t: string }; symbol: string }>();
  const bar = data.bar;
  if (!bar) return jsonResp({ error: `No bar data returned for ${symbol}` }, 404);

  await env.DB.prepare(`
    INSERT OR REPLACE INTO latest_bars_cache
      (symbol, bar_time, fetched_at, open, high, low, close, volume, trade_count, vwap)
    VALUES (?, ?, datetime('now'), ?, ?, ?, ?, ?, ?, ?)
  `).bind(symbol, bar.t, bar.o, bar.h, bar.l, bar.c, bar.v, bar.n, bar.vw).run();

  return jsonResp({
    symbol,
    bar_time: bar.t,
    open: bar.o,
    high: bar.h,
    low: bar.l,
    close: bar.c,
    volume: bar.v,
    trade_count: bar.n,
    vwap: bar.vw,
    cached: false,
  });
}

// ── Federal tax engine ──────────────────────────────────────────────────────────
//
// SCOPE: U.S. FEDERAL tax only. AMT and state/local tax are OUT OF SCOPE.
// Computes ordinary-income tax (progressive brackets), long-term capital-gains /
// qualified-dividend tax (0/15/20% stacked on top of ordinary taxable income),
// and the 3.8% Net Investment Income Tax (NIIT). Carryforward losses net against
// current gains, with up to $3,000 of net capital loss deducting against ordinary
// income (remainder is not consumed in a single-year computation).

type Filing = 'single' | 'mfj' | 'mfs' | 'hoh';

interface Bracket {
  upTo: number; // inclusive upper bound of this band; Infinity for the top band
  rate: number;
}

interface YearConstants {
  stdDeduction: Record<Filing, number>;
  ordinary: Record<Filing, Bracket[]>;
  ltcg: Record<Filing, Bracket[]>;
  niitThreshold: Record<Filing, number>;
}

const NIIT_RATE = 0.038;

// NIIT MAGI thresholds are fixed by statute (not inflation-adjusted).
const NIIT_THRESHOLD: Record<Filing, number> = {
  single: 200_000,
  mfj: 250_000,
  mfs: 125_000,
  hoh: 200_000,
};

// ── FEDERAL TAX CONSTANTS BY YEAR (UPDATE YEARLY) ──
// TODO confirm against the IRS annual Rev. Proc. before relying on exact figures.
// 2025 = Rev. Proc. 2024-40; 2026 = Rev. Proc. 2025-32.
const TAX_CONSTANTS: Record<number, YearConstants> = {
  2025: {
    stdDeduction: { single: 15_750, mfj: 31_500, mfs: 15_750, hoh: 23_625 },
    ordinary: {
      single: [
        { upTo: 11_925, rate: 0.10 }, { upTo: 48_475, rate: 0.12 },
        { upTo: 103_350, rate: 0.22 }, { upTo: 197_300, rate: 0.24 },
        { upTo: 250_525, rate: 0.32 }, { upTo: 626_350, rate: 0.35 },
        { upTo: Infinity, rate: 0.37 },
      ],
      mfj: [
        { upTo: 23_850, rate: 0.10 }, { upTo: 96_950, rate: 0.12 },
        { upTo: 206_700, rate: 0.22 }, { upTo: 394_600, rate: 0.24 },
        { upTo: 501_050, rate: 0.32 }, { upTo: 751_600, rate: 0.35 },
        { upTo: Infinity, rate: 0.37 },
      ],
      mfs: [
        { upTo: 11_925, rate: 0.10 }, { upTo: 48_475, rate: 0.12 },
        { upTo: 103_350, rate: 0.22 }, { upTo: 197_300, rate: 0.24 },
        { upTo: 250_525, rate: 0.32 }, { upTo: 375_800, rate: 0.35 },
        { upTo: Infinity, rate: 0.37 },
      ],
      hoh: [
        { upTo: 17_000, rate: 0.10 }, { upTo: 64_850, rate: 0.12 },
        { upTo: 103_350, rate: 0.22 }, { upTo: 197_300, rate: 0.24 },
        { upTo: 250_500, rate: 0.32 }, { upTo: 626_350, rate: 0.35 },
        { upTo: Infinity, rate: 0.37 },
      ],
    },
    ltcg: {
      single: [{ upTo: 48_350, rate: 0.0 }, { upTo: 533_400, rate: 0.15 }, { upTo: Infinity, rate: 0.20 }],
      mfj: [{ upTo: 96_700, rate: 0.0 }, { upTo: 600_050, rate: 0.15 }, { upTo: Infinity, rate: 0.20 }],
      mfs: [{ upTo: 48_350, rate: 0.0 }, { upTo: 300_000, rate: 0.15 }, { upTo: Infinity, rate: 0.20 }],
      hoh: [{ upTo: 64_750, rate: 0.0 }, { upTo: 566_700, rate: 0.15 }, { upTo: Infinity, rate: 0.20 }],
    },
    niitThreshold: NIIT_THRESHOLD,
  },
  2026: {
    stdDeduction: { single: 16_100, mfj: 32_200, mfs: 16_100, hoh: 24_150 },
    ordinary: {
      single: [
        { upTo: 12_400, rate: 0.10 }, { upTo: 50_400, rate: 0.12 },
        { upTo: 105_700, rate: 0.22 }, { upTo: 201_775, rate: 0.24 },
        { upTo: 256_225, rate: 0.32 }, { upTo: 640_600, rate: 0.35 },
        { upTo: Infinity, rate: 0.37 },
      ],
      mfj: [
        { upTo: 24_800, rate: 0.10 }, { upTo: 100_800, rate: 0.12 },
        { upTo: 211_400, rate: 0.22 }, { upTo: 403_550, rate: 0.24 },
        { upTo: 512_450, rate: 0.32 }, { upTo: 768_700, rate: 0.35 },
        { upTo: Infinity, rate: 0.37 },
      ],
      mfs: [
        { upTo: 12_400, rate: 0.10 }, { upTo: 50_400, rate: 0.12 },
        { upTo: 105_700, rate: 0.22 }, { upTo: 201_775, rate: 0.24 },
        { upTo: 256_225, rate: 0.32 }, { upTo: 384_350, rate: 0.35 },
        { upTo: Infinity, rate: 0.37 },
      ],
      hoh: [
        { upTo: 17_700, rate: 0.10 }, { upTo: 67_450, rate: 0.12 },
        { upTo: 105_700, rate: 0.22 }, { upTo: 201_775, rate: 0.24 },
        { upTo: 256_200, rate: 0.32 }, { upTo: 640_600, rate: 0.35 },
        { upTo: Infinity, rate: 0.37 },
      ],
    },
    ltcg: {
      single: [{ upTo: 49_450, rate: 0.0 }, { upTo: 545_500, rate: 0.15 }, { upTo: Infinity, rate: 0.20 }],
      mfj: [{ upTo: 98_900, rate: 0.0 }, { upTo: 613_700, rate: 0.15 }, { upTo: Infinity, rate: 0.20 }],
      mfs: [{ upTo: 49_450, rate: 0.0 }, { upTo: 306_850, rate: 0.15 }, { upTo: Infinity, rate: 0.20 }],
      hoh: [{ upTo: 66_200, rate: 0.0 }, { upTo: 579_600, rate: 0.15 }, { upTo: Infinity, rate: 0.20 }],
    },
    niitThreshold: NIIT_THRESHOLD,
  },
};

// Returns the constants for `year`, falling back to the latest known year for
// future years (future inflation-adjusted brackets are unknowable) and the
// earliest known year for years before the table.
function constantsFor(year: number): YearConstants {
  if (TAX_CONSTANTS[year]) return TAX_CONSTANTS[year];
  const years = Object.keys(TAX_CONSTANTS).map(Number).sort((a, b) => a - b);
  const clamped = year > years[years.length - 1] ? years[years.length - 1] : years[0];
  return TAX_CONSTANTS[clamped];
}

interface TaxInputs {
  filing_status: Filing;
  w2_income: number;
  interest_income: number;
  ordinary_dividends: number;
  qualified_dividends: number; // subset of ordinary_dividends
  st_capital_gains: number;
  lt_capital_gains: number;
  rental_income: number;
  deduction_choice: 'standard' | 'itemized';
  itemized_deductions: number;
  carryforward_st_loss: number; // positive number = a loss carried in
  carryforward_lt_loss: number;
}

// Tax on the income interval [lo, hi] walked across `brackets`.
function tieredTaxOnInterval(lo: number, hi: number, brackets: Bracket[]): number {
  let tax = 0;
  let prev = 0;
  for (const b of brackets) {
    const bandLo = Math.max(lo, prev);
    const bandHi = Math.min(hi, b.upTo);
    if (bandHi > bandLo) tax += (bandHi - bandLo) * b.rate;
    prev = b.upTo;
    if (prev >= hi) break;
  }
  return tax;
}

function progressiveTax(taxable: number, brackets: Bracket[]): number {
  return tieredTaxOnInterval(0, Math.max(0, taxable), brackets);
}

// Nets carryforward losses against current gains (ST↔ST, LT↔LT, then cross-net),
// returning the ordinary-bound short-term component, the long-term stack
// component, and any ordinary loss deduction (capped at $3,000).
function nettCapitalGains(inp: TaxInputs): {
  ordinaryStComponent: number;
  ltComponent: number;
  ordinaryLossDeduction: number;
} {
  let netSt = inp.st_capital_gains - inp.carryforward_st_loss;
  let netLt = inp.lt_capital_gains - inp.carryforward_lt_loss;

  // Cross-net a loss in one bucket against a gain in the other.
  if (netSt < 0 && netLt > 0) {
    const use = Math.min(-netSt, netLt);
    netSt += use;
    netLt -= use;
  } else if (netLt < 0 && netSt > 0) {
    const use = Math.min(-netLt, netSt);
    netLt += use;
    netSt -= use;
  }

  // IRC §1211(b): MFS cap is $1,500; all other statuses cap at $3,000.
  const lossDeductionCap = inp.filing_status === 'mfs' ? 1_500 : 3_000;
  const totalNet = netSt + netLt;
  const ordinaryLossDeduction = totalNet < 0 ? Math.min(lossDeductionCap, -totalNet) : 0;

  return {
    ordinaryStComponent: Math.max(0, netSt),
    ltComponent: Math.max(0, netLt),
    ordinaryLossDeduction,
  };
}

function computeFederalTax(inp: TaxInputs, year: number): number {
  const c = constantsFor(year);
  const fs = inp.filing_status;
  const cap = nettCapitalGains(inp);

  const qualDiv = Math.min(inp.qualified_dividends, inp.ordinary_dividends);
  const nonQualDiv = Math.max(0, inp.ordinary_dividends - qualDiv);
  const ordinaryIncome = Math.max(
    0,
    inp.w2_income + inp.interest_income + nonQualDiv + cap.ordinaryStComponent +
      inp.rental_income - cap.ordinaryLossDeduction,
  );

  const deduction = inp.deduction_choice === 'itemized'
    ? Math.max(0, inp.itemized_deductions)
    : c.stdDeduction[fs];

  // Deduction applies to ordinary income first; any excess reduces the LT stack.
  const ordinaryTaxable = Math.max(0, ordinaryIncome - deduction);
  const remainingDeduction = Math.max(0, deduction - ordinaryIncome);
  const ltStack = cap.ltComponent + qualDiv;
  const ltTaxable = Math.max(0, ltStack - remainingDeduction);

  const ordinaryTax = progressiveTax(ordinaryTaxable, c.ordinary[fs]);
  const ltcgTax = tieredTaxOnInterval(ordinaryTaxable, ordinaryTaxable + ltTaxable, c.ltcg[fs]);

  // NIIT: 3.8% on the lesser of net investment income and (MAGI − threshold).
  // Rental income is treated as investment income here (an approximation).
  const nii = inp.interest_income + inp.ordinary_dividends + cap.ordinaryStComponent +
    cap.ltComponent + inp.rental_income;
  const magi = ordinaryIncome + ltStack;
  const niit = NIIT_RATE * Math.max(0, Math.min(nii, magi - c.niitThreshold[fs]));

  return ordinaryTax + ltcgTax + niit;
}

// Marginal tax incurred by realizing an incremental ST/LT gain on top of the
// baseline profile: tax(baseline + gains) − tax(baseline).
function marginalTradeTax(
  baseline: TaxInputs,
  gains: { st_gain: number; lt_gain: number },
  year: number,
  baselineTax: number,
): number {
  const withTrade: TaxInputs = {
    ...baseline,
    st_capital_gains: baseline.st_capital_gains + gains.st_gain,
    lt_capital_gains: baseline.lt_capital_gains + gains.lt_gain,
  };
  return computeFederalTax(withTrade, year) - baselineTax;
}

// POST /tax — compute marginal federal tax for a trade (or batch of positions)
// against the authenticated user's stored income profile for the given year.
//   single: { tax_year, st_gain, lt_gain }            → { tax, baseline_tax }
//   batch:  { tax_year, items: [{ id, st_gain, lt_gain }] } → { results: [{ id, tax }] }
async function handleTax(
  request: Request,
  user: SupabaseUser,
  authHeader: string,
  env: Env,
): Promise<Response> {
  let body: any;
  try {
    body = await request.json();
  } catch {
    return jsonResp({ error: 'Invalid JSON body' }, 400);
  }

  const taxYear = Number(body?.tax_year);
  if (!Number.isFinite(taxYear)) return jsonResp({ error: 'tax_year required' }, 400);
  if (!Number.isInteger(taxYear)) return jsonResp({ error: 'tax year must be an integer' }, 400);

  // Read the user's profile for this year (RLS-scoped via the user's own JWT).
  const profileUrl =
    `${supabaseBase(env)}/rest/v1/tax_profiles?user_id=eq.${user.id}` +
    `&tax_year=eq.${taxYear}&select=payload`;
  const resp = await fetch(profileUrl, {
    headers: { apikey: env.SUPABASE_ANON_KEY, Authorization: authHeader },
  });
  if (!resp.ok) return jsonResp({ error: 'Failed to read tax profile' }, 502);

  const rows = await resp.json<Array<{ payload: { revisions?: TaxInputs[] } }>>();
  const revisions = rows[0]?.payload?.revisions;
  if (!revisions || revisions.length === 0) {
    return jsonResp({ error: `No tax profile for ${taxYear}` }, 422);
  }
  const baseline = revisions[revisions.length - 1];

  if (Array.isArray(body.items)) {
    const baselineTax = computeFederalTax(baseline, taxYear);
    for (const it of body.items) {
      const st = Number(it?.st_gain ?? 0);
      const lt = Number(it?.lt_gain ?? 0);
      if (typeof it?.id !== 'string' || !Number.isFinite(st) || !Number.isFinite(lt)) {
        return jsonResp({ error: 'items must include id, st_gain, lt_gain (numbers)' }, 400);
      }
    }
    const results = body.items.map((it: { id: string; st_gain: number; lt_gain: number }) => {
      const st = Number(it.st_gain ?? 0);
      const lt = Number(it.lt_gain ?? 0);
      return { id: it.id, tax: marginalTradeTax(baseline, { st_gain: st, lt_gain: lt }, taxYear, baselineTax) };
    });
    return jsonResp({ results });
  }

  const stGain = Number(body?.st_gain ?? 0);
  const ltGain = Number(body?.lt_gain ?? 0);
  if (!Number.isFinite(stGain) || !Number.isFinite(ltGain)) {
    return jsonResp({ error: 'st_gain and lt_gain must be numbers' }, 400);
  }

  const tax = marginalTradeTax(
    baseline,
    { st_gain: stGain, lt_gain: ltGain },
    taxYear,
  );
  const baseline_tax = computeFederalTax(baseline, taxYear);
  return jsonResp({ tax, baseline_tax });
}

// ── Entry point ───────────────────────────────────────────────────────────────

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    const origin = request.headers.get('Origin') ?? '';
    const cors = corsHeaders(env.ALLOWED_ORIGIN);

    if (request.method === 'OPTIONS') {
      if (!env.ALLOWED_ORIGIN || origin !== env.ALLOWED_ORIGIN) return new Response(null, { status: 403 });
      return new Response(null, { headers: cors });
    }

    // Reject requests from any origin other than the configured app origin.
    if (!env.ALLOWED_ORIGIN || origin !== env.ALLOWED_ORIGIN) {
      return jsonResp({ error: 'Forbidden' }, 403);
    }

    try {
      const user = await verifyToken(request, env);
      if (!user) return withCors(jsonResp({ error: 'Unauthorized' }, 401), cors);
      const authHeader = request.headers.get('Authorization')!;

      const url = new URL(request.url);

      const response = await (async (): Promise<Response> => {
        if (url.pathname === '/quote' && request.method === 'GET') {
          const symbol = url.searchParams.get('symbol')?.toUpperCase();
          if (!symbol) return jsonResp({ error: 'symbol required' }, 400);
          return handleQuote(symbol, env);
        }

        if (url.pathname === '/option-quote' && request.method === 'GET') return handleOptionQuote(url, env);
        if (url.pathname === '/option-chain' && request.method === 'GET') return handleOptionChain(url, env);
        if (url.pathname === '/option-meta' && request.method === 'GET') return handleOptionMeta(url, env);
        if (url.pathname === '/close-prices' && request.method === 'GET') return handleClosePrices(url, env);
        if (url.pathname === '/forward-vol' && request.method === 'GET') return handleForwardVol(url, env);
        if (url.pathname === '/term-rates' && request.method === 'GET') return handleTermRates(url, env);
        if (url.pathname === '/latest-bar' && request.method === 'GET') return handleLatestBar(url, env);
        if (url.pathname === '/tax' && request.method === 'POST') return handleTax(request, user, authHeader, env);

        return jsonResp({ error: 'Not found' }, 404);
      })();

      return withCors(response, cors);
    } catch (e) {
      return withCors(jsonResp({ error: e instanceof Error ? e.message : 'Internal error' }, 500), cors);
    }
  },
} satisfies ExportedHandler<Env>;
