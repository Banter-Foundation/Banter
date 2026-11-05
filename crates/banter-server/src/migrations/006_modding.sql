-- Bots, plugin installs, plugin-authored messages, and custom channel kinds.

ALTER TABLE users ADD COLUMN IF NOT EXISTS bot BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE users ALTER COLUMN password_hash DROP NOT NULL;

ALTER TABLE messages ALTER COLUMN author_id DROP NOT NULL;
ALTER TABLE messages ADD COLUMN IF NOT EXISTS author_kind TEXT NOT NULL DEFAULT 'user';
ALTER TABLE messages ADD COLUMN IF NOT EXISTS plugin_id TEXT;

ALTER TABLE channels ADD COLUMN IF NOT EXISTS created_by_plugin TEXT;

CREATE TABLE IF NOT EXISTS bots (
    user_id BIGINT PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    owner_id BIGINT NOT NULL REFERENCES users(id),
    intents TEXT NOT NULL DEFAULT '[]',
    interactions_url TEXT
);

CREATE TABLE IF NOT EXISTS bot_tokens (
    token TEXT PRIMARY KEY,
    user_id BIGINT NOT NULL UNIQUE REFERENCES users(id) ON DELETE CASCADE,
    created_at BIGINT NOT NULL
);

CREATE TABLE IF NOT EXISTS server_bots (
    server_id BIGINT NOT NULL REFERENCES servers(id) ON DELETE CASCADE,
    bot_user_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    accepted_intents TEXT NOT NULL DEFAULT '[]',
    PRIMARY KEY (server_id, bot_user_id)
);

CREATE TABLE IF NOT EXISTS bot_commands (
    bot_user_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    body TEXT NOT NULL,
    PRIMARY KEY (bot_user_id, name)
);

CREATE TABLE IF NOT EXISTS plugins (
    server_id BIGINT NOT NULL REFERENCES servers(id) ON DELETE CASCADE,
    plugin_id TEXT NOT NULL,
    name TEXT NOT NULL,
    version TEXT NOT NULL,
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    client TEXT NOT NULL,
    manifest TEXT NOT NULL,
    package_dir TEXT NOT NULL,
    hard_error TEXT,
    PRIMARY KEY (server_id, plugin_id)
);

CREATE TABLE IF NOT EXISTS plugin_storage (
    server_id BIGINT NOT NULL,
    plugin_id TEXT NOT NULL,
    key TEXT NOT NULL,
    value TEXT NOT NULL,
    PRIMARY KEY (server_id, plugin_id, key)
);

CREATE TABLE IF NOT EXISTS plugin_opt_in (
    server_id BIGINT NOT NULL,
    plugin_id TEXT NOT NULL,
    user_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    version TEXT NOT NULL,
    PRIMARY KEY (server_id, plugin_id, user_id)
);

CREATE TABLE IF NOT EXISTS plugin_sync (
    server_id BIGINT NOT NULL,
    plugin_id TEXT NOT NULL,
    channel_id BIGINT NOT NULL,
    state TEXT,
    PRIMARY KEY (server_id, plugin_id, channel_id)
);
