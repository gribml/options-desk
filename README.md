# Options Desk

A personal options portfolio manager compiled to WebAssembly. Runs entirely in the browser — no backend server.

## Features

- **Pricer** — Black-Scholes pricing with full Greeks (Δ Γ ν θ ρ) and implied volatility solver
- **Portfolio** — Track stock and option positions; live mark-to-market values and portfolio-level Greeks given current market inputs
- **Scenarios** — What-if analysis: set assumed prices and vols on a future date, see effect of opening and closing positions on the overall portfolio.

## Tech

- [Leptos](https://leptos.dev) 0.8 (Rust → WASM, client-side rendering)
- [Trunk](https://trunk-rs.github.io/trunk/) for building and dev server
- [Supabase](https://supabase.com) for auth and data persistence (positions, scenarios)
- Tailwind CSS (CDN)

## Local development

**Prerequisites:** Rust stable, `wasm32-unknown-unknown` target, trunk.

```bash
rustup target add wasm32-unknown-unknown
cargo install trunk
```

Copy `.env.example` to `.env` and fill in your Supabase credentials:

```bash
cp .env.example .env
```

```
SUPABASE_URL=https://your-project.supabase.co
SUPABASE_ANON_KEY=your-anon-key
```

Run the dev server:

```bash
trunk serve
```

Open `http://localhost:8090`.

## Supabase setup

1. Create a project at [supabase.com](https://supabase.com)
2. Run `supabase_schema.sql` in the SQL editor to create the `positions` and `scenarios` tables
3. Create your user: Authentication → Users → Add user
4. Disable public signups: Authentication → Providers → Email → disable "Enable sign ups"

## Deployment (Cloudflare Pages)

1. Push to GitHub
2. Connect repo in Cloudflare Pages
3. Build command: `bash build.sh`
4. Output directory: `dist`
5. Add `SUPABASE_URL` and `SUPABASE_ANON_KEY` as environment variables

First build takes ~10 minutes (compiling Rust); subsequent builds are faster.

## Pricing model

European-style Black-Scholes (no dividends). Suitable for equity options on non-dividend-paying US stocks. IV is solved by bisection. The model is isolated in `src/pricing/black_scholes.rs` and can be swapped out independently of the UI.
