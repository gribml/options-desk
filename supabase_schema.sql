-- Run this in the Supabase SQL editor.
-- Safe to re-run: uses IF NOT EXISTS / DROP IF EXISTS throughout.

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

-- ──────────────────────────────────────────────────────────────────────────────

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
