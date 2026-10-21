import {
  computeFederalTax,
  marginalTradeTax,
  sanitizeTaxInputs,
  withTradeGains,
  constantsFor,
  MIN_TAX_YEAR,
  type TaxInputs,
} from './tax';
import { verifyToken } from './auth';

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
  // All browser requests carry credentials. Shared market caching happens only
  // inside the Worker after authentication, never in a downstream HTTP cache.
  headers.set('Cache-Control', 'private, no-store');
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

// ── Route handlers ────────────────────────────────────────────────────────────

// GET /quote?symbol=AAPL
// Returns the latest price and day-over-day change from bars_1min.
async function handleQuote(symbol: string, env: Env): Promise<Response> {
  const latest = await env.DB.prepare(`
    SELECT timestamp, close FROM bars_1min WHERE symbol = ?
    ORDER BY timestamp DESC LIMIT 1
  `).bind(symbol).first<{ timestamp: string; close: number }>();
  if (!latest) return jsonResp({ error: `No data for ${symbol}` }, 404);
  const day = latest.timestamp.slice(0, 10);
  const previous = await env.DB.prepare(`
    SELECT close FROM bars_1min WHERE symbol = ? AND timestamp < ?
    ORDER BY timestamp DESC LIMIT 1
  `).bind(symbol, day).first<{ close: number }>();
  const price = latest.close;
  const prevClose = previous?.close ?? price;
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

  const limit = Math.max(1, Math.min(parseInt(url.searchParams.get('limit') ?? '31') || 31, 500));

  const latest = await env.DB.prepare(`
    SELECT timestamp, close FROM bars_1min WHERE symbol = ? ORDER BY timestamp DESC LIMIT 1
  `).bind(symbol).first<{ timestamp: string; close: number }>();
  if (!latest) return jsonResp([]);
  const day = latest.timestamp.slice(0, 10);
  const initialized = await env.DB.prepare(`
    SELECT 1 AS ok FROM daily_closes_state WHERE symbol = ?
  `).bind(symbol).first<{ ok: number }>();
  if (!initialized) {
    // Pay the historical grouping cost once per symbol, then use indexed daily
    // rows on every subsequent request. The cron keeps recent rows current.
    await env.DB.prepare(`
      INSERT INTO daily_closes (symbol, day, close)
      SELECT ?, date(timestamp), close FROM bars_1min
      WHERE symbol = ? AND timestamp IN (
        SELECT MAX(timestamp) FROM bars_1min WHERE symbol = ? GROUP BY date(timestamp)
      )
      ORDER BY timestamp DESC LIMIT 500
      ON CONFLICT(symbol, day) DO UPDATE SET close = excluded.close
    `).bind(symbol, symbol, symbol).run();
    await env.DB.prepare(`
      INSERT OR REPLACE INTO daily_closes_state (symbol, backfilled_at)
      VALUES (?, datetime('now'))
    `).bind(symbol).run();
  }
  if (limit === 1) return jsonResp([latest.close]);
  const previous = await env.DB.prepare(`
    SELECT timestamp, close FROM bars_1min WHERE symbol = ? AND timestamp < ?
    ORDER BY timestamp DESC LIMIT 1
  `).bind(symbol, day).first<{ timestamp: string; close: number }>();
  if (!previous) return jsonResp([latest.close]);
  const { results } = await env.DB.prepare(`
    SELECT close FROM daily_closes WHERE symbol = ? AND day < ?
    ORDER BY day DESC LIMIT ?
  `).bind(symbol, previous.timestamp.slice(0, 10), limit - 2).all<{ close: number }>();
  return jsonResp([latest.close, previous.close, ...results.map(r => r.close)]);
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

// One auth check and one browser request for a portfolio's stock marks. Keep
// upstream concurrency bounded so many simultaneous portfolios do not flood
// Alpaca when their 15-minute cache expires together.
async function handleLatestBars(url: URL, env: Env): Promise<Response> {
  const symbols = [...new Set((url.searchParams.get('symbols') ?? '').split(',').map(s => s.trim().toUpperCase()))]
    .filter(Boolean);
  if (!symbols.length || symbols.length > 40 || symbols.some(s => !/^[A-Z0-9.]{1,32}$/.test(s))) {
    return jsonResp({ error: 'Provide 1 to 40 valid symbols' }, 400);
  }
  const results: Array<unknown> = new Array(symbols.length);
  let next = 0;
  await Promise.all(Array.from({ length: Math.min(6, symbols.length) }, async () => {
    while (next < symbols.length) {
      const index = next++;
      try {
        const response = await handleLatestBar(new URL(`https://local/latest-bar?symbol=${encodeURIComponent(symbols[index])}`), env);
        if (response.ok) results[index] = await response.json();
      } catch (error) {
        console.error('Latest bar failed:', symbols[index], error);
      }
    }
  }));
  return jsonResp(results.filter(Boolean));
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
  if (!underlying || !/^[A-Z0-9.]{1,32}$/.test(underlying)) return jsonResp({ error: 'valid symbol required' }, 400);
  const pageToken = url.searchParams.get('page_token');
  const requestedRefresh = url.searchParams.get('refresh_id');
  let refreshId: string;

  // Keep completed cache reads paged as well. Large underlyings can have far
  // more contracts than fit comfortably in one D1 result or HTTP response.
  const cachedPage = async (offset: number): Promise<Response> => {
    const { results } = await env.DB.prepare(`
      SELECT symbol, underlying, expiration AS expiry, option_type AS type, strike,
             bid, ask, (bid + ask) / 2.0 AS mid, last_price AS last,
             implied_volatility AS implied_vol, delta, gamma, theta, vega,
             NULL AS open_interest, NULL AS volume
      FROM option_chain_cache
      WHERE underlying = ?
      ORDER BY expiration, option_type, strike
      LIMIT 1001 OFFSET ?
    `).bind(underlying, offset).all<OptionChainEntry>();
    const more = results.length > 1000;
    return jsonResp({
      entries: results.slice(0, 1000),
      next_page_token: more ? `cache:${offset + 1000}` : null,
      cached: true,
      refresh_id: null,
    });
  };
  if (pageToken?.startsWith('cache:')) {
    const offset = Number(pageToken.slice(6));
    if (!Number.isSafeInteger(offset) || offset < 1000 || offset % 1000 !== 0) {
      return jsonResp({ error: 'Invalid cache page token' }, 400);
    }
    const state = await env.DB.prepare(`
      SELECT status FROM option_chain_refresh WHERE underlying = ?
    `).bind(underlying).first<{ status: string }>();
    if (state?.status !== 'ready') return jsonResp({ error: 'Cached chain changed; restart' }, 409);
    return cachedPage(offset);
  }

  // Only a completed generation can be served as a cached chain. An atomic D1
  // lease allows just one request to start a refresh for each underlying.
  if (!pageToken) {
    const state = await env.DB.prepare(`
      SELECT status, fetched_at FROM option_chain_refresh WHERE underlying = ?
    `).bind(underlying).first<{ status: string; fetched_at: string | null }>();
    if (state?.status === 'ready' && state.fetched_at && Date.now() - Date.parse(`${state.fetched_at}Z`) < 15 * 60_000) {
      return cachedPage(0);
    }
    refreshId = crypto.randomUUID();
    const claim = await env.DB.prepare(`
      INSERT INTO option_chain_refresh (underlying, refresh_id, status, lease_until)
      VALUES (?, ?, 'loading', datetime('now', '+2 minutes'))
      ON CONFLICT(underlying) DO UPDATE SET
        refresh_id = excluded.refresh_id, status = 'loading',
        fetched_at = NULL, lease_until = excluded.lease_until
      WHERE (option_chain_refresh.status = 'ready'
             AND (option_chain_refresh.fetched_at IS NULL
                  OR option_chain_refresh.fetched_at < datetime('now', '-15 minutes')))
         OR (option_chain_refresh.status = 'loading'
             AND option_chain_refresh.lease_until < datetime('now'))
    `).bind(underlying, refreshId).run();
    if (!claim.meta.changes) return jsonResp({ error: 'Chain refresh in progress' }, 503);
    await env.DB.prepare(`
      DELETE FROM option_chain_cache WHERE underlying = ?
    `).bind(underlying).run();
  } else {
    // Older browser bundles do not send refresh_id. Let them finish the active
    // chain while the new Worker and frontend versions roll out independently.
    const active = requestedRefresh ? null : await env.DB.prepare(`
      SELECT refresh_id FROM option_chain_refresh
      WHERE underlying = ? AND status = 'loading'
    `).bind(underlying).first<{ refresh_id: string }>();
    refreshId = requestedRefresh ?? active?.refresh_id ?? '';
    if (!refreshId) return jsonResp({ error: 'No active chain refresh' }, 409);
    const lease = await env.DB.prepare(`
      UPDATE option_chain_refresh SET lease_until = datetime('now', '+2 minutes')
      WHERE underlying = ? AND refresh_id = ? AND status = 'loading'
        AND lease_until >= datetime('now')
    `).bind(underlying, refreshId).run();
    if (!lease.meta.changes) return jsonResp({ error: 'Chain refresh expired; restart' }, 409);
  }

  // Fetch one Alpaca page.
  let endpoint = `v1beta1/options/snapshots/${encodeURIComponent(underlying)}?feed=indicative&limit=1000`;
  if (pageToken) endpoint += `&page_token=${encodeURIComponent(pageToken)}`;

  const abandon = async () => {
    await env.DB.prepare(`
      UPDATE option_chain_refresh SET lease_until = datetime('now', '-1 second')
      WHERE underlying = ? AND refresh_id = ? AND status = 'loading'
    `).bind(underlying, refreshId).run();
  };
  try {
    const alpacaResp = await fetchAlpaca(endpoint, env);
    if (!alpacaResp.ok) {
      console.error(`Market data upstream error (${alpacaResp.status}):`, await alpacaResp.text());
      await abandon();
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

    if (!data.next_page_token) {
      await env.DB.prepare(`
        UPDATE option_chain_refresh
        SET status = 'ready', fetched_at = datetime('now'), lease_until = NULL
        WHERE underlying = ? AND refresh_id = ? AND status = 'loading'
      `).bind(underlying, refreshId).run();
    }

    entries.sort((a, b) =>
      a.expiry.localeCompare(b.expiry) || a.type.localeCompare(b.type) || a.strike - b.strike);

    return jsonResp({ entries, next_page_token: data.next_page_token ?? null, cached: false, refresh_id: refreshId });
  } catch (error) {
    await abandon();
    throw error;
  }
}

// Batch size cap. Each item costs two full tax computations; this bounds the CPU
// a single request can consume (see the cpu_ms limit in wrangler.toml) and is far
// above any plausible portfolio.
const MAX_TAX_ITEMS = 500;
const TAX_BASELINE_TTL_MS = 5_000;
const taxBaselines = new Map<string, { expires: number; baseline: TaxInputs }>();

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
  user: { id: string },
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

  const cacheKey = `${user.id}:${taxYear}`;
  const cachedBaseline = taxBaselines.get(cacheKey);
  let baseline = cachedBaseline && cachedBaseline.expires > Date.now()
    ? cachedBaseline.baseline : undefined;
  if (!baseline) {
    // RLS-scoped read. Short per-isolate reuse absorbs bursts of mark changes;
    // profile edits become visible after at most five seconds.
    const profileUrl =
      `${supabaseBase(env)}/rest/v1/tax_profiles?user_id=eq.${user.id}` +
      `&tax_year=eq.${taxYear}&select=payload`;
    const resp = await fetch(profileUrl, {
      headers: { apikey: env.SUPABASE_ANON_KEY, Authorization: authHeader },
    });
    if (!resp.ok) return jsonResp({ error: 'Failed to read tax profile' }, 502);
    const rows = await resp.json<Array<{ payload: { revisions?: unknown[]; trade_gains?: unknown } }>>();
    const revisions = rows[0]?.payload?.revisions;
    if (!revisions || revisions.length === 0) {
      return jsonResp({ error: `No tax profile for ${taxYear}` }, 422);
    }
    const typed = sanitizeTaxInputs(revisions[revisions.length - 1]);
    if (!typed) {
      return jsonResp({ error: `Tax profile for ${taxYear} is incomplete or invalid` }, 422);
    }
    baseline = withTradeGains(typed, rows[0]?.payload?.trade_gains);
    if (taxBaselines.size >= 100) taxBaselines.delete(taxBaselines.keys().next().value!);
    taxBaselines.set(cacheKey, { expires: Date.now() + TAX_BASELINE_TTL_MS, baseline });
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
  async scheduled(_controller: ScheduledController, env: Env): Promise<void> {
    // One rolling daily aggregation replaces a scan of minute history on each
    // request. The pipeline can also write this table as new bars arrive.
    await env.DB.prepare(`
      INSERT INTO daily_closes (symbol, day, close)
      SELECT symbol, day, close FROM (
        SELECT symbol, date(timestamp) AS day, close,
               ROW_NUMBER() OVER (PARTITION BY symbol, date(timestamp) ORDER BY timestamp DESC) AS rank
        FROM bars_1min WHERE timestamp >= datetime('now', '-7 days')
      ) WHERE rank = 1
      ON CONFLICT(symbol, day) DO UPDATE SET close = excluded.close
    `).run();
  },
  async fetch(request: Request, env: Env, ctx: ExecutionContext): Promise<Response> {
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

      // These routes contain market data shared by every user. Cache only
      // after authentication, with a token-free key and route-specific TTL.
      const marketTtl: Record<string, number> = {
        '/quote': 60, '/close-prices': 300, '/latest-bar': 60,
        '/option-quote': 60,
        '/option-chain': 900, '/option-meta': 900, '/option-history': 300,
        '/forward-vol': 300, '/term-rates': 300,
      };
      const ttl = request.method === 'GET' ? marketTtl[path] : undefined;
      let cacheKey: Request | undefined;
      if (ttl) {
        const cacheUrl = new URL(request.url);
        cacheUrl.pathname = path;
        for (const key of ['symbol', 'underlying']) {
          const value = cacheUrl.searchParams.get(key);
          if (value) cacheUrl.searchParams.set(key, value.toUpperCase());
        }
        cacheUrl.searchParams.sort();
        cacheKey = new Request(cacheUrl.toString());
        const hit = await caches.default.match(cacheKey);
        if (hit) return withCors(hit, cors);
      }

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
        if (path === '/latest-bars' && request.method === 'GET') return handleLatestBars(url, env);
        if (path === '/tax' && request.method === 'POST') return handleTax(request, user, authHeader, env);
        return jsonResp({ error: 'Not found' }, 404);
      })();

      if (ttl && cacheKey && response.ok) {
        const cached = new Response(response.clone().body, {
          status: response.status,
          headers: { ...Object.fromEntries(response.headers), 'Cache-Control': `public, max-age=${ttl}` },
        });
        ctx.waitUntil(caches.default.put(cacheKey, cached));
      }

      return withCors(response, cors);
    } catch (e) {
      const status = e instanceof UpstreamError ? e.status : 500;
      return withCors(jsonResp({ error: e instanceof Error ? e.message : 'Internal error' }, status), cors);
    }
  },
} satisfies ExportedHandler<Env>;
