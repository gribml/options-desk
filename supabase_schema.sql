-- Run this in the Supabase SQL editor after creating your project.
-- Enable RLS so users can only see their own data.

create table if not exists positions (
    id          uuid primary key,
    user_id     uuid not null references auth.users(id) on delete cascade,
    payload     jsonb not null,
    created_at  timestamptz not null default now()
);

alter table positions enable row level security;

create policy "users see own positions" on positions
    for all using (auth.uid() = user_id);

create index on positions (user_id);

-- ──────────────────────────────────────────────────────────────────────────────

create table if not exists scenarios (
    id          uuid primary key,
    user_id     uuid not null references auth.users(id) on delete cascade,
    payload     jsonb not null,
    created_at  timestamptz not null default now()
);

alter table scenarios enable row level security;

create policy "users see own scenarios" on scenarios
    for all using (auth.uid() = user_id);

create index on scenarios (user_id, created_at desc);
