-- MessengerBird cloud (Pi app) — not VisionLab.
-- device singleton, senders mirror, tx_events log. RLS: authenticated only.

create extension if not exists "pgcrypto";

create table if not exists public.device (
  id integer primary key default 1 check (id = 1),
  display_name text not null default 'MessengerBird',
  lan_ip text,
  tailscale_ip text,
  magicdns text,
  auth_token_enabled boolean not null default false,
  updated_at timestamptz not null default now(),
  last_seen_at timestamptz
);

create table if not exists public.senders (
  id text primary key,
  name text not null,
  port integer not null,
  phrase text not null,
  listen text not null,
  muted boolean not null default false,
  updated_at timestamptz not null default now()
);

create table if not exists public.tx_events (
  id uuid primary key default gen_random_uuid(),
  -- Text id (no FK) so events can land before sender upsert / offline catch-up.
  sender_id text,
  phrase text not null,
  ok boolean not null,
  source text not null check (source in ('http', 'ui', 'test')),
  client_host text,
  detail text,
  fired_at timestamptz not null default now()
);

create index if not exists tx_events_fired_at_idx on public.tx_events (fired_at desc);
create index if not exists tx_events_sender_id_idx on public.tx_events (sender_id);

alter table public.device enable row level security;
alter table public.senders enable row level security;
alter table public.tx_events enable row level security;

drop policy if exists "authenticated_all_device" on public.device;
create policy "authenticated_all_device" on public.device
  for all to authenticated using (true) with check (true);

drop policy if exists "authenticated_all_senders" on public.senders;
create policy "authenticated_all_senders" on public.senders
  for all to authenticated using (true) with check (true);

drop policy if exists "authenticated_all_tx_events" on public.tx_events;
create policy "authenticated_all_tx_events" on public.tx_events
  for all to authenticated using (true) with check (true);

insert into public.device (id, display_name)
values (1, 'MessengerBird')
on conflict (id) do nothing;

insert into public.senders (id, name, port, phrase, listen, muted)
values
  ('silo_empty', 'Silo is empty', 8787, 'Silo is empty', '0.0.0.0:8787', false),
  ('check_camera', 'Check camera', 8788, 'Check camera', '0.0.0.0:8788', false),
  ('starlinger_red', 'Starlinger red', 8789, 'Starlinger red', '0.0.0.0:8789', false)
on conflict (id) do nothing;
