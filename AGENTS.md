# money-management-api — agent notes

## Database & API

See the root `money-management/AGENTS.md` for shared conventions (UUID IDs, Supabase auth, RLS, migrations, internal cron).

## Invite onboarding (API enforcement)

- `users.onboarding_completed_at` (migration `20260615200000_user_onboarding_completed`): `NULL` blocks all `/api/v1/*` routes except `POST /auth/complete-onboarding` (403 `onboarding_required`).
- `POST /auth/complete-onboarding` verifies `auth.users.encrypted_password` is non-empty via `admin_connection` (not client `user_metadata`), then sets `onboarding_completed_at`.
- Existing users are backfilled at migration time; new invited users stay blocked until they set a password on web and the client calls complete-onboarding.
- Apply migration before deploy: `./scripts/migrate.sh`.

## Local build (macOS + libpq)

Diesel links against `libpq`. On macOS, `brew install libpq` and ensure `.cargo/config.toml` darwin `rustflags` match your Homebrew prefix (default `/opt/homebrew`).

If linking fails, export:

```bash
export PQ_LIB_DIR="$(brew --prefix libpq)/lib"
export PQ_INCLUDE_DIR="$(brew --prefix libpq)/include"
```

Do **not** put macOS-only `PQ_*` paths in global Cargo `[env]` — that breaks Linux Docker/Railway builds.

## Rate limiting

Per-IP limits on `/api/v1`, auth-failure tracking, and force-refresh throttling are **disabled in debug builds** (`cargo run`) and **enabled in release** (Docker/Railway). Override with `RATE_LIMIT_ENABLED=true` or `false`.

## In-process caching

Read-heavy endpoints use a revision-keyed **moka** cache (`src/cache/`). `user_settings.cache_revision` increments on every write (including daily cron materialization); cache keys are `(user_id, resource, revision)` so stale entries miss automatically.

| Env | Default | Notes |
|-----|---------|-------|
| `CACHE_ENABLED` | `true` | Set `false` to bypass cache (debug) |
| `CACHE_MAX_ENTRIES` | `10000` | Per-resource moka capacity |
| `DB_POOL_MAX_SIZE` | `10` | bb8 pool max connections |

**New write paths** must call `settings::bump_cache_revision` inside the same Postgres transaction and `state.cache.invalidate(InvalidationScope::…, user_id)` from the route handler (mirrors web `invalidation.ts`). Scopes live in `src/cache/invalidation.rs`.

**Query params (performance):**
- `GET /expenses?from=YYYY-MM-DD&to=YYYY-MM-DD` — date-filtered list (bypasses moka; both params required). Unfiltered list still uses moka cache.
- `GET /expenses/period-view?period=last-period|last-month|last-3-months&includeProjected=true|false&asOf=YYYY-MM-DD` — server-computed period items + `totalSpend`, plus `byTag` / `subscriptionSplit` chart aggregates for the resolved period (moka keyed by period + `includeProjected` + revision + `asOf`). Default `includeProjected=false` (actual spend only); web passes `true` to include projected recurring/planned/budget rows in pay period. Chart aggregates are folded in here (no separate chart-summary endpoint) so a tab load is one request. Also returns `extraSpent` + `extraSpentLimit` (see below).
- **Extra spent / limit** (`extra_spent_limit` on `user_settings`, nullable int, display-currency minor units): `extraSpent` in the period-view response is the sum of **persisted** expenses in the resolved period whose `recurring_id`, `planned_expense_id`, and `budget_id` are **all NULL** (i.e. manual/unplanned spend), converted to display currency. It is computed from raw expense rows in `compute_extra_spent` (`services/expense_period.rs`), **not** from `items`, so it ignores `includeProjected` and budget-summary aggregation. `extraSpentLimit` echoes the user's setting. The limit is informational only (no enforcement on expense creation). Clients only surface the `/ limit` comparison and warning colors for the pay period (`isPayPeriod`); calendar ranges (`last-month`, `last-3-months`) show `extraSpent` alone. Individual persisted expense rows in `items` must echo `budget_id` from the DB row (do not zero it out) so clients can distinguish budget-linked charges from manual `extra` spend — dated-budget line items are still omitted from `items` on the pay period when a budget summary row is shown.
- `GET /expenses/upcoming-payable?horizonDays=30&asOf=YYYY-MM-DD` — upcoming recurring/planned payables (moka keyed by horizon + revision + `asOf`).
- `GET /settings` embeds `primarySchedule` when `primaryScheduleId` is set (avoids separate income-schedule fetch on tab init).
- `GET /settings` includes `language` (`en` | `es`) from `user_settings.language`; `PATCH /settings` accepts `language` and validates to that fixed set.
- `GET /projections?includePast=false&asOf=YYYY-MM-DD` — omits past rows from response (default `includePast=true` for web backward compat). Full projection rows remain in moka cache keyed by revision + `asOf`; filter applied on read.
- `GET /settings` exposes `cacheRevision` for client foreground sync (iOS compares on app resume).
- **`POST /budgets/:id/expenses`:** open-ended budgets require the expense `date` in the **current pay period** (primary schedule). **Dated budgets** accept **any** valid expense `date` — no budget-range or pay-period check on write. Projection / period-view display still uses budget `start_date`/`end_date` for when the envelope appears (`budget_status`, `expense_period`).

**`asOf` (client local calendar date):** Pay-period boundaries, rolling calendar ranges (`last-month` / `last-3-months`), projection `is_past`, and upcoming-payable windows all depend on a reference “today”. `validation::today_iso()` is **UTC** and is only the fallback when `asOf` is omitted. **Web and iOS must pass `asOf` in the user's local `YYYY-MM-DD`** on `period-view`, `upcoming-payable`, and `projections` — otherwise the API can still resolve the previous pay period for hours after local midnight in UTC+ timezones (pay period rolls the day **after** payday, not on payday itself). Internal cron / write validation still use UTC `today_iso()`.

Exchange rates also use an in-memory layer (`src/services/fx_memory.rs`) atop the existing `exchange_rate_snapshots` table. Auth skips `ensure_user_exists` after first successful upsert per process (`AppState.known_users`).

Multi-replica deploys: DB revision gives correctness without Redis; each replica holds independent memory until TTL/eviction.

## Income materialization (parity with recurring expenses)

Scheduled income is **materialized by the daily cron**, not pre-synced. `jobs/daily_expenses.rs` now runs both `charge_due_expenses_for_date` and `charge_due_income_for_date` under the same advisory lock; income creation invalidates `IncomeChange` separately. There is **no** `sync_scheduled_income` service and **no** `POST /income/sync-scheduled` route anymore (both removed). Schedule create/update only bump `cache_revision`; they do **not** write `income` rows.

- **No backfill:** creating a schedule does not retro-create past pay rows (matches recurring expenses). Past income that predates a schedule must be added manually.
- **Idempotency:** the cron skips a schedule when an `income` row already exists for `(schedule_id, date)` — including soft-deleted tombstones — and treats the `income_scheduled_schedule_date_unique` violation as a no-op (`charge_due_income.rs`).
- **`income.amount_overridden` / `income.deleted_at`** (migration `20260615190000_income_materialization`): amount edits on a materialized scheduled row set `amount_overridden`; deletes on scheduled rows are **soft deletes** (set `deleted_at`). The tombstone keeps the `(schedule_id, date)` slot occupied so the cron never resurrects a deleted occurrence. **Do not** hard-delete scheduled income or you reintroduce the resurrection bug. `IncomeRow._amount_overridden` is prefixed `_` (mapped via `#[diesel(column_name)]`) because it is written but never read in Rust.
- **Edit/delete rules** (`routes/income.rs`): manual income → full edit (`update`) + hard delete; scheduled income → amount-only override (`update_amount`, schedule-owned name/date/currency untouched) + soft delete (`soft_delete`).
- **Repo reads:** `income::list_all*` returns **active** rows only (`deleted_at IS NULL`) for `GET /income`; `list_with_deleted_with_conn` returns tombstones too and is used **only** by projections.
- **Projections** (`services/projections.rs`): `income_total` = active persisted rows in the period **plus** projected future occurrences (`date >= today`) from **every** pay schedule (not just primary) whose `(schedule_id, date)` is neither materialized nor tombstoned. The migration deletes pre-synced **future** `source='scheduled'` rows so the cron/projection becomes the single source of truth; past materialized rows are kept. The `income` migration must be applied (`./scripts/migrate.sh`) before running the API, or `GET /income`/projections fail on the missing columns.

## Railway deployment

The repo includes a multi-stage `Dockerfile` (cargo-chef + `libpq-dev` at build, `libpq5` at runtime) and `railway.toml` with `healthcheckPath = "/health"`.

1. Connect the GitHub repo in Railway (or `railway up` from this directory).
2. Railway auto-detects the Dockerfile via `railway.toml`.
3. Set service variables (no `.env` file in the image):

   | Variable | Notes |
   |----------|-------|
   | `DATABASE_URL` | Supabase **transaction pooler** (`:6543`) for runtime |
   | `SUPABASE_URL` | Supabase project URL (JWKS) |
   | `CORS_ORIGIN` | Deployed UI origin(s) (the Vite SPA on Railway), comma-separated |
   | `DAILY_EXPENSES_HOUR` | UTC on Railway containers (default `0`) |

4. Do **not** set `PORT` unless you have a reason — Railway injects it and the app reads it from env.
5. Run migrations separately via `./scripts/migrate.sh` (session pooler `:5432`), not on deploy.
6. Generate a public domain under **Networking** and point the UI's `VITE_API_URL` at it (rebuild the UI — it's baked at build time).

The internal daily-expense scheduler works as-is on Railway (always-on process + Postgres advisory lock).
