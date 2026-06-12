# money-management-api

Rust HTTP API for the money-management app. Built with **Axum**, **Tokio**, **Tower** middleware, **Diesel** (async), and **Supabase JWT** auth.

The Next.js frontend calls this API for all business logic and database access. Supabase Auth is used only in the UI for login and JWT issuance.

## Prerequisites

- [Rust](https://rustup.rs/) (edition 2021)
- PostgreSQL client library (`libpq`) for linking Diesel
  - macOS: `brew install libpq` and ensure `.cargo/config.toml` paths match your install (or set `LDFLAGS` / `CPPFLAGS` as in `brew info libpq`)
- [Diesel CLI](https://diesel.rs/guides/getting-started) (optional, for schema introspection):
  ```bash
  cargo install diesel_cli --no-default-features --features postgres
  ```

## Setup

```bash
cp .env.example .env
```

Fill in `.env`:

| Variable | Purpose |
|----------|---------|
| `DATABASE_URL` | Supabase Postgres connection string |
| `SUPABASE_URL` | Supabase project URL (fetches public JWKS keys for ES256 access tokens) |
| `CORS_ORIGIN` | Comma-separated allowed origins (default `http://localhost:3000`) |
| `ENABLE_INTERNAL_CRON` | Run in-process daily recurring-expense job (default `true`) |
| `DAILY_EXPENSES_HOUR` | Hour (0–23, server local time) to charge due recurring expenses (default `0`) |

## Database schema

Migrations live in `migrations/`. `src/schema.rs` is generated from the live Postgres schema.

**Multi-user model:** `users.id` matches Supabase Auth `sub`. All user-owned tables include `user_id`; API repos scope queries by the authenticated user. `exchange_rate_snapshots` stays global.

**Row-level security:** Migration `20250612130000_row_level_security` enables Postgres RLS on tenant tables. The API sets `app.user_id` (or `app.is_admin` for internal jobs) per connection via `src/repos/connection.rs`.

**Run migrations** (use session-mode pooler port `5432`, not transaction pooler `6543`).

Diesel CLI does **not** read `.env` automatically. If `DATABASE_URL` is unset, it falls back to SQLite and Postgres migrations fail with `near "EXTENSION": syntax error`.

```bash
./scripts/migrate.sh
# or manually:
set -a && source .env && set +a
export DATABASE_URL="${DATABASE_URL//:6543/:5432}"
diesel migration run
diesel print-schema > src/schema.rs
```

Then adjust `src/models.rs` if new enums or types were added.

Entity primary keys and foreign keys are **UUID**. JSON API responses serialize IDs as strings.

## Run

```bash
cargo run
```

The server listens on `HOST:PORT` (default `0.0.0.0:8080`).

## Internal daily expense job

Recurring expenses are materialized into the `expenses` ledger by an **in-process background scheduler** — there is no HTTP cron endpoint. The job runs once per day at `DAILY_EXPENSES_HOUR` (server local time). Multiple API replicas coordinate via a Postgres advisory lock so only one instance runs the job.

Set `ENABLE_INTERNAL_CRON=false` to disable the scheduler (e.g. if you run a separate worker later).

## Endpoints

All `/api/v1/*` routes require `Authorization: Bearer <supabase_access_token>`.

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/health` | Health check |
| `GET/PATCH` | `/api/v1/settings` | User preferences |
| `GET` | `/api/v1/money-context` | Display currency + exchange rates |
| `GET/POST` | `/api/v1/income-schedules` | Pay schedule CRUD |
| `GET/PATCH/DELETE` | `/api/v1/income-schedules/:id` | |
| `GET/POST` | `/api/v1/income` | Income entries |
| `GET/PATCH/DELETE` | `/api/v1/income/:id` | |
| `POST` | `/api/v1/income/sync-scheduled` | Re-sync all scheduled income |
| `GET/POST` | `/api/v1/expenses` | Expense ledger |
| `GET/PATCH/DELETE` | `/api/v1/expenses/:id` | |
| `POST` | `/api/v1/expenses/early-pay` | Record early payment |
| `GET/POST` | `/api/v1/recurring-expenses` | Recurring expense templates |
| `GET/PATCH/DELETE` | `/api/v1/recurring-expenses/:id` | |
| `GET/POST` | `/api/v1/planned-expenses` | Planned future expenses |
| `GET/PATCH/DELETE` | `/api/v1/planned-expenses/:id` | |
| `GET/POST` | `/api/v1/budgets` | Budget envelopes |
| `GET/PATCH/DELETE` | `/api/v1/budgets/:id` | |
| `GET/POST` | `/api/v1/budgets/:id/expenses` | Budget expense ledger |
| `DELETE` | `/api/v1/budgets/:id/expenses/:expense_id` | |
| `GET` | `/api/v1/savings` | Savings entries |
| `GET` | `/api/v1/tags` | All tag names |
| `GET` | `/api/v1/projections` | Computed cash-flow projection |

## Middleware stack

- Request ID (`X-Request-Id`)
- CORS (`CORS_ORIGIN`)
- Request timeout (`REQUEST_TIMEOUT_SECS`)
- Request body limit (256 KB)
- Per-IP rate limiting on `/api/v1` (30 req/s, burst 60)
- HTTP tracing
- Response compression

## Deploy to Railway

This API runs as an always-on web service — no serverless adapter needed. The repo ships a `Dockerfile` and `railway.toml`.

1. Create a Railway project and connect this repo (or run `railway init` + `railway up` locally).
2. Set service variables in the Railway dashboard:

   | Variable | Value |
   |----------|-------|
   | `DATABASE_URL` | Supabase Postgres connection string (transaction pooler `:6543` is fine) |
   | `SUPABASE_URL` | Supabase project URL |
   | `CORS_ORIGIN` | Your deployed Next.js URL (e.g. `https://your-app.vercel.app`) |
   | `DAILY_EXPENSES_HOUR` | Hour in **UTC** (Railway containers use UTC) |

3. Under **Networking → Generate Domain** to get a public URL.
4. Point the Next.js app’s `API_URL` at that domain (no trailing slash).
5. Run DB migrations from your machine before or after first deploy — they are **not** run automatically:

   ```bash
   ./scripts/migrate.sh
   ```

Railway injects `PORT`; the server binds `0.0.0.0:$PORT` by default. Health checks use `GET /health`.

## Architecture

```
Next.js UI  ──Bearer JWT──▶  Axum API  ──Diesel──▶  Supabase Postgres
         └──login/session──▶  Supabase Auth
                              └── internal scheduler (daily expenses)
```
