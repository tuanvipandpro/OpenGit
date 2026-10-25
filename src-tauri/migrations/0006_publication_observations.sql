CREATE TABLE publication_observations (
  id TEXT PRIMARY KEY,
  repo_id TEXT NOT NULL,
  source_ref TEXT,
  source_oid TEXT,
  observed_at TEXT NOT NULL,
  charged_bytes INTEGER NOT NULL CHECK(charged_bytes > 0 AND charged_bytes <= 8388608),
  record TEXT NOT NULL
);
CREATE INDEX publication_source ON publication_observations(repo_id,source_ref,source_oid,observed_at);
