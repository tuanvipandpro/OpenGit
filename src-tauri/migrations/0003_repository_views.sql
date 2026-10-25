CREATE TABLE repository_views(
    repo_id TEXT PRIMARY KEY REFERENCES repositories(id) ON DELETE CASCADE,
    revision INTEGER NOT NULL CHECK(revision >= 0),
    value TEXT NOT NULL CHECK(typeof(value) = 'text' AND length(CAST(value AS BLOB)) <= 16384)
);
