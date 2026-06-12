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
