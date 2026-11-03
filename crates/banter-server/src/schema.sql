PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS users (
    id INTEGER PRIMARY KEY,
    username TEXT NOT NULL UNIQUE,
    display_name TEXT NOT NULL,
    password_hash TEXT NOT NULL,
    avatar_file TEXT,
    created_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS sessions (
    token TEXT PRIMARY KEY,
    user_id INTEGER NOT NULL REFERENCES users(id),
    created_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS friend_requests (
    id INTEGER PRIMARY KEY,
    from_user INTEGER NOT NULL REFERENCES users(id),
    to_user INTEGER NOT NULL REFERENCES users(id),
    created_at INTEGER NOT NULL,
    UNIQUE(from_user, to_user)
);

CREATE TABLE IF NOT EXISTS friendships (
    user_a INTEGER NOT NULL REFERENCES users(id),
    user_b INTEGER NOT NULL REFERENCES users(id),
    created_at INTEGER NOT NULL,
    PRIMARY KEY (user_a, user_b),
    CHECK (user_a < user_b)
);

CREATE TABLE IF NOT EXISTS servers (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    owner_id INTEGER NOT NULL REFERENCES users(id),
    created_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS invites (
    code TEXT PRIMARY KEY,
    server_id INTEGER NOT NULL REFERENCES servers(id),
    created_by INTEGER NOT NULL REFERENCES users(id),
    created_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS memberships (
    server_id INTEGER NOT NULL REFERENCES servers(id),
    user_id INTEGER NOT NULL REFERENCES users(id),
    nickname TEXT,
    PRIMARY KEY (server_id, user_id)
);

CREATE TABLE IF NOT EXISTS categories (
    id INTEGER PRIMARY KEY,
    server_id INTEGER NOT NULL REFERENCES servers(id),
    name TEXT NOT NULL,
    position INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS channels (
    id INTEGER PRIMARY KEY,
    server_id INTEGER NOT NULL REFERENCES servers(id),
    category_id INTEGER NOT NULL REFERENCES categories(id),
    name TEXT NOT NULL,
    kind TEXT NOT NULL,
    position INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS direct_messages (
    id INTEGER PRIMARY KEY,
    user_a INTEGER NOT NULL REFERENCES users(id),
    user_b INTEGER NOT NULL REFERENCES users(id),
    created_at INTEGER NOT NULL,
    UNIQUE(user_a, user_b),
    CHECK (user_a < user_b)
);

CREATE TABLE IF NOT EXISTS messages (
    id INTEGER PRIMARY KEY,
    channel_id INTEGER REFERENCES channels(id),
    dm_id INTEGER REFERENCES direct_messages(id),
    author_id INTEGER NOT NULL REFERENCES users(id),
    body TEXT NOT NULL,
    reply_to INTEGER REFERENCES messages(id),
    created_at INTEGER NOT NULL,
    edited_at INTEGER,
    CHECK ((channel_id IS NULL) != (dm_id IS NULL))
);

CREATE TABLE IF NOT EXISTS attachments (
    id INTEGER PRIMARY KEY,
    message_id INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    filename TEXT NOT NULL,
    byte_size INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS mentions (
    id INTEGER PRIMARY KEY,
    message_id INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    user_id INTEGER NOT NULL REFERENCES users(id),
    read_at INTEGER
);

CREATE INDEX IF NOT EXISTS idx_messages_channel ON messages(channel_id, id);
CREATE INDEX IF NOT EXISTS idx_messages_dm ON messages(dm_id, id);
CREATE INDEX IF NOT EXISTS idx_mentions_user ON mentions(user_id, read_at);
CREATE INDEX IF NOT EXISTS idx_memberships_user ON memberships(user_id);
CREATE INDEX IF NOT EXISTS idx_sessions_user ON sessions(user_id);
