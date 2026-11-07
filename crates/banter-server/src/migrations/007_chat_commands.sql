-- Per-guild chat command prefix and aliases (BA-104).

ALTER TABLE servers ADD COLUMN IF NOT EXISTS command_prefix TEXT NOT NULL DEFAULT '/';

CREATE TABLE IF NOT EXISTS server_command_aliases (
    server_id BIGINT NOT NULL REFERENCES servers(id) ON DELETE CASCADE,
    alias TEXT NOT NULL,
    target TEXT NOT NULL,
    PRIMARY KEY (server_id, alias)
);

CREATE INDEX IF NOT EXISTS server_command_aliases_server_idx
    ON server_command_aliases (server_id);
