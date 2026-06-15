ALTER TABLE users
    ADD COLUMN onboarding_completed_at TIMESTAMPTZ;

-- Existing accounts already have passwords; invited users created after this get NULL until complete-onboarding.
UPDATE users
SET onboarding_completed_at = created_at
WHERE onboarding_completed_at IS NULL;
