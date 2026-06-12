-- Best-effort rollback. Production rollback is unlikely; restore from backup instead.

-- This down migration is intentionally minimal — reversing UUID PK swaps with
-- existing data is error-prone. Use a database backup to roll back in production.
