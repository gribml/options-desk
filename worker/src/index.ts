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
    SELECT date(ts) AS day, close
    FROM bars_1min
    WHERE symbol = ?
      AND ts IN (SELECT MAX(ts) FROM bars_1min WHERE symbol = ? GROUP BY date(ts))
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
    SELECT (bid + ask) / 2.0 AS mid, implied_vol
    FROM option_chain
    WHERE underlying = ?
      AND expiry = ?
      AND option_type = ?
      AND strike = ?
    ORDER BY snapshot_time DESC
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
    SELECT symbol, underlying, expiry, option_type AS type, strike,
           bid, ask, (bid + ask) / 2.0 AS mid, implied_vol, delta, gamma, theta, vega,
           open_interest, volume
    FROM option_chain
    WHERE underlying = ?
      AND snapshot_time = (SELECT MAX(snapshot_time) FROM option_chain WHERE underlying = ?)
    ORDER BY expiry, option_type, strike
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
    SELECT DISTINCT expiry, option_type, strike
    FROM option_chain
    WHERE underlying = ?
      AND snapshot_time = (SELECT MAX(snapshot_time) FROM option_chain WHERE underlying = ?)
    ORDER BY expiry, option_type, strike
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
      AND ts IN (SELECT MAX(ts) FROM bars_1min WHERE symbol = ? GROUP BY date(ts))
    ORDER BY date(ts) DESC
    LIMIT ?
  `).bind(symbol, symbol, limit).all<{ close: number }>();

  return jsonResp(results.map(r => r.close));
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

      return jsonResp({ error: 'Not found' }, 404);
    } catch (e) {
      return jsonResp({ error: e instanceof Error ? e.message : 'Internal error' }, 500);
    }
  },
} satisfies ExportedHandler<Env>;
