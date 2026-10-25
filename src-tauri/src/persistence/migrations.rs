use rusqlite::Connection;
pub fn migrate(connection: &mut Connection) -> rusqlite::Result<()> {
    let version: i64 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if !(0..=6).contains(&version) {
        return Err(rusqlite::Error::InvalidQuery);
    }
    if version == 6 {
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
    if version < 3 {
        transaction.execute_batch(include_str!("../../migrations/0003_repository_views.sql"))?;
        transaction.pragma_update(None, "user_version", 3)?;
    }
    if version < 4 {
        transaction.execute_batch(include_str!("../../migrations/0004_repository_actions.sql"))?;
        transaction.pragma_update(None, "user_version", 4)?;
    }
    if version < 5 {
        transaction.execute_batch(include_str!("../../migrations/0005_fetch_observations.sql"))?;
    }
    transaction.execute_batch(include_str!(
        "../../migrations/0006_publication_observations.sql"
    ))?;
    transaction.pragma_update(None, "user_version", 6)?;
    transaction.commit()?;
    Ok(())
}
