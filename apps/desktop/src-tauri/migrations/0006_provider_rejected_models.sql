CREATE TABLE provider_rejected_models (
    provider_profile_id TEXT NOT NULL REFERENCES provider_profiles(id),
    model_id TEXT NOT NULL,
    credential_generation INTEGER NOT NULL,
    rejected_at_unix_ms INTEGER NOT NULL,
    PRIMARY KEY (provider_profile_id, model_id)
) STRICT;

CREATE INDEX provider_rejected_models_generation_index
ON provider_rejected_models (
    provider_profile_id,
    credential_generation
);
