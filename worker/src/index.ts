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
}

// ── Helpers ───────────────────────────────────────────────────────────────────

const CORS: Record<string, string> = {
  'Access-Control-Allow-Origin': '*',
  'Access-Control-Allow-Methods': 'GET, OPTIONS',
  'Access-Control-Allow-Headers': 'Content-Type, Authorization',
};

function jsonResp(data: unknown, status = 200): Response {
  return new Response(JSON.stringify(data), {
    status,
    headers: { ...CORS, 'Content-Type': 'application/json' },
  });
}

function supabaseBase(env: Env): string {
  return env.SUPABASE_URL.replace(/\/rest\/v1\/?$/, '').replace(/\/$/, '');
}

async function verifyToken(request: Request, env: Env): Promise<boolean> {
  const auth = request.headers.get('Authorization');
  if (!auth?.startsWith('Bearer ')) return false;
  const resp = await fetch(`${supabaseBase(env)}/auth/v1/user`, {
    headers: { apikey: env.SUPABASE_ANON_KEY, Authorization: auth },
  });
  return resp.ok;
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

  const alpacaResp = await fetch(
    `https://data.alpaca.markets/v2/stocks/${encodeURIComponent(symbol)}/bars/latest`,
    { headers: { 'APCA-API-KEY-ID': env.ALPACA_KEY, 'APCA-API-SECRET-KEY': env.ALPACA_SECRET } },
  );

  if (!alpacaResp.ok) {
    const text = await alpacaResp.text();
    return jsonResp({ error: `Alpaca error: ${text}` }, alpacaResp.status);
  }

  const data = await alpacaResp.json<{ bars: Record<string, { o: number; h: number; l: number; c: number; v: number; vw: number; n: number; t: string }> }>();
  const bar = data.bars?.[symbol];
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

// ── Entry point ───────────────────────────────────────────────────────────────

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    try {
      if (request.method === 'OPTIONS') return new Response(null, { headers: CORS });

      if (!(await verifyToken(request, env))) return jsonResp({ error: 'Unauthorized' }, 401);

      const url = new URL(request.url);

      if (url.pathname === '/quote' && request.method === 'GET') {
        const symbol = url.searchParams.get('symbol')?.toUpperCase();
        if (!symbol) return jsonResp({ error: 'symbol required' }, 400);
        return await handleQuote(symbol, env);
      }

      if (url.pathname === '/option-quote' && request.method === 'GET') {
        return await handleOptionQuote(url, env);
      }

      if (url.pathname === '/option-chain' && request.method === 'GET') {
        return await handleOptionChain(url, env);
      }

      if (url.pathname === '/option-meta' && request.method === 'GET') {
        return await handleOptionMeta(url, env);
      }

      if (url.pathname === '/close-prices' && request.method === 'GET') {
        return await handleClosePrices(url, env);
      }

      if (url.pathname === '/forward-vol' && request.method === 'GET') {
        return await handleForwardVol(url, env);
      }

      if (url.pathname === '/latest-bar' && request.method === 'GET') {
        return await handleLatestBar(url, env);
      }

      return jsonResp({ error: 'Not found' }, 404);
    } catch (e) {
      return jsonResp({ error: e instanceof Error ? e.message : 'Internal error' }, 500);
    }
  },
} satisfies ExportedHandler<Env>;
