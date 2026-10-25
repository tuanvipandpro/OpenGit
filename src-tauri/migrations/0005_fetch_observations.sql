-- Cache rows are expendable; operation evidence survives registry forget and
-- is deleted only with the existing safely compacted operation.
CREATE TABLE fetch_actions(
 operation_id TEXT PRIMARY KEY REFERENCES operations(id) ON DELETE CASCADE,
 reserved_bytes INTEGER NOT NULL CHECK(reserved_bytes >= 0 AND reserved_bytes <= 8388608),
 record TEXT CHECK(record IS NULL OR length(CAST(record AS BLOB)) <= 8387584)
);
CREATE TABLE fetch_observations(
 repo_id TEXT NOT NULL CHECK(length(CAST(repo_id AS BLOB)) <= 256),
 remote_name TEXT NOT NULL CHECK(length(CAST(remote_name AS BLOB)) <= 1024),
 ref_name TEXT NOT NULL CHECK(length(CAST(ref_name AS BLOB)) <= 1024),
 config_digest TEXT NOT NULL CHECK(length(config_digest) = 64),
 oid TEXT CHECK(oid IS NULL OR length(oid) IN (40,64)),
 observed_at TEXT NOT NULL CHECK(length(CAST(observed_at AS BLOB)) <= 64),
 weight INTEGER NOT NULL CHECK(weight > 0 AND weight <= 8192),
 PRIMARY KEY(repo_id, remote_name, ref_name)
);
CREATE INDEX fetch_observations_age ON fetch_observations(observed_at);
