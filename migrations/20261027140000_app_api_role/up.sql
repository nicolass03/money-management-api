-- Dedicated runtime role so row-level security is actually enforced. The API used to connect as
-- `postgres`, which owns every table and has BYPASSRLS, so all tenant policies were skipped.
-- `app_api` owns nothing and only gets data access, so every query goes through the policies.
--
-- The password is set out of band, never in git:
--   ALTER ROLE app_api WITH LOGIN PASSWORD '<secret>';
-- The API then connects as `app_api.<project-ref>` on the session pooler (:5432). Migrations keep
-- running as `postgres` (DDL, ownership, default privileges).
DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'app_api') THEN
        CREATE ROLE app_api NOLOGIN NOBYPASSRLS;
    END IF;
END
$$;

GRANT USAGE ON SCHEMA public TO app_api;
GRANT SELECT, INSERT, UPDATE, DELETE ON
    users, user_settings, accounts, tags, expenses, expense_tags, recurring_expenses,
    recurring_expense_tags, planned_expenses, planned_expense_tags, budgets, budget_tags, income,
    income_pay_schedules, savings, subscription_reminders, projection_history,
    exchange_rate_snapshots
TO app_api;
GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA public TO app_api;
GRANT EXECUTE ON FUNCTION app_rls_allowed(UUID), app_rls_allowed_user_row(UUID) TO app_api;

-- Tables and sequences created by later migrations (run as postgres) are granted automatically.
ALTER DEFAULT PRIVILEGES FOR ROLE postgres IN SCHEMA public
    GRANT SELECT, INSERT, UPDATE, DELETE ON TABLES TO app_api;
ALTER DEFAULT PRIVILEGES FOR ROLE postgres IN SCHEMA public
    GRANT USAGE, SELECT ON SEQUENCES TO app_api;

-- Onboarding must know whether the Supabase auth user has set a password. `auth.users` stays
-- off-limits to app_api; this definer function exposes only that one boolean.
CREATE OR REPLACE FUNCTION public.app_user_has_password(uid UUID) RETURNS BOOLEAN
    LANGUAGE sql STABLE SECURITY DEFINER SET search_path = ''
AS $$
    SELECT COALESCE(
        (SELECT encrypted_password IS NOT NULL AND length(encrypted_password) > 0
         FROM auth.users WHERE id = uid),
        false
    );
$$;
REVOKE ALL ON FUNCTION public.app_user_has_password(UUID) FROM PUBLIC;
-- Supabase grants new public functions to its Data API roles by default; keep this one private.
REVOKE ALL ON FUNCTION public.app_user_has_password(UUID) FROM anon, authenticated, service_role;
GRANT EXECUTE ON FUNCTION public.app_user_has_password(UUID) TO app_api;
