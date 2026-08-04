CREATE TABLE agent_sources (
    id TEXT PRIMARY KEY NOT NULL,
    origin_kind TEXT NOT NULL,
    display_name TEXT NOT NULL,
    byte_count INTEGER NOT NULL,
    content_sha256 TEXT NOT NULL,
    content TEXT NOT NULL,
    created_at_unix_ms INTEGER NOT NULL
) STRICT;

CREATE TABLE agent_turn_sources (
    turn_id TEXT NOT NULL REFERENCES agent_turns(id),
    source_id TEXT NOT NULL REFERENCES agent_sources(id),
    attachment_order INTEGER NOT NULL,
    PRIMARY KEY (turn_id, source_id),
    UNIQUE (turn_id, attachment_order),
    UNIQUE (source_id)
) STRICT;

CREATE INDEX agent_turn_sources_turn_order_index
ON agent_turn_sources (turn_id, attachment_order ASC);
