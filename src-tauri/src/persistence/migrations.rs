use rusqlite::Connection;
pub fn migrate(connection: &mut Connection) -> rusqlite::Result<()> {
    let version: i64 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if !(0..=3).contains(&version) {
        return Err(rusqlite::Error::InvalidQuery);
    }
    if version == 3 {
        return Ok(());
    }
    let transaction = connection.transaction()?;
    if version == 0 {
        transaction.execute_batch(include_str!("../../migrations/0001_initial.sql"))?;
        transaction.execute(
            "INSERT INTO workspace(id) VALUES(?1)",
            [uuid::Uuid::new_v4().to_string()],
        )?;
        transaction.pragma_update(None, "user_version", 1)?;
    }
    if version < 2 {
        transaction.execute_batch(include_str!("../../migrations/0002_clone_intents.sql"))?;
        transaction.pragma_update(None, "user_version", 2)?;
    }
    transaction.execute_batch(include_str!("../../migrations/0003_repository_views.sql"))?;
    transaction.pragma_update(None, "user_version", 3)?;
    transaction.commit()?;
    Ok(())
}
