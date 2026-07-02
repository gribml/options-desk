-- Run this in the Supabase SQL editor.
-- Safe to re-run: uses IF NOT EXISTS / DROP IF EXISTS throughout.

-- ── Positions ─────────────────────────────────────────────────────────────────

create table if not exists positions (
    id          uuid primary key,
    user_id     uuid not null references auth.users(id) on delete cascade,
    payload     jsonb not null,
    created_at  timestamptz not null default now()
);

alter table positions enable row level security;

drop policy if exists "users see own positions" on positions;
create policy "users see own positions" on positions
    for all using (auth.uid() = user_id);

create index if not exists positions_user_id_idx on positions (user_id);

-- ── Scenarios ─────────────────────────────────────────────────────────────────

create table if not exists scenarios (
    id          uuid primary key,
    user_id     uuid not null references auth.users(id) on delete cascade,
    payload     jsonb not null,
    created_at  timestamptz not null default now()
);

alter table scenarios enable row level security;

drop policy if exists "users see own scenarios" on scenarios;
create policy "users see own scenarios" on scenarios
    for all using (auth.uid() = user_id);

create index if not exists scenarios_user_id_created_at_idx on scenarios (user_id, created_at desc);

-- ── Tax profiles ──────────────────────────────────────────────────────────────
-- One row per (user, tax_year). payload holds an append-only list of timestamped
-- revisions; the latest revision is the current profile. Written by the frontend
-- (user JWT) and read by the Cloudflare Worker (user JWT, RLS-scoped) to compute
-- federal tax. The unique (user_id, tax_year) is the upsert conflict target.

create table if not exists tax_profiles (
    id          uuid primary key,
    user_id     uuid not null references auth.users(id) on delete cascade,
    tax_year    smallint not null,
    payload     jsonb not null,
    created_at  timestamptz not null default now(),
    unique (user_id, tax_year)
);

alter table tax_profiles enable row level security;

drop policy if exists "users see own tax_profiles" on tax_profiles;
create policy "users see own tax_profiles" on tax_profiles
    for all using (auth.uid() = user_id);

create index if not exists tax_profiles_user_id_year_idx on tax_profiles (user_id, tax_year);

-- ── Price history ─────────────────────────────────────────────────────────────
-- Written by the Cloudflare Worker (service key, bypasses RLS).
-- Read by the frontend (authenticated user JWT).

create table if not exists price_history (
    symbol      text        not null,
    date        date        not null,
    open        numeric     not null,
    high        numeric     not null,
    low         numeric     not null,
    close       numeric     not null,
    volume      bigint,
    primary key (symbol, date)
);

alter table price_history enable row level security;

drop policy if exists "authenticated users read price_history" on price_history;
create policy "authenticated users read price_history" on price_history
    for select using (auth.role() = 'authenticated');

create index if not exists price_history_symbol_date_idx on price_history (symbol, date desc);

create table if not exists combos (
  id uuid primary key,
  user_id uuid not null references auth.users(id) default auth.uid(),
  payload jsonb not null,
  created_at timestamptz
);
alter table combos enable row level security;
create policy "own combos" on combos
  for all using (auth.uid() = user_id) with check (auth.uid() = user_id);