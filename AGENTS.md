# money-management-api — agent notes

## Database & API

See the root `money-management/AGENTS.md` for shared conventions (UUID IDs, Supabase auth, RLS, migrations, internal cron).

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
- `GET /expenses/period-view?period=last-period|last-month|last-3-months&includeProjected=true|false` — server-computed period items + `totalSpend`, plus `byTag` / `subscriptionSplit` chart aggregates for the resolved period (moka keyed by period + `includeProjected` + revision + day). Default `includeProjected=false` (actual spend only); web passes `true` to include projected recurring/planned/budget rows in pay period. Chart aggregates are folded in here (no separate chart-summary endpoint) so a tab load is one request.
- `GET /expenses/upcoming-payable?horizonDays=30` — upcoming recurring/planned payables (moka keyed by horizon + revision + day).
- `GET /settings` embeds `primarySchedule` when `primaryScheduleId` is set (avoids separate income-schedule fetch on tab init).
- `GET /projections?includePast=false` — omits past rows from response (default `includePast=true` for web backward compat). Full projection rows remain in moka cache; filter applied on read.
- `GET /settings` exposes `cacheRevision` for client foreground sync (iOS compares on app resume).

Exchange rates also use an in-memory layer (`src/services/fx_memory.rs`) atop the existing `exchange_rate_snapshots` table. Auth skips `ensure_user_exists` after first successful upsert per process (`AppState.known_users`).

Multi-replica deploys: DB revision gives correctness without Redis; each replica holds independent memory until TTL/eviction.

## Railway deployment

The repo includes a multi-stage `Dockerfile` (cargo-chef + `libpq-dev` at build, `libpq5` at runtime) and `railway.toml` with `healthcheckPath = "/health"`.

1. Connect the GitHub repo in Railway (or `railway up` from this directory).
2. Railway auto-detects the Dockerfile via `railway.toml`.
3. Set service variables (no `.env` file in the image):

   | Variable | Notes |
   |----------|-------|
   | `DATABASE_URL` | Supabase **transaction pooler** (`:6543`) for runtime |
   | `SUPABASE_URL` | Supabase project URL (JWKS) |
   | `CORS_ORIGIN` | Your Vercel/Next.js origin(s), comma-separated |
   | `DAILY_EXPENSES_HOUR` | UTC on Railway containers (default `0`) |

4. Do **not** set `PORT` unless you have a reason — Railway injects it and the app reads it from env.
5. Run migrations separately via `./scripts/migrate.sh` (session pooler `:5432`), not on deploy.
6. Generate a public domain under **Networking** and point the Next.js `API_URL` at it.

The internal daily-expense scheduler works as-is on Railway (always-on process + Postgres advisory lock).
