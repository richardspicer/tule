-- Work 0011: sole Phase 1 provider slot moves to xAI subscription OAuth.
-- Historical openai-chatgpt-compat rows remain for session provenance.

INSERT INTO provider_profiles (
    id, provider_kind, visible_model_id, credential_handle, expires_at_unix_ms,
    created_at_unix_ms, updated_at_unix_ms
)
SELECT
    'xai-subscription-oauth',
    'xai-subscription-oauth',
    'grok-3',
    NULL,
    NULL,
    updated_at_unix_ms,
    updated_at_unix_ms
FROM provider_profiles
WHERE id = 'openai-chatgpt-compat'
ON CONFLICT(id) DO NOTHING;

-- Do not copy ChatGPT selected models (e.g. gpt-5.5) onto the xAI profile.
INSERT INTO provider_model_selection (provider_profile_id, selected_model_id, updated_at_unix_ms)
SELECT 'xai-subscription-oauth', 'grok-3', updated_at_unix_ms
FROM provider_profiles
WHERE id = 'openai-chatgpt-compat'
ON CONFLICT(provider_profile_id) DO NOTHING;

UPDATE provider_profiles
SET credential_handle = NULL, expires_at_unix_ms = NULL, updated_at_unix_ms = updated_at_unix_ms
WHERE id = 'openai-chatgpt-compat';
