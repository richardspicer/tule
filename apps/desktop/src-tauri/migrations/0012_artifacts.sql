CREATE TABLE artifacts (
    id TEXT PRIMARY KEY NOT NULL,
    title TEXT NOT NULL,
    kind TEXT NOT NULL,
    project_id TEXT REFERENCES projects(id),
    created_at_unix_ms INTEGER NOT NULL
) STRICT;

CREATE TABLE artifact_versions (
    id TEXT PRIMARY KEY NOT NULL,
    artifact_id TEXT NOT NULL REFERENCES artifacts(id),
    version_ordinal INTEGER NOT NULL,
    content TEXT NOT NULL,
    content_sha256 TEXT NOT NULL,
    source_session_id TEXT NOT NULL REFERENCES agent_sessions(id),
    source_turn_id TEXT NOT NULL REFERENCES agent_turns(id),
    provider_profile_id TEXT NOT NULL REFERENCES provider_profiles(id),
    model_id TEXT NOT NULL,
    prompt_version TEXT NOT NULL,
    project_id TEXT REFERENCES projects(id),
    provider_request_id TEXT NOT NULL,
    created_at_unix_ms INTEGER NOT NULL,
    UNIQUE (artifact_id, version_ordinal)
) STRICT;

CREATE INDEX artifact_versions_source_session_index
ON artifact_versions (source_session_id);

CREATE INDEX artifacts_project_id_index
ON artifacts (project_id);

CREATE INDEX artifact_versions_artifact_ordinal_index
ON artifact_versions (artifact_id, version_ordinal ASC);
