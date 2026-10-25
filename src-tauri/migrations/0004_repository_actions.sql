CREATE TABLE repository_actions(operation_id TEXT PRIMARY KEY REFERENCES operations(id), record TEXT NOT NULL);
