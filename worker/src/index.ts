import {
  computeFederalTax,
  marginalTradeTax,
  sanitizeTaxInputs,
  constantsFor,
  MIN_TAX_YEAR,
  type TaxInputs,
} from './tax';

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
  last: number | null;
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
  ALLOWED_ORIGIN: string;  // production Pages URL — set via `wrangler secret put`
  DEV_ORIGIN?: string;     // optional extra origin — set as a [vars] in wrangler.local.toml
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

const ALPACA_BASE = 'https://data.alpaca.markets';
const ALPACA_TIMEOUT_MS = 8000;

// Error carrying an HTTP status, so upstream failures map to a sensible response
// code (504 timeout / 502 bad gateway) instead of a generic 500.
class UpstreamError extends Error {
  constructor(message: string, readonly status = 502) {
    super(message);
  }
}

// Fetch from Alpaca with auth headers and an AbortController timeout. A slow or
// hung upstream is aborted and surfaced as a clean 504 rather than hanging the
// request open — letting the frontend fall back to manual price entry.
async function fetchAlpaca(endpoint: string, env: Env): Promise<Response> {
  const ctrl = new AbortController();
  const timer = setTimeout(() => ctrl.abort(), ALPACA_TIMEOUT_MS);
  try {
    return await fetch(`${ALPACA_BASE}/${endpoint}`, {
      headers: { 'APCA-API-KEY-ID': env.ALPACA_KEY, 'APCA-API-SECRET-KEY': env.ALPACA_SECRET },
      signal: ctrl.signal,
    });
  } catch (e) {
    if (e instanceof DOMException && e.name === 'AbortError') {
      throw new UpstreamError(`Market data API timed out after ${ALPACA_TIMEOUT_MS}ms`, 504);
    }
    console.error('Market data fetch failed:', e);
    throw new UpstreamError('Market data API request failed', 502);
  } finally {
    clearTimeout(timer);
  }
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

  let row = await env.DB.prepare(`
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

  // Fall back to the on-demand Alpaca cache when the pipeline has no row for
  // this contract — so cache-only symbols still price with real IV/mid.
  if (!row) {
    row = await env.DB.prepare(`
      SELECT (bid + ask) / 2.0 AS mid, implied_volatility AS implied_vol
      FROM option_chain_cache
      WHERE underlying = ?
        AND expiration = ?
        AND option_type = ?
        AND strike = ?
      LIMIT 1
    `).bind(symbol, expiry, type, strike)
      .first<{ mid: number | null; implied_vol: number | null }>();
  }

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

// GET /option-history?underlying=AAPL&expiry=2025-01-17&type=call&strike=150
// Historical per-contract time series from the accumulated option_chain
// snapshots: one point per snapshot_date with its mid price and implied vol.
// Used by the Combos tracker to plot a leg's price/vol history.
async function handleOptionHistory(url: URL, env: Env): Promise<Response> {
  const underlying = url.searchParams.get('underlying')?.toUpperCase();
  const expiry = url.searchParams.get('expiry');
  const type = url.searchParams.get('type');
  const strike = parseFloat(url.searchParams.get('strike') ?? '');

  if (!underlying || !expiry || (type !== 'call' && type !== 'put') || !Number.isFinite(strike)) {
    return jsonResp({ error: 'underlying, expiry, type (call|put), and strike are required' }, 400);
  }

  const { results } = await env.DB.prepare(`
    SELECT snapshot_date AS t,
           (bid + ask) / 2.0 AS mid,
           implied_volatility AS implied_vol
    FROM option_chain
    WHERE underlying = ? AND expiration = ? AND option_type = ? AND strike = ?
    ORDER BY snapshot_date
  `).bind(underlying, expiry, type, strike)
    .all<{ t: string; mid: number | null; implied_vol: number | null }>();

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

  const alpacaResp = await fetchAlpaca(api_endpoint, env);

  if (!alpacaResp.ok) {
    console.error(`Market data upstream error (${alpacaResp.status}):`, await alpacaResp.text());
    return jsonResp({ error: 'Market data API error' }, alpacaResp.status);
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

// ── On-demand option chain ──────────────────────────────────────────────────────

// Parses an OCC option symbol from the right (the suffix is fixed-width, the root
// is variable): ROOT + YYMMDD + C|P + STRIKE×1000 (8 digits, zero-padded).
//   AAPL250117C00150000 → { expiry: '2025-01-17', type: 'call', strike: 150 }
function parseOcc(occ: string): { expiry: string; type: 'call' | 'put'; strike: number } | null {
  if (occ.length < 16) return null;
  const suffix = occ.slice(-15);
  const ymd = suffix.slice(0, 6);
  const cp = suffix.slice(6, 7);
  const strikeRaw = suffix.slice(7);
  if (!/^\d{6}$/.test(ymd) || (cp !== 'C' && cp !== 'P') || !/^\d{8}$/.test(strikeRaw)) return null;
  const expiry = `20${ymd.slice(0, 2)}-${ymd.slice(2, 4)}-${ymd.slice(4, 6)}`;
  return { expiry, type: cp === 'C' ? 'call' : 'put', strike: parseInt(strikeRaw, 10) / 1000 };
}

interface AlpacaSnapshot {
  latestQuote?: { bp?: number; ap?: number };
  latestTrade?: { p?: number };
  greeks?: { delta?: number; gamma?: number; theta?: number; vega?: number; rho?: number };
  impliedVolatility?: number;
}

// GET /option-chain-live?symbol=AAPL[&page_token=...]
// On-demand option chain, paginated so each Alpaca page streams back to the
// frontend (which merges it into the expiry/strike dropdowns) while the next
// page is fetched. The first page (no page_token) serves from option_chain_cache
// when it is < 15 min old; otherwise it purges the stale rows and begins a fresh
// Alpaca paginated fetch, upserting each page into the cache as it goes.
async function handleOptionChainLive(url: URL, env: Env): Promise<Response> {
  const underlying = url.searchParams.get('symbol')?.toUpperCase();
  if (!underlying) return jsonResp({ error: 'symbol required' }, 400);
  const pageToken = url.searchParams.get('page_token');

  // First page: serve from cache if fresh, else purge stale rows and refetch.
  if (!pageToken) {
    const fresh = await env.DB.prepare(`
      SELECT 1 AS ok FROM option_chain_cache
      WHERE underlying = ? AND fetched_at >= datetime('now', '-15 minutes')
      LIMIT 1
    `).bind(underlying).first<{ ok: number }>();

    if (fresh) {
      const { results } = await env.DB.prepare(`
        SELECT symbol, underlying, expiration AS expiry, option_type AS type, strike,
               bid, ask, (bid + ask) / 2.0 AS mid, last_price AS last,
               implied_volatility AS implied_vol, delta, gamma, theta, vega,
               NULL AS open_interest, NULL AS volume
        FROM option_chain_cache
        WHERE underlying = ?
        ORDER BY expiration, option_type, strike
      `).bind(underlying).all<OptionChainEntry>();
      return jsonResp({ entries: results, next_page_token: null, cached: true });
    }

    await env.DB.prepare(`
      DELETE FROM option_chain_cache
      WHERE underlying = ? AND fetched_at < datetime('now', '-15 minutes')
    `).bind(underlying).run();
  }

  // Fetch one Alpaca page.
  let endpoint = `v1beta1/options/snapshots/${encodeURIComponent(underlying)}?feed=indicative&limit=1000`;
  if (pageToken) endpoint += `&page_token=${encodeURIComponent(pageToken)}`;

  const alpacaResp = await fetchAlpaca(endpoint, env);
  if (!alpacaResp.ok) {
    console.error(`Market data upstream error (${alpacaResp.status}):`, await alpacaResp.text());
    return jsonResp({ error: 'Market data API error' }, alpacaResp.status);
  }

  const data = await alpacaResp.json<{ snapshots?: Record<string, AlpacaSnapshot>; next_page_token: string | null }>();
  const snapshots = data.snapshots ?? {};

  const insert = env.DB.prepare(`
    INSERT OR REPLACE INTO option_chain_cache
      (symbol, underlying, fetched_at, expiration, option_type, strike,
       bid, ask, last_price, implied_volatility, delta, gamma, theta, vega, rho)
    VALUES (?, ?, datetime('now'), ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
  `);

  const entries: OptionChainEntry[] = [];
  const stmts = [];
  for (const [occ, snap] of Object.entries(snapshots)) {
    const parsed = parseOcc(occ);
    if (!parsed) continue;
    const bid = snap.latestQuote?.bp ?? 0;
    const ask = snap.latestQuote?.ap ?? 0;
    const mid = bid && ask ? (bid + ask) / 2 : (bid || ask || 0);
    const last = snap.latestTrade?.p ?? null;
    const iv = snap.impliedVolatility ?? null;
    const g = snap.greeks ?? {};

    entries.push({
      symbol: occ, underlying, expiry: parsed.expiry, type: parsed.type, strike: parsed.strike,
      bid, ask, mid, last, implied_vol: iv,
      delta: g.delta ?? null, gamma: g.gamma ?? null, theta: g.theta ?? null, vega: g.vega ?? null,
      open_interest: null, volume: null,
    });
    stmts.push(insert.bind(
      occ, underlying, parsed.expiry, parsed.type, parsed.strike,
      bid, ask, last, iv, g.delta ?? null, g.gamma ?? null, g.theta ?? null, g.vega ?? null, g.rho ?? null,
    ));
  }
  if (stmts.length) await env.DB.batch(stmts);

  entries.sort((a, b) =>
    a.expiry.localeCompare(b.expiry) || a.type.localeCompare(b.type) || a.strike - b.strike);

  return jsonResp({ entries, next_page_token: data.next_page_token ?? null, cached: false });
}

// Batch size cap. Each item costs two full tax computations; this bounds the CPU
// a single request can consume (see the cpu_ms limit in wrangler.toml) and is far
// above any plausible portfolio.
const MAX_TAX_ITEMS = 500;

// POST /tax — compute marginal federal tax for a trade (or batch of positions)
// against the authenticated user's stored income profile for the given year.
//   single: { tax_year, st_gain, lt_gain }
//     → { tax, baseline_tax, constants_year }
//   batch:  { tax_year, items: [{ id, st_gain, lt_gain }] }
//     → { results: [{ id, tax }], baseline_tax, constants_year }
// `constants_year` is the year whose bracket tables were used — it differs from
// `tax_year` for future years, whose inflation adjustments aren't published yet.
// Per-item taxes are each marginal against the same baseline, so they do not sum
// to the tax of liquidating everything at once.
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
  // Years before the table would otherwise be computed with the earliest known
  // brackets, which is silently wrong rather than merely imprecise. Future years
  // are allowed and clamp forward (reported via `constants_year`).
  if (taxYear < MIN_TAX_YEAR) {
    return jsonResp(
      { error: `Tax year ${taxYear} is not supported (earliest is ${MIN_TAX_YEAR})` },
      422,
    );
  }

  // Read the user's profile for this year (RLS-scoped via the user's own JWT).
  const profileUrl =
    `${supabaseBase(env)}/rest/v1/tax_profiles?user_id=eq.${user.id}` +
    `&tax_year=eq.${taxYear}&select=payload`;
  const resp = await fetch(profileUrl, {
    headers: { apikey: env.SUPABASE_ANON_KEY, Authorization: authHeader },
  });
  if (!resp.ok) return jsonResp({ error: 'Failed to read tax profile' }, 502);

  const rows = await resp.json<Array<{ payload: { revisions?: unknown[] } }>>();
  const revisions = rows[0]?.payload?.revisions;
  if (!revisions || revisions.length === 0) {
    return jsonResp({ error: `No tax profile for ${taxYear}` }, 422);
  }
  // A malformed stored revision (unknown filing status, non-numeric amount)
  // would otherwise throw on an undefined bracket table or return NaN, which
  // serializes to null and fails to deserialize on the client.
  const baseline: TaxInputs | null = sanitizeTaxInputs(revisions[revisions.length - 1]);
  if (!baseline) {
    return jsonResp({ error: `Tax profile for ${taxYear} is incomplete or invalid` }, 422);
  }

  const constantsYear = constantsFor(taxYear).year;
  const baselineTax = computeFederalTax(baseline, taxYear);

  if (Array.isArray(body.items)) {
    if (body.items.length > MAX_TAX_ITEMS) {
      return jsonResp({ error: `At most ${MAX_TAX_ITEMS} items per request` }, 400);
    }
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
    return jsonResp({ results, baseline_tax: baselineTax, constants_year: constantsYear });
  }

  const stGain = Number(body?.st_gain ?? 0);
  const ltGain = Number(body?.lt_gain ?? 0);
  if (!Number.isFinite(stGain) || !Number.isFinite(ltGain)) {
    return jsonResp({ error: 'st_gain and lt_gain must be numbers' }, 400);
  }

  const tax = marginalTradeTax(baseline, { st_gain: stGain, lt_gain: ltGain }, taxYear, baselineTax);
  return jsonResp({ tax, baseline_tax: baselineTax, constants_year: constantsYear });
}

// ── Entry point ───────────────────────────────────────────────────────────────

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    const origin = request.headers.get('Origin') ?? '';
    const allowed = [env.ALLOWED_ORIGIN, env.DEV_ORIGIN].filter(Boolean) as string[];
    const originOk = allowed.length > 0 && allowed.includes(origin);
    const cors = corsHeaders(origin);

    if (request.method === 'OPTIONS') {
      if (!originOk) return new Response(null, { status: 403 });
      return new Response(null, { headers: cors });
    }

    if (!originOk) {
      return jsonResp({ error: 'Forbidden' }, 403);
    }

    try {
      const user = await verifyToken(request, env);
      if (!user) return withCors(jsonResp({ error: 'Unauthorized' }, 401), cors);
      const authHeader = request.headers.get('Authorization')!;

      const url = new URL(request.url);
      // Under the api.martingale.cc/v1/* route, requests arrive with a /v1
      // prefix; strip it so the path table matches on both that route and the
      // workers.dev domain (where paths are served at the root).
      const path = url.pathname.replace(/^\/v1(?=\/|$)/, '') || '/';

      const response = await (async (): Promise<Response> => {
        if (path === '/quote' && request.method === 'GET') {
          const symbol = url.searchParams.get('symbol')?.toUpperCase();
          if (!symbol) return jsonResp({ error: 'symbol required' }, 400);
          return handleQuote(symbol, env);
        }

        if (path === '/option-quote' && request.method === 'GET') return handleOptionQuote(url, env);
        if (path === '/option-chain' && request.method === 'GET') return handleOptionChain(url, env);
        if (path === '/option-history' && request.method === 'GET') return handleOptionHistory(url, env);
        if (path === '/option-chain-live' && request.method === 'GET') return handleOptionChainLive(url, env);
        if (path === '/option-meta' && request.method === 'GET') return handleOptionMeta(url, env);
        if (path === '/close-prices' && request.method === 'GET') return handleClosePrices(url, env);
        if (path === '/forward-vol' && request.method === 'GET') return handleForwardVol(url, env);
        if (path === '/term-rates' && request.method === 'GET') return handleTermRates(url, env);
        if (path === '/latest-bar' && request.method === 'GET') return handleLatestBar(url, env);
        if (path === '/tax' && request.method === 'POST') return handleTax(request, user, authHeader, env);

        return jsonResp({ error: 'Not found' }, 404);
      })();

      return withCors(response, cors);
    } catch (e) {
      const status = e instanceof UpstreamError ? e.status : 500;
      return withCors(jsonResp({ error: e instanceof Error ? e.message : 'Internal error' }, status), cors);
    }
  },
} satisfies ExportedHandler<Env>;
