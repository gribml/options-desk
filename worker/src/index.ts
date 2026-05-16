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

interface DailyBar {
  date: string; // YYYY-MM-DD
  open: number;
  high: number;
  low: number;
  close: number;
  volume: number;
}

// ── Provider interface ────────────────────────────────────────────────────────
//
// To add a new provider, implement this interface and swap the constructor
// call in each route handler below.

interface MarketDataProvider {
  fetchQuote(symbol: string): Promise<Quote>;
  fetchOptionQuote(
    symbol: string,
    expiry: string,   // YYYY-MM-DD
    type: 'call' | 'put',
    strike: number,
  ): Promise<OptionQuote>;
  fetchDailyBars(symbol: string, fromDate: string, toDate: string): Promise<DailyBar[]>;
}

// ── Polygon.io implementation ─────────────────────────────────────────────────

class PolygonProvider implements MarketDataProvider {
  private readonly base = 'https://api.polygon.io';

  constructor(private readonly apiKey: string) {}

  async fetchQuote(symbol: string): Promise<Quote> {
    const url =
      `${this.base}/v2/snapshot/locale/us/markets/stocks/tickers/${symbol}` +
      `?apiKey=${this.apiKey}`;
    const resp = await fetch(url);
    if (!resp.ok) throw new Error(`Polygon snapshot ${resp.status}`);

    const data = (await resp.json()) as Record<string, unknown>;
    const t = data['ticker'] as Record<string, unknown> | undefined;
    if (!t) throw new Error(`No ticker data for ${symbol}`);

    const lastTrade = t['lastTrade'] as Record<string, number> | undefined;
    const day = t['day'] as Record<string, number> | undefined;
    const prevDay = t['prevDay'] as Record<string, number> | undefined;

    const price = lastTrade?.['p'] ?? day?.['c'] ?? prevDay?.['c'] ?? 0;
    const prevClose = prevDay?.['c'] ?? price;
    const change = price - prevClose;
    const change_pct = prevClose !== 0 ? (change / prevClose) * 100 : 0;

    return { symbol: (t['ticker'] as string) ?? symbol, price, change, change_pct };
  }

  async fetchOptionQuote(
    symbol: string,
    expiry: string,
    type: 'call' | 'put',
    strike: number,
  ): Promise<OptionQuote> {
    const ticker = buildOptionTicker(symbol, expiry, type, strike);
    const url =
      `${this.base}/v3/snapshot/options/${symbol.toUpperCase()}/${ticker}` +
      `?apiKey=${this.apiKey}`;
    const resp = await fetch(url);
    if (!resp.ok) throw new Error(`Polygon option snapshot ${resp.status}`);

    const data = (await resp.json()) as Record<string, unknown>;
    const results = data['results'] as Record<string, unknown> | undefined;
    if (!results) throw new Error(`No option data for ${ticker}`);

    const lastQuote = results['last_quote'] as Record<string, number> | undefined;
    const lastTrade = results['last_trade'] as Record<string, number> | undefined;
    const day = results['day'] as Record<string, number> | undefined;

    let price = 0;
    if (lastQuote?.['bid'] != null && lastQuote?.['ask'] != null) {
      price = (lastQuote['bid'] + lastQuote['ask']) / 2;
    } else if (lastTrade?.['price'] != null) {
      price = lastTrade['price'];
    } else if (day?.['c'] != null) {
      price = day['c'];
    }

    const implied_vol = (results['implied_volatility'] as number | undefined) ?? null;

    return { price, implied_vol };
  }

  async fetchDailyBars(symbol: string, fromDate: string, toDate: string): Promise<DailyBar[]> {
    const all: DailyBar[] = [];
    // limit=50000 is well above 2yr (~504 bars); handle pagination via next_url.
    let url: string | null =
      `${this.base}/v2/aggs/ticker/${symbol}/range/1/day/${fromDate}/${toDate}` +
      `?adjusted=true&sort=asc&limit=50000&apiKey=${this.apiKey}`;

    while (url) {
      const resp = await fetch(url);
      if (!resp.ok) throw new Error(`Polygon aggs ${resp.status}`);
      const data = (await resp.json()) as Record<string, unknown>;

      const results = data['results'];
      if (Array.isArray(results)) {
        for (const r of results as Record<string, number>[]) {
          all.push({
            date: new Date(r['t']).toISOString().slice(0, 10),
            open: r['o'],
            high: r['h'],
            low: r['l'],
            close: r['c'],
            volume: r['v'],
          });
        }
      }

      const nextUrl = data['next_url'];
      url = typeof nextUrl === 'string' ? `${nextUrl}&apiKey=${this.apiKey}` : null;
    }

    return all;
  }
}

// Polygon option ticker format: O:{SYMBOL}{YYMMDD}{C|P}{strike×1000 padded to 8 digits}
// Example: AAPL $150 call exp 2025-01-17 → O:AAPL250117C00150000
function buildOptionTicker(symbol: string, expiry: string, type: 'call' | 'put', strike: number): string {
  const [year, month, day] = expiry.split('-');
  const yy = year.slice(2);
  const typeChar = type === 'call' ? 'C' : 'P';
  const strikeStr = Math.round(strike * 1000).toString().padStart(8, '0');
  return `O:${symbol.toUpperCase()}${yy}${month}${day}${typeChar}${strikeStr}`;
}

// ── Worker env ────────────────────────────────────────────────────────────────

interface Env {
  POLYGON_KEY: string;
  SUPABASE_URL: string;
  SUPABASE_SERVICE_KEY: string;
  SUPABASE_ANON_KEY: string;
}

// ── Helpers ───────────────────────────────────────────────────────────────────

const CORS: Record<string, string> = {
  'Access-Control-Allow-Origin': '*',
  'Access-Control-Allow-Methods': 'GET, POST, OPTIONS',
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

async function cachedJson(
  key: string,
  ttlSeconds: number,
  produce: () => Promise<unknown>,
): Promise<Response> {
  const cache = caches.default;
  const cacheReq = new Request(`https://cache.internal/${key}`);

  const cached = await cache.match(cacheReq);
  if (cached) {
    const r = new Response(cached.body, cached);
    for (const [k, v] of Object.entries(CORS)) r.headers.set(k, v);
    return r;
  }

  const data = await produce();
  const body = JSON.stringify(data);

  await cache.put(
    cacheReq,
    new Response(body, {
      headers: {
        'Content-Type': 'application/json',
        'Cache-Control': `public, max-age=${ttlSeconds}`,
      },
    }),
  );

  return new Response(body, {
    headers: { ...CORS, 'Content-Type': 'application/json' },
  });
}

// ── Route: GET /quote?symbol=AAPL ─────────────────────────────────────────────

async function handleQuote(symbol: string, env: Env): Promise<Response> {
  const provider = new PolygonProvider(env.POLYGON_KEY);
  return cachedJson(`quote/${symbol}`, 300, () => provider.fetchQuote(symbol));
}

// ── Route: GET /option-quote?symbol=AAPL&expiry=2025-01-17&type=call&strike=150

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

  const cacheKey = `option-quote/${buildOptionTicker(symbol, expiry, type, strike)}`;
  const provider = new PolygonProvider(env.POLYGON_KEY);
  return cachedJson(cacheKey, 300, () =>
    provider.fetchOptionQuote(symbol, expiry, type, strike),
  );
}

// ── Route: POST /history ──────────────────────────────────────────────────────
// Body: { symbols: string[] }
// Fetches 2 years of daily bars per symbol and upserts into price_history.

async function handleHistory(request: Request, env: Env): Promise<Response> {
  const body = (await request.json()) as { symbols?: unknown };
  if (!Array.isArray(body.symbols) || body.symbols.length === 0) {
    return jsonResp({ error: 'symbols array required' }, 400);
  }
  const symbols: string[] = (body.symbols as unknown[]).map((s) => String(s).toUpperCase());

  const today = new Date().toISOString().slice(0, 10);
  const twoYearsAgo = new Date(Date.now() - 2 * 365.25 * 24 * 60 * 60 * 1000)
    .toISOString()
    .slice(0, 10);

  const provider = new PolygonProvider(env.POLYGON_KEY);
  const results: Record<string, string> = {};

  for (const symbol of symbols) {
    try {
      const bars = await provider.fetchDailyBars(symbol, twoYearsAgo, today);
      if (bars.length === 0) {
        results[symbol] = 'no data';
        continue;
      }

      const rows = bars.map((b) => ({ symbol, ...b }));
      const sb = await fetch(`${supabaseBase(env)}/rest/v1/price_history`, {
        method: 'POST',
        headers: {
          apikey: env.SUPABASE_SERVICE_KEY,
          Authorization: `Bearer ${env.SUPABASE_SERVICE_KEY}`,
          'Content-Type': 'application/json',
          Prefer: 'resolution=merge-duplicates',
        },
        body: JSON.stringify(rows),
      });

      results[symbol] = sb.ok ? `ok (${bars.length} bars)` : `db ${sb.status}`;
    } catch (e) {
      results[symbol] = `error: ${e instanceof Error ? e.message : String(e)}`;
    }
  }

  return jsonResp({ results });
}

// ── Entry point ───────────────────────────────────────────────────────────────

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    if (request.method === 'OPTIONS') return new Response(null, { headers: CORS });

    if (!(await verifyToken(request, env))) return jsonResp({ error: 'Unauthorized' }, 401);

    const url = new URL(request.url);

    if (url.pathname === '/quote' && request.method === 'GET') {
      const symbol = url.searchParams.get('symbol')?.toUpperCase();
      if (!symbol) return jsonResp({ error: 'symbol required' }, 400);
      return handleQuote(symbol, env);
    }

    if (url.pathname === '/option-quote' && request.method === 'GET') {
      return handleOptionQuote(url, env);
    }

    if (url.pathname === '/history' && request.method === 'POST') {
      return handleHistory(request, env);
    }

    return jsonResp({ error: 'Not found' }, 404);
  },
} satisfies ExportedHandler<Env>;
