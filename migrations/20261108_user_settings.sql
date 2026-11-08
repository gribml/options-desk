-- User preferences shared across devices.
create table if not exists public.user_settings (
    user_id uuid primary key references auth.users(id) on delete cascade,
    hide_closed_positions boolean not null default false
);
alter table public.user_settings enable row level security;
drop policy if exists "users manage own settings" on public.user_settings;
create policy "users manage own settings" on public.user_settings
    for all to authenticated
    using (auth.uid() = user_id) with check (auth.uid() = user_id);
grant select, insert, update, delete on public.user_settings to authenticated;
