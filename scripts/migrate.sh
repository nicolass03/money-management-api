#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

if [[ ! -f .env ]]; then
  echo "error: .env not found — copy .env.example and set DATABASE_URL" >&2
  exit 1
fi

set -a
# shellcheck disable=SC1091
source .env
set +a

if [[ -z "${DATABASE_URL:-}" ]]; then
  echo "error: DATABASE_URL is empty in .env" >&2
  exit 1
fi

if [[ "${DATABASE_URL}" != postgresql://* && "${DATABASE_URL}" != postgres://* ]]; then
  echo "error: DATABASE_URL must be a PostgreSQL URL (got scheme: ${DATABASE_URL%%://*})" >&2
  exit 1
fi

export DATABASE_URL="${DATABASE_URL//:6543/:5432}"

echo "Running migrations (session mode, scheme=${DATABASE_URL%%://*})..."
diesel migration run "$@"
