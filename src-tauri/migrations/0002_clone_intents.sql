CREATE TABLE clone_intents(operation_id TEXT PRIMARY KEY REFERENCES operations(id), context TEXT NOT NULL);
