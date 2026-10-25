use crate::{
    domain::contracts::*,
    persistence::workspace::{db_error, Database},
};
pub fn write(db: &Database, snapshot: &OperationSnapshot) -> Result<(), AppError> {
    let connection = db.lock()?;
    write_to(&connection, snapshot)
}
pub(crate) fn write_to(
    connection: &rusqlite::Connection,
    snapshot: &OperationSnapshot,
) -> Result<(), AppError> {
    // Prompt/response are transient. Error text is always regenerated from code.
    let mut safe = snapshot.clone();
    safe.auth_prompt = None;
    if let Some(e) = &mut safe.error {
        *e = if safe.kind == OperationSnapshotKind::Action {
            crate::actions::safe_error(e)
        } else {
            crate::git::error::sanitized_error(e)
        };
    }
    let serialized = serde_json::to_string(&safe).map_err(|_| db_error())?;
    if serialized.len() > 32768 {
        return Err(db_error());
    }
    connection.execute("INSERT INTO operations(id,snapshot) VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET snapshot=excluded.snapshot",rusqlite::params![safe.id,serialized]).map_err(|_|db_error())?;
    Ok(())
}
pub fn load(db: &Database) -> Result<Vec<OperationSnapshot>, AppError> {
    let connection = db.lock()?;
    let mut statement = connection
        .prepare("SELECT snapshot FROM operations ORDER BY id")
        .map_err(|_| db_error())?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|_| db_error())?;
    let mut snapshots = Vec::new();
    for row in rows {
        let text = row.map_err(|_| db_error())?;
        if text.len() > 32768 {
            return Err(db_error());
        }
        let mut snapshot: OperationSnapshot =
            serde_json::from_str(&text).map_err(|_| db_error())?;
        snapshot.auth_prompt = None;
        if let Some(e) = &mut snapshot.error {
            *e = if snapshot.kind == OperationSnapshotKind::Action {
                crate::actions::safe_error(e)
            } else {
                crate::git::error::sanitized_error(e)
            };
        }
        snapshots.push(snapshot);
    }
    Ok(snapshots)
}
