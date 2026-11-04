-- Optional Lucide icon id on channels. Null means the client shows the kind
-- default (hash / volume-2 / pencil). Idempotent for volumes that already ran
-- a tip schema.sql that includes channels.icon.

ALTER TABLE channels ADD COLUMN IF NOT EXISTS icon TEXT;
