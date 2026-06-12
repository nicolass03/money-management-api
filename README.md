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
| `CRON_SECRET` | Bearer token for `/api/v1/cron/daily-expenses` |

## Database schema

Migrations live in `migrations/`. `src/schema.rs` is generated from the live Postgres schema.

**Multi-user model:** `users.id` matches Supabase Auth `sub`. All user-owned tables include `user_id`; API repos scope queries by the authenticated user. `exchange_rate_snapshots` stays global.

**Run migrations** (use session-mode pooler port `5432`, not transaction pooler `6543`):

```bash
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

## Endpoints

All `/api/v1/*` routes require `Authorization: Bearer <supabase_access_token>` except the cron route.

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
| `POST` | `/api/v1/cron/daily-expenses` | Charge due recurring expenses (`CRON_SECRET`) |

## Middleware stack

- Request ID (`X-Request-Id`)
- CORS (`CORS_ORIGIN`)
- Request timeout (`REQUEST_TIMEOUT_SECS`)
- HTTP tracing
- Response compression

## Architecture

```
Next.js UI  ──Bearer JWT──▶  Axum API  ──Diesel──▶  Supabase Postgres
         └──login/session──▶  Supabase Auth
```
