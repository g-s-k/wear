CREATE TABLE IF NOT EXISTS garments (
  id          INTEGER PRIMARY KEY NOT NULL,
  name        TEXT NOT NULL,
  description TEXT NOT NULL DEFAULT '',
  color       TEXT NOT NULL,
  tags        TEXT NOT NULL DEFAULT '',
  count       INTEGER NOT NULL DEFAULT 0,
  last        TEXT
);
