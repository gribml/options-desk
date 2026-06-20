# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

```bash
# Frontend (Rust/WASM) — app is mounted under /app (see Trunk.toml public_url)
trunk serve                                      # Dev server; open http://localhost:8090/app/
trunk build --release                            # Production WASM build → dist/app/
cargo check --target wasm32-unknown-unknown      # Fast compile check (use this, not cargo check)

# Worker (TypeScript)
cd worker && npm run dev                         # Local worker on localhost:8787
cd worker && npm run deploy                      # Deploy to Cloudflare Workers

# Utility scripts
./scripts/option-chain.sh AAPL                  # Curl Alpaca option chain
./scripts/bars-1min.sh AAPL 2024-01-01 2024-01-02 out.json  # Curl Alpaca 1-min bars
```

There are no tests.

## Architecture

This is a Leptos 0.8 CSR (client-side rendered) WASM app. All pricing math and UI logic runs in the browser. A Cloudflare Worker acts as a thin API gateway — it validates Supabase JWTs and reads from a Cloudflare D1 database. User data (positions, scenarios) is persisted directly to Supabase from the frontend.

**Routing split:** the WASM app is mounted under `/app` (`leptos_router` `base="/app"`, set from `config::APP_BASE`, matching `public_url` in `Trunk.toml`). The site root `/` serves a static, non-WASM landing page (`landing/index.html`) so logged-out visitors don't download the WASM bundle. trunk builds the SPA into `dist/app/`; the deploy workflow copies `landing/index.html` → `dist/index.html` and `landing/_redirects` → `dist/_redirects`. `_redirects` rewrites `/app/*` → `/app/index.html` for client-side-router deep links. Router-driven links (`<A>`, `<Redirect>`) are base-aware automatically; imperative `web_sys` `set_href` navigations must prepend `APP_BASE`.

```
Browser (WASM/Leptos)
  ├── Auth: Supabase JWT via /auth/v1 (stored in LocalStorage, refreshed every 45min)
  ├── User data: Supabase REST /rest/v1/positions, /rest/v1/scenarios
  └── Market data: Cloudflare Worker
        └── D1 database (bars_1min, option_chain tables)
              ↑ populated by a separate data pipeline (not in this repo)
```

### Environment Variables

Injected at compile time via `build.rs`, which reads `.env` and emits `cargo:rustc-env` directives. Accessed in Rust via `option_env!()` in `src/config.rs`. Required keys:

- `SUPABASE_URL` — e.g. `https://xxxx.supabase.co`
- `SUPABASE_ANON_KEY` — public anon key
- `WORKER_URL` — Cloudflare Worker URL (defaults to `http://localhost:8787` if unset)

Copy `.env.example` to `.env` to get started. The built WASM binary contains these values as string literals.

### Leptos Patterns

- `RwSignal<T>` is `Copy` — signals can be freely captured in closures without cloning
- Reactive closures passed to `spawn_local` must be `Send`; use `get_untracked()` instead of `get()` inside async blocks to avoid holding reactive subscriptions across await points
- Components in `view!` that return different types require `.into_any()` to unify them
- Responsive number rendering: use `<Num value=x />` (unsigned) or `<Num value=x signed=true />` — renders full precision on desktop (≥640px) and abbreviated (e.g. `$1.5M`) on mobile via `hidden sm:inline` / `sm:hidden`

### Key Files

- `src/app.rs` — root component, routing, global `AuthState` context, token refresh loop
- `src/config.rs` — compile-time env vars
- `src/format.rs` — number formatting + `Num` component
- `src/pricing/black_scholes.rs` — European option pricing, full Greeks, bisection IV solver
- `src/pages/scenarios.rs` — `evaluate()` function: accumulates trade cash flows, detects option assignments, applies tax (37% ST / 20% LT) to `net_cash`
- `src/api/market.rs` — all Worker HTTP calls
- `src/api/supabase.rs` — all Supabase REST calls (positions, scenarios, auth)
- `worker/src/index.ts` — Cloudflare Worker: JWT verification via Supabase, D1 queries
- `worker/schema.sql` — D1 table definitions (`bars_1min`, `option_chain`)

### Data Storage

**Supabase** (user data, RLS enforced):
- `positions` — `{ id, user_id, payload: json }` — `payload` is a serialized `Position` struct
- `scenarios` — `{ id, user_id, payload: json }` — `payload` is a serialized `Scenario` struct

**Cloudflare D1** (market data, written by external pipeline):
- `bars_1min (symbol, ts, open, high, low, close, volume)` — primary key `(symbol, ts)`
- `option_chain (snapshot_time, symbol, underlying, expiry, option_type, strike, bid, ask, mid, implied_vol, delta, gamma, theta, vega, open_interest, volume)` — primary key `(snapshot_time, symbol)`

### Deployment

The frontend deploys automatically via Cloudflare Pages watching the GitHub `main` branch — no Action needed. The worker deploys via `.github/workflows/deploy-worker.yml`, which only triggers when `worker/**` files change. D1 credentials (`D1_DATABASE_NAME`, `D1_DATABASE_ID`) are injected into `wrangler.toml` at deploy time via `sed` from GitHub Actions secrets (the toml uses `PLACEHOLDER_*` strings).

### Tailwind

Loaded from CDN in `index.html` (not bundled). Custom theme colors: `surface` (#0f1117), `panel` (#1a1d27), `border` (#2a2d3a). Dark-first design. Use `sm:` prefix for desktop-specific styles (≥640px breakpoint).
