CREATE TABLE provider_model_selection (
    provider_profile_id TEXT PRIMARY KEY NOT NULL REFERENCES provider_profiles(id),
    selected_model_id TEXT,
    updated_at_unix_ms INTEGER NOT NULL
) STRICT;

CREATE TABLE provider_model_catalog_state (
    provider_profile_id TEXT PRIMARY KEY NOT NULL REFERENCES provider_profiles(id),
    credential_generation INTEGER NOT NULL,
    compatibility_revision TEXT NOT NULL,
    etag TEXT,
    retrieved_at_unix_ms INTEGER NOT NULL,
    updated_at_unix_ms INTEGER NOT NULL
) STRICT;

CREATE TABLE provider_model_catalog_entries (
    provider_profile_id TEXT NOT NULL REFERENCES provider_profiles(id),
    credential_generation INTEGER NOT NULL,
    model_id TEXT NOT NULL,
    display_name TEXT NOT NULL,
    description TEXT,
    sort_order INTEGER NOT NULL,
    is_provider_default INTEGER NOT NULL CHECK (is_provider_default IN (0, 1)),
    PRIMARY KEY (provider_profile_id, credential_generation, model_id)
) STRICT;

CREATE INDEX provider_model_catalog_entries_order_index
ON provider_model_catalog_entries (
    provider_profile_id,
    credential_generation,
    sort_order ASC,
    model_id ASC
);

INSERT INTO provider_model_selection (provider_profile_id, selected_model_id, updated_at_unix_ms)
SELECT id, 'gpt-5.5', updated_at_unix_ms
FROM provider_profiles
WHERE id = 'openai-chatgpt-compat';
