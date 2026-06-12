# money-management-api

Rust HTTP API for the money-management app. Built with **Axum**, **Tokio**, **Tower** middleware, **Diesel** (async), and **Supabase JWT** auth.

The Next.js frontend will call this API instead of querying Postgres directly via Drizzle.

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

Fill in `.env` using the same Supabase values as the Next.js app, plus `SUPABASE_JWT_SECRET` from **Supabase Dashboard → Settings → API → JWT Secret**.

## Database schema

The live Supabase database is already migrated via Drizzle in the `money-management` repo. This API does not run migrations in the scaffold phase.

`src/schema.rs` was authored to match the current Drizzle schema. After future DB shape changes, re-introspect:

```bash
diesel print-schema > src/schema.rs
```

Then adjust `src/models.rs` if new enums or types were added.

## Run

```bash
cargo run
```

The server listens on `HOST:PORT` (default `0.0.0.0:8080`).

## Endpoints

| Method | Path | Auth | Description |
|--------|------|------|-------------|
| `GET` | `/health` | No | `{ "status": "ok" }` |
| `GET` | `/api/v1/settings` | Bearer JWT | Singleton `user_settings` row |

### Example

```bash
curl http://localhost:8080/health

# Obtain a Supabase access token, then:
curl -H "Authorization: Bearer $TOKEN" http://localhost:8080/api/v1/settings
```

## Middleware stack

Applied via Tower `ServiceBuilder`:

- Request ID (`X-Request-Id`)
- CORS (`CORS_ORIGIN`)
- Request timeout (`REQUEST_TIMEOUT_SECS`)
- HTTP tracing
- Response compression

Auth (`Authorization: Bearer`) is applied only under `/api/v1/*`.

## Architecture

```
Next.js UI  ──Bearer JWT──▶  Axum API  ──Diesel──▶  Supabase Postgres
         └──login/session──▶  Supabase Auth
```
