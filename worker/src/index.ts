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

interface OptionChainEntry {
  symbol: string;       // OCC symbol e.g. AAPL250117C00150000
  underlying: string;
  expiry: string;       // YYYY-MM-DD
  type: 'call' | 'put';
  strike: number;
  bid: number;
  ask: number;
  mid: number;
  last: number | null;
  implied_vol: number | null;
  delta: number | null;
  gamma: number | null;
  theta: number | null;
  vega: number | null;
  open_interest: number | null;
  volume: number | null;
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
  fetchOptionChain(symbol: string): Promise<OptionChainEntry[]>;
}

// ── Alpaca implementation ─────────────────────────────────────────────────────

class AlpacaProvider implements MarketDataProvider {
  private readonly base = 'https://data.alpaca.markets';

  constructor(
    private readonly apiKey: string,
    private readonly apiSecret: string,
  ) {}

  private headers(): Record<string, string> {
    return {
      'APCA-API-KEY-ID': this.apiKey,
      'APCA-API-SECRET-KEY': this.apiSecret,
    };
  }

  async fetchQuote(symbol: string): Promise<Quote> {
    const url = `${this.base}/v2/stocks/${symbol.toUpperCase()}/snapshot?feed=iex`;
    const resp = await fetch(url, { headers: this.headers() });
    if (!resp.ok) throw new Error(`Alpaca snapshot ${resp.status}`);

    const data = (await resp.json()) as Record<string, unknown>;
    const latestTrade = data['latestTrade'] as Record<string, number> | undefined;
    const latestQuote = data['latestQuote'] as Record<string, number> | undefined;
    const dailyBar = data['dailyBar'] as Record<string, number> | undefined;
    const prevDailyBar = data['prevDailyBar'] as Record<string, number> | undefined;

    const price = latestTrade?.['p'] ?? latestQuote?.['ap'] ?? dailyBar?.['c'] ?? 0;
    const prevClose = prevDailyBar?.['c'] ?? price;
    const change = price - prevClose;
    const change_pct = prevClose !== 0 ? (change / prevClose) * 100 : 0;

    return { symbol: symbol.toUpperCase(), price, change, change_pct };
  }

  async fetchOptionQuote(
    symbol: string,
    expiry: string,
    type: 'call' | 'put',
    strike: number,
  ): Promise<OptionQuote> {
    const ticker = buildOptionTicker(symbol, expiry, type, strike);
    const url =
      `${this.base}/v1beta1/options/snapshots/${symbol.toUpperCase()}` +
      `?symbols=${ticker}&feed=indicative`;
    const resp = await fetch(url, { headers: this.headers() });
    if (!resp.ok) throw new Error(`Alpaca option snapshot ${resp.status}`);

    const data = (await resp.json()) as Record<string, unknown>;
    const snapshots = data['snapshots'] as Record<string, unknown> | undefined;
    const snap = snapshots?.[ticker] as Record<string, unknown> | undefined;
    if (!snap) throw new Error(`No option data for ${ticker}`);

    const latestQuote = snap['latestQuote'] as Record<string, number> | undefined;
    const latestTrade = snap['latestTrade'] as Record<string, number> | undefined;
    const dailyBar = snap['dailyBar'] as Record<string, number> | undefined;

    let price = 0;
    if (latestQuote?.['bp'] != null && latestQuote?.['ap'] != null) {
      price = (latestQuote['bp'] + latestQuote['ap']) / 2;
    } else if (latestTrade?.['p'] != null) {
      price = latestTrade['p'];
    } else if (dailyBar?.['c'] != null) {
      price = dailyBar['c'];
    }

    const implied_vol = (snap['impliedVolatility'] as number | undefined) ?? null;
    return { price, implied_vol };
  }

  async fetchDailyBars(symbol: string, fromDate: string, toDate: string): Promise<DailyBar[]> {
    const all: DailyBar[] = [];
    let url: string | null =
      `${this.base}/v2/stocks/${symbol.toUpperCase()}/bars` +
      `?timeframe=1Day&start=${fromDate}&end=${toDate}&limit=10000&feed=iex&sort=asc`;

    while (url) {
      const resp = await fetch(url, { headers: this.headers() });
      if (!resp.ok) throw new Error(`Alpaca bars ${resp.status}`);
      const data = (await resp.json()) as Record<string, unknown>;

      const bars = data['bars'];
      if (Array.isArray(bars)) {
        for (const b of bars as Record<string, unknown>[]) {
          all.push({
            date: (b['t'] as string).slice(0, 10),
            open: b['o'] as number,
            high: b['h'] as number,
            low: b['l'] as number,
            close: b['c'] as number,
            volume: b['v'] as number,
          });
        }
      }

      const nextToken = data['next_page_token'];
      if (typeof nextToken === 'string' && nextToken) {
        const next = new URL(url);
        next.searchParams.set('page_token', nextToken);
        url = next.toString();
      } else {
        url = null;
      }
    }

    return all;
  }

  async fetchOptionChain(symbol: string): Promise<OptionChainEntry[]> {
    const all: OptionChainEntry[] = [];
    let url: string | null =
      `${this.base}/v1beta1/options/snapshots/${symbol.toUpperCase()}` +
      `?feed=indicative&limit=1000`;

    while (url) {
      const resp = await fetch(url, { headers: this.headers() });
      if (!resp.ok) throw new Error(`Alpaca option chain ${resp.status}`);
      const data = (await resp.json()) as Record<string, unknown>;

      const snapshots = data['snapshots'] as Record<string, unknown> | undefined;
      if (snapshots) {
        for (const [occSymbol, snap] of Object.entries(snapshots)) {
          const s = snap as Record<string, unknown>;
          const details = s['details'] as Record<string, unknown> | undefined;
          const latestQuote = s['latestQuote'] as Record<string, number> | undefined;
          const latestTrade = s['latestTrade'] as Record<string, number> | undefined;
          const greeks = s['greeks'] as Record<string, number> | undefined;
          const dailyBar = s['dailyBar'] as Record<string, number> | undefined;

          const bid = latestQuote?.['bp'] ?? 0;
          const ask = latestQuote?.['ap'] ?? 0;

          all.push({
            symbol: occSymbol,
            underlying: symbol.toUpperCase(),
            expiry: (details?.['expirationDate'] as string) ?? '',
            type: (details?.['contractType'] as string) === 'put' ? 'put' : 'call',
            strike: parseFloat((details?.['strikePrice'] as string) ?? '0'),
            bid,
            ask,
            mid: (bid + ask) / 2,
            last: latestTrade?.['p'] ?? null,
            implied_vol: (s['impliedVolatility'] as number | undefined) ?? null,
            delta: greeks?.['delta'] ?? null,
            gamma: greeks?.['gamma'] ?? null,
            theta: greeks?.['theta'] ?? null,
            vega: greeks?.['vega'] ?? null,
            open_interest: (s['openInterest'] as number | undefined) ?? null,
            volume: dailyBar?.['v'] ?? null,
          });
        }
      }

      const nextToken = data['next_page_token'];
      if (typeof nextToken === 'string' && nextToken) {
        const next = new URL(url);
        next.searchParams.set('page_token', nextToken);
        url = next.toString();
      } else {
        url = null;
      }
    }

    return all;
  }
}

// OCC option ticker: O:{SYMBOL}{YYMMDD}{C|P}{strike×1000 padded to 8 digits}
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
  ALPACA_KEY: string;
  ALPACA_SECRET: string;
  SUPABASE_URL: string;
  SUPABASE_SERVICE_KEY: string;
  SUPABASE_ANON_KEY: string;
  DB: D1Database;
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

// ── Route handlers ────────────────────────────────────────────────────────────

async function handleQuote(symbol: string, env: Env): Promise<Response> {
  const provider = new AlpacaProvider(env.ALPACA_KEY, env.ALPACA_SECRET);
  return cachedJson(`quote/${symbol}`, 300, () => provider.fetchQuote(symbol));
}

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
  const provider = new AlpacaProvider(env.ALPACA_KEY, env.ALPACA_SECRET);
  return cachedJson(cacheKey, 300, () =>
    provider.fetchOptionQuote(symbol, expiry, type, strike),
  );
}

// GET /option-chain?symbol=AAPL
// Returns the full option chain snapshot. Cached 5 min.
async function handleOptionChain(url: URL, env: Env): Promise<Response> {
  const symbol = url.searchParams.get('symbol')?.toUpperCase();
  if (!symbol) return jsonResp({ error: 'symbol required' }, 400);

  const provider = new AlpacaProvider(env.ALPACA_KEY, env.ALPACA_SECRET);
  return cachedJson(`option-chain/${symbol}`, 300, () =>
    provider.fetchOptionChain(symbol),
  );
}

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

  const provider = new AlpacaProvider(env.ALPACA_KEY, env.ALPACA_SECRET);
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

    if (url.pathname === '/option-chain' && request.method === 'GET') {
      return handleOptionChain(url, env);
    }

    if (url.pathname === '/history' && request.method === 'POST') {
      return handleHistory(request, env);
    }

    return jsonResp({ error: 'Not found' }, 404);
  },
} satisfies ExportedHandler<Env>;
