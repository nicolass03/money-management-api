-- Frozen projection aggregates are sums / running balances: widen them to 64-bit so large
-- balances (notably in COP, which has no minor unit) can't overflow.
ALTER TABLE projection_history
    ALTER COLUMN income TYPE BIGINT,
    ALTER COLUMN planned_spent TYPE BIGINT,
    ALTER COLUMN free TYPE BIGINT,
    ALTER COLUMN cumulative TYPE BIGINT;

-- Rows frozen before this release used the old projection rules (dated-budget lines that moved
-- from start to end date, monthly/yearly occurrences before their anchor), so they can conflict
-- with periods now computed live. Drop them: reads fall back to a full live computation and the
-- daily job re-freezes every closed period under the current rules.
DELETE FROM projection_history;
