CREATE TABLE appearance_preference (
    id INTEGER PRIMARY KEY NOT NULL CHECK (id = 1),
    value TEXT NOT NULL CHECK (value IN ('system', 'light', 'dark'))
) STRICT;

INSERT INTO appearance_preference (id, value) VALUES (1, 'system');
