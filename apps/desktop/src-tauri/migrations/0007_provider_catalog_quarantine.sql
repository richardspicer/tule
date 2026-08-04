CREATE TABLE provider_model_catalog_quarantine (
    provider_profile_id TEXT PRIMARY KEY NOT NULL REFERENCES provider_profiles(id)
) STRICT;
