CREATE TABLE provider_profiles (
    id TEXT PRIMARY KEY NOT NULL,
    provider_kind TEXT NOT NULL,
    visible_model_id TEXT NOT NULL,
    credential_handle TEXT,
    expires_at_unix_ms INTEGER,
    created_at_unix_ms INTEGER NOT NULL,
    updated_at_unix_ms INTEGER NOT NULL
) STRICT;

CREATE TABLE agent_sessions (
    id TEXT PRIMARY KEY NOT NULL,
    title TEXT NOT NULL,
    project_id TEXT REFERENCES projects(id),
    provider_profile_id TEXT NOT NULL REFERENCES provider_profiles(id),
    model_id TEXT NOT NULL,
    created_at_unix_ms INTEGER NOT NULL,
    updated_at_unix_ms INTEGER NOT NULL
) STRICT;

CREATE TABLE agent_turns (
    id TEXT PRIMARY KEY NOT NULL,
    session_id TEXT NOT NULL REFERENCES agent_sessions(id),
    ordinal INTEGER NOT NULL,
    user_text TEXT NOT NULL,
    agent_text TEXT NOT NULL,
    state TEXT NOT NULL,
    error_code TEXT,
    provider_profile_id TEXT NOT NULL REFERENCES provider_profiles(id),
    model_id TEXT NOT NULL,
    provider_request_id TEXT NOT NULL,
    provider_response_id TEXT,
    usage_input_tokens INTEGER,
    usage_output_tokens INTEGER,
    project_id TEXT REFERENCES projects(id),
    project_instructions TEXT NOT NULL,
    prompt_version TEXT NOT NULL,
    started_at_unix_ms INTEGER NOT NULL,
    finished_at_unix_ms INTEGER,
    UNIQUE (session_id, ordinal)
) STRICT;

CREATE TABLE agent_events (
    id TEXT PRIMARY KEY NOT NULL,
    session_id TEXT NOT NULL REFERENCES agent_sessions(id),
    turn_id TEXT REFERENCES agent_turns(id),
    sequence INTEGER NOT NULL,
    kind TEXT NOT NULL,
    created_at_unix_ms INTEGER NOT NULL,
    UNIQUE (session_id, sequence)
) STRICT;

CREATE INDEX agent_sessions_updated_at_index
ON agent_sessions (updated_at_unix_ms DESC, id DESC);
CREATE INDEX agent_turns_session_ordinal_index
ON agent_turns (session_id, ordinal ASC);
CREATE INDEX agent_events_session_sequence_index
ON agent_events (session_id, sequence ASC);
