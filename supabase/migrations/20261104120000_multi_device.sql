-- Multi-device MessengerBird: stable device_key, per-user RLS.
-- Apply after 20261103180000_messengerbird_cloud.sql.

create sequence if not exists public.device_id_seq;
select setval(
  'public.device_id_seq',
  greatest(1, coalesce((select max(id) from public.device), 1))
);

alter table public.device drop constraint if exists device_id_check;
alter table public.device alter column id set default nextval('public.device_id_seq');

alter table public.device add column if not exists device_key text;
alter table public.device add column if not exists owner_id uuid;

create unique index if not exists device_device_key_uidx on public.device (device_key);

alter table public.senders add column if not exists device_key text not null default '';
alter table public.senders drop constraint if exists senders_pkey;
alter table public.senders add primary key (device_key, id);

alter table public.tx_events add column if not exists device_key text;
create index if not exists tx_events_device_key_idx on public.tx_events (device_key);

drop policy if exists "authenticated_all_device" on public.device;
drop policy if exists "authenticated_all_senders" on public.senders;
drop policy if exists "authenticated_all_tx_events" on public.tx_events;

create policy "device_owner" on public.device
  for all to authenticated
  using (owner_id = auth.uid())
  with check (owner_id = auth.uid());

create policy "senders_owner" on public.senders
  for all to authenticated
  using (
    device_key in (
      select device_key from public.device where owner_id = auth.uid()
    )
  )
  with check (
    device_key in (
      select device_key from public.device where owner_id = auth.uid()
    )
  );

create policy "tx_events_owner" on public.tx_events
  for all to authenticated
  using (
    device_key in (
      select device_key from public.device where owner_id = auth.uid()
    )
  )
  with check (
    device_key in (
      select device_key from public.device where owner_id = auth.uid()
    )
  );
