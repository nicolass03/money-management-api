# money-management-api — agent notes

## Database & API

See the root `money-management/AGENTS.md` for shared conventions (UUID IDs, Supabase auth, RLS, migrations, internal cron).

## Invite onboarding (API enforcement)

- `users.onboarding_completed_at` (migration `20260615200000_user_onboarding_completed`): `NULL` blocks all `/api/v1/*` routes except `POST /auth/complete-onboarding` (403 `onboarding_required`).
- `POST /auth/complete-onboarding` verifies `auth.users.encrypted_password` is non-empty via the `app_user_has_password(uuid)` SECURITY DEFINER function (the runtime role can't read `auth.users`; not client `user_metadata`), then sets `onboarding_completed_at`.
- `require_auth` checks onboarding once per user per process (`AppState.onboarded_users`); onboarding never reverts.
- Existing users are backfilled at migration time; new invited users stay blocked until they set a password on web and the client calls complete-onboarding.
- Apply migration before deploy: `./scripts/migrate.sh`.

## Local build

The API needs **no libpq or OpenSSL**: Diesel is built with only its `postgres_backend` feature (queries run through diesel-async / tokio-postgres) and outbound HTTP uses rustls. Only the **Diesel CLI** (used by `./scripts/migrate.sh`) still links libpq.

## Database roles and RLS

- Runtime connects as **`app_api`** (migration `20261027140000_app_api_role`): no `BYPASSRLS`, owns no tables, data-only grants (plus default privileges for future tables). Every query goes through the tenant policies (`app.user_id` / `app.is_admin`), so RLS is enforced. The `postgres` role owns the tables and bypasses RLS — use it only for migrations (`MIGRATION_DATABASE_URL`).
- Runtime must use the **session pooler (`:5432`)**: `set_user_context` sets session-level `app.user_id`, and the daily job uses a session advisory lock. Under the transaction pooler (`:6543`) consecutive statements may land on different backends.

## Rate limiting

Per-IP limits on `/api/v1`, auth-failure tracking, and force-refresh throttling are **disabled in debug builds** (`cargo run`) and **enabled in release** (Docker/Railway). Override with `RATE_LIMIT_ENABLED=true` or `false`.

- The client IP comes from `SmartIpKeyExtractor` (`X-Forwarded-For` / `X-Real-IP`, which Railway's edge overwrites), not the peer address (Railway's proxy).
- `/api/v1`: 100 req/s per IP, burst 200 (`per_millisecond(10)` — `per_second(n)` means one request every *n seconds*).
- Failed token validations: burst 30 per IP, refilling 1/min; beyond that → **429**. JWKS refetches (unknown `kid`) are capped at one per 60 s.
- `GET /money-context?forceRefresh=true`: 10 per user, refilling 1/min → **429**.

## In-process caching

Read-heavy endpoints use a revision-keyed **moka** cache (`src/cache/`). `user_settings.cache_revision` increments on every write (including daily cron materialization); cache keys are `(user_id, resource, revision)` so stale entries miss automatically.

| Env | Default | Notes |
|-----|---------|-------|
| `CACHE_ENABLED` | `true` | Set `false` to bypass cache (debug) |
| `CACHE_MAX_ENTRIES` | `10000` | Per-resource moka capacity |
| `DB_POOL_MAX_SIZE` | `10` | bb8 pool max connections |

**New write paths** must call `settings::bump_cache_revision` inside the same Postgres transaction and finish the route with `routes::helpers::finish_write(&state, user_id, history)` (`history` = which frozen projection periods to re-freeze, or `None`). Invalidation just evicts the cached settings row: the next read sees the new revision and every revision-keyed entry misses by key (there is no per-resource scope map — moka predicate invalidation was never enabled, so it never ran). Cache hits return shared `Arc`s (no copying). Another replica sees a write once its 60 s settings TTL expires.

**Query params (performance):**
- `GET /expenses?from=YYYY-MM-DD&to=YYYY-MM-DD` — date-filtered list (both params required), filtered from the cached full list.
- `GET /expenses/period-view?period=last-period|last-month|last-3-months&includeProjected=true|false&asOf=YYYY-MM-DD` — server-computed period items + `totalSpend`, plus `byTag` / `subscriptionSplit` chart aggregates for the resolved period (moka keyed by period + `includeProjected` + revision + `asOf`). Default `includeProjected=false` (actual spend only); web passes `true` to include projected recurring/planned/budget rows in pay period. Chart aggregates are folded in here (no separate chart-summary endpoint) so a tab load is one request. Also returns `extraSpent` + `extraSpentLimit` (see below). **`items` order:** `created_at` descending (newest first) — expenses tab only; do not re-sort client-side.
- **Extra spent / limit** (`extra_spent_limit` on `user_settings`, nullable int, display-currency minor units): `extraSpent` in the period-view response is the sum of **persisted** expenses in the resolved period whose `recurring_id`, `planned_expense_id`, and `budget_id` are **all NULL** (i.e. manual/unplanned spend), converted to display currency. It is computed from raw expense rows in `compute_extra_spent` (`services/expense_period.rs`), **not** from `items`, so it ignores `includeProjected` and budget-summary aggregation. `extraSpentLimit` echoes the user's setting. The limit is informational only (no enforcement on expense creation). Clients only surface the `/ limit` comparison and warning colors for the pay period (`isPayPeriod`); calendar ranges (`last-month`, `last-3-months`) show `extraSpent` alone. Individual persisted expense rows in `items` must echo `budget_id` from the DB row (do not zero it out) so clients can distinguish budget-linked charges from manual `extra` spend — dated-budget line items are still omitted from `items` on the pay period when a budget summary row is shown.
- `GET /expenses/upcoming-payable?horizonDays=30&asOf=YYYY-MM-DD` — upcoming recurring/planned payables (moka keyed by horizon + revision + `asOf`).
- `GET /settings` embeds `primarySchedule` when `primaryScheduleId` is set (avoids separate income-schedule fetch on tab init).
- `GET /settings` includes `language` (`en` | `es`) from `user_settings.language`; `PATCH /settings` accepts `language` and validates to that fixed set.
- `GET /projections?includePast=false&asOf=YYYY-MM-DD` — omits past rows from response (default `includePast=true` for web backward compat). Full projection rows remain in moka cache keyed by revision + `asOf`; filter applied on read. The horizon comes from nullable `user_settings.projection_end_date`; `NULL` means a rolling 12 months, while an explicit end includes its full containing primary-schedule pay period. Settings writes validate explicit ends from client-local `asOf` through `asOf + 24 months`. **`expenseItems` order:** charge `date` ascending (chronological) — projections tab + `GET /projections/period-items`; distinct from period-view `created_at` desc.
- `GET /reports/summary?from=YYYY-MM-DD&to=YYYY-MM-DD&comparePrior=true|false` — custom date-range reporting (web-only consumer). Returns KPIs (income, expenses, net, extra spent, counts, avg daily spend), `byTag`, `extraSpentByTag` (manual/unplanned spend only), `subscriptionSplit`, time-series buckets (day/week/month auto granularity), `extraSpentTimeSeries` (manual spend per primary-schedule pay period overlapping the range; clipped to the selected dates), optional equal-length prior-period KPI comparison. Actual persisted expenses/income only (no projected rows). Max range 730 days. Built from the cached full expense/income lists (filtered in memory).
- `GET /settings` exposes `cacheRevision` for client foreground sync (iOS compares on app resume).
- **`POST /expenses`:** any valid expense `date` (no pay-period gate). **Early pay** (`POST /expenses/early-pay`) still requires `paid_date` in the current pay period and not in the future.
- **`POST /budgets/:id/expenses`:** any valid expense `date` (no pay-period gate). No remaining-budget cap on write — tracking only. Rejects when `completed_at` is set. Projection / period-view display still uses budget `start_date`/`end_date` for when the envelope appears (`budget_status`, `expense_period`).
- **`POST /budgets/:id/complete`:** optional body `{ "asOf": "YYYY-MM-DD" }` (client local today). Sets `completed_at`, `end_date = asOf`; open-ended budgets also get `start_date` from earliest expense or `created_at`. `PATCH /budgets/:id` rejected when completed. `get_budget_projection_amount` returns actual `spent` when `completed_at` is set (immediate switch on finish day). Migration: `20260702120000_budget_completed_at`.

**`asOf` (client local calendar date):** Pay-period boundaries, rolling calendar ranges (`last-month` / `last-3-months`), projection `is_past`, and upcoming-payable windows all depend on a reference “today”. `validation::today_iso()` is **UTC** and is only the fallback when `asOf` is omitted. **Web and iOS must pass `asOf` in the user's local `YYYY-MM-DD`** on `period-view`, `upcoming-payable`, and `projections` — otherwise the API can still resolve the previous pay period for hours after local midnight in UTC+ timezones (pay period rolls the day **after** payday, not on payday itself). Internal cron uses UTC `today_utc()`.

**`asOf` on writes:** date rules on writes follow the client's local today via `validation::resolve_client_today` (must be within ±1 day of UTC today; UTC fallback when omitted): `POST /planned-expenses` body `asOf` (a dated item must be after it), `POST /planned-expenses/{id}/pay` body `asOf` (payment date), `POST /expenses/early-pay` body `asOf` (paid date ≤ it, within its pay period), `POST /budgets/:id/complete`, `PATCH /settings`, and the `?asOf=` query on `GET /accounts`, `PATCH /accounts/{id}` (balances) and `GET /subscription-reminders`.

Exchange rates also use an in-memory layer (`src/services/fx_memory.rs`) atop the existing `exchange_rate_snapshots` table. Auth skips `ensure_user_exists` after first successful upsert per process (`AppState.known_users`).

Multi-replica deploys: DB revision gives correctness without Redis; each replica holds independent memory until TTL/eviction.

## Income materialization (parity with recurring expenses)

Scheduled income is **materialized by the daily cron**, not pre-synced. `jobs/daily_expenses.rs` now runs both `charge_due_expenses_for_date` and `charge_due_income_for_date` under the same advisory lock; income creation invalidates `IncomeChange` separately. There is **no** `sync_scheduled_income` service and **no** `POST /income/sync-scheduled` route anymore (both removed). Schedule create/update only bump `cache_revision`; they do **not** write `income` rows.

- **No backfill:** creating a schedule does not retro-create past pay rows (matches recurring expenses). Past income that predates a schedule must be added manually.
- **Idempotency:** the cron skips a schedule when an `income` row already exists for `(schedule_id, date)` — including soft-deleted tombstones — and treats the `income_scheduled_schedule_date_unique` violation as a no-op (`charge_due_income.rs`).
- **`income.amount_overridden` / `income.deleted_at`** (migration `20260615190000_income_materialization`): amount edits on a materialized scheduled row set `amount_overridden`; deletes on scheduled rows are **soft deletes** (set `deleted_at`). The tombstone keeps the `(schedule_id, date)` slot occupied so the cron never resurrects a deleted occurrence. **Do not** hard-delete scheduled income or you reintroduce the resurrection bug. `IncomeRow._amount_overridden` is prefixed `_` (mapped via `#[diesel(column_name)]`) because it is written but never read in Rust.
- **Edit/delete rules** (`routes/income.rs`): manual income → full edit (`update`) + hard delete; scheduled income → amount-only override (`update_amount`, schedule-owned name/date/currency untouched) + soft delete (`soft_delete`).
- **Repo reads:** `income::list_all*` returns **active** rows only (`deleted_at IS NULL`) for `GET /income`; `list_with_deleted_with_conn` returns tombstones too and is used **only** by projections.
- **Projections** (`services/projections.rs`): opening balance = the sum of every account's (active + archived) `initial_amount` converted to display currency (`projection_history::load_projection_inputs`, shared by the live and frozen paths). `income_total` = active persisted rows in the period **plus** projected future occurrences (`date >= today`) from **every** pay schedule (not just primary) whose `(schedule_id, date)` is neither materialized nor tombstoned. The migration deletes pre-synced **future** `source='scheduled'` rows so the cron/projection becomes the single source of truth; past materialized rows are kept. The `income` migration must be applied (`./scripts/migrate.sh`) before running the API, or `GET /income`/projections fail on the missing columns.

## Recurring expense delete (soft)

- **`DELETE /recurring-expenses/:id`** sets `recurring_expenses.deleted_at` (migration `20260702180000_recurring_expense_soft_delete`). **Materialized `expenses` rows are kept** — deleting a subscription cancels future charges, it does not refund/erase history.
- Active reads (`list_all`, `find_by_id`, cron, upcoming-payable, projections) filter `deleted_at IS NULL`. Same practical effect as ending the schedule; use **`last_payment_date`** when you want to keep the row visible with an end date.
- `expenses.recurring_id` FK is **`ON DELETE RESTRICT`** so a hard DB delete cannot cascade-wipe charges. **Do not** hard-delete recurring rows.

## Income pay schedule delete (soft)

- **`DELETE /income-schedules/:id`** sets `income_pay_schedules.deleted_at` (migration `20260911120000_income_schedule_soft_delete`) and clears it as the primary schedule. **Materialized `income` rows (and tombstones) are kept**: deleting a schedule stops future pay dates, it does not erase received income.
- All reads in `repos/income_schedules.rs` (`list_all`, `find_by_id*`, `update`) filter `deleted_at IS NULL`, so the cron, projections, settings and reports treat a deleted schedule as gone. **Do not** hard-delete schedules. `income.schedule_id` has a NO ACTION FK, so a hard delete fails while income references the schedule.

## One-time (planned) expenses

- `planned_expenses.date` is **nullable** (migration `20260911130000_planned_expense_optional_date`). An undated item (for example a debt with no due date) never appears in projections, periods or upcoming-payable; it counts only once paid.
- **Nothing auto-records planned expenses.** The daily job only charges recurring expenses. A dated item is projected until its date; after that it stays in the list (clients show it as "overdue") until the user pays it.
- **`POST /planned-expenses/{id}/pay`** `{ amount }` records the full payment **today** (one payment per item; the amount may differ from the planned amount, which sets `amount_overridden`). It draws from the item's account while that account is active, otherwise a same-currency account (`routes/helpers.rs::pick_payment_account`, shared with early pay). The expense links back via `planned_expense_id`, and `GET /planned-expenses` returns `paid: true` for the item from then on.
- **`DELETE /planned-expenses/{id}` is a soft delete** (`deleted_at`, migration `20261027120000_planned_expense_soft_delete`; 404 when missing). A hard delete would null the payment's `planned_expense_id` (`ON DELETE SET NULL`) and turn planned spend into "extra" spend. All reads in `repos/planned_expenses.rs` filter `deleted_at IS NULL`. **Do not** hard-delete planned expenses.

## Expense edits

- **`PATCH /expenses/{id}`** with only `{ amount }`: amount override for any expense. A **recurring charge may be set to `0`** to void an occurrence that didn't happen — the row keeps its `(recurring_id, date)` slot, so the daily job never re-charges it, and it contributes nothing to totals or balances.
- With the full field set `{ amount, name, currency, date, tags, isSubscription, accountId? }`: full edit of a **manual** expense (no recurring/planned/budget source; others → 400). Currency follows the account (`resolve_account_for_update`); history re-freezes from the earlier of the old/new dates.
- Period-view / projection items carry `accountId` for recorded expenses, so clients can open the full edit form straight from a list row.
- Clients: web `ExpenseForm` (edit mode) / iOS `ExpenseFormModel(editing:)` for manual rows; the amount editor allows `0` only for recurring rows ("0 = didn't happen").
- A unique-index violation (e.g. a double-tapped pay) returns **409**, not 500.

## Railway deployment

The repo includes a multi-stage `Dockerfile` (cargo-chef; no system libraries beyond `ca-certificates`) and `railway.toml` with `healthcheckPath = "/health"`.

1. Connect the GitHub repo in Railway (or `railway up` from this directory).
2. Railway auto-detects the Dockerfile via `railway.toml`.
3. Set service variables (no `.env` file in the image):

   | Variable | Notes |
   |----------|-------|
   | `DATABASE_URL` | Supabase **session pooler** (`:5432`) as `app_api.<project-ref>` (see *Database roles and RLS*) |
   | `SUPABASE_URL` | Supabase project URL (JWKS) |
   | `CORS_ORIGIN` | Deployed UI origin(s) (the Vite SPA on Railway), comma-separated |
   | `DAILY_EXPENSES_HOUR` | UTC on Railway containers (default `0`) |

4. Do **not** set `PORT` unless you have a reason — Railway injects it and the app reads it from env.
5. Run migrations separately via `./scripts/migrate.sh` (session pooler `:5432`, as `postgres` — set `MIGRATION_DATABASE_URL` in `.env` once `DATABASE_URL` points at `app_api`), not on deploy. Migrations go **before** the deploy that needs them.
6. Generate a public domain under **Networking** and point the UI's `VITE_API_URL` at it (rebuild the UI — it's baked at build time).

The internal daily-expense scheduler works as-is on Railway (always-on process + Postgres advisory lock).
