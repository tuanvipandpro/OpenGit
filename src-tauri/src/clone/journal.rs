use crate::{
    domain::contracts::AppError,
    persistence::workspace::{db_error, Database},
};
use rusqlite::OptionalExtension;
pub fn retained(db: &Database, id: &str) -> Result<bool, AppError> {
    let connection = db.lock()?;
    let text: Option<String> = connection
        .query_row(
            "SELECT context FROM clone_intents WHERE operation_id=?1",
            [id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|_| db_error())?;
    let Some(text) = text else { return Ok(false) };
    if text.len() > 65536 {
        return Err(db_error());
    }
    let record: CloneRecord = serde_json::from_str(&text).map_err(|_| db_error())?;
    Ok(record.cleanup.retained())
}
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum ClonePhase {
    Preparing,
    Staged,
    PublishIntent,
    Registered,
    Aborted,
}
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize, Default)]
pub enum CleanupDisposition {
    #[default]
    NotAttempted,
    AlreadyAbsent,
    Removed,
    Retained(CleanupReason),
}
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum CleanupReason {
    NonEmpty,
    OwnershipUnproven,
    IdentityChanged,
    IoFailure,
    UnexpectedEntries,
}
impl CleanupDisposition {
    pub fn retained(&self) -> bool {
        matches!(self, Self::Retained(_))
    }
}
/// Only validated path identities and ownership evidence are durable. Source
/// transport URL, auth prompts/responses and subprocess output are absent.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CloneRecord {
    pub operation_id: String,
    pub parent: Vec<u8>,
    pub parent_identity: Vec<u8>,
    pub leaf: String,
    pub container: String,
    pub container_identity: Option<Vec<u8>>,
    pub nonce: String,
    pub repository_identity: Option<Vec<u8>>,
    pub phase: ClonePhase,
    #[serde(default)]
    pub marker_identity: Option<Vec<u8>>,
    #[serde(default)]
    pub cleanup: CleanupDisposition,
}
pub fn write(db: &Database, record: &CloneRecord) -> Result<(), AppError> {
    let connection = db.lock()?;
    write_to(&connection, record)
}
pub(crate) fn write_to(
    connection: &rusqlite::Connection,
    record: &CloneRecord,
) -> Result<(), AppError> {
    let text = serde_json::to_string(record).map_err(|_| db_error())?;
    if text.len() > 65536 {
        return Err(db_error());
    }
    connection.execute("INSERT INTO clone_intents(operation_id,context) VALUES(?1,?2) ON CONFLICT(operation_id) DO UPDATE SET context=excluded.context",rusqlite::params![record.operation_id,text]).map_err(|_|db_error())?;
    Ok(())
}
pub fn load(db: &Database) -> Result<Vec<CloneRecord>, AppError> {
    let connection = db.lock()?;
    let mut statement = connection
        .prepare("SELECT context FROM clone_intents ORDER BY operation_id")
        .map_err(|_| db_error())?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|_| db_error())?;
    let mut result = Vec::new();
    for row in rows {
        let text = row.map_err(|_| db_error())?;
        if text.len() > 65536 {
            return Err(db_error());
        }
        result.push(serde_json::from_str(&text).map_err(|_| db_error())?);
    }
    Ok(result)
}
