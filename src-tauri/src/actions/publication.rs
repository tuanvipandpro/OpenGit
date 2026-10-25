//! Bounded, dated server acknowledgements. Never remote-current authority.
use crate::{domain::contracts::*, persistence::workspace::db_error};
pub(crate) const ROWS: usize = 4096;
pub(crate) const BYTES: usize = 8 * 1024 * 1024;
pub(crate) fn validate(p: &PublicationEvidence) -> Result<(), AppError> {
    let oid = |v: &str| matches!(v.len(), 40 | 64) && v.bytes().all(|b| b.is_ascii_hexdigit());
    if p.remote_name.len() > 256
        || !super::fetch::remote_name(&p.remote_name)
        || p.role != RemoteEndpointRole::Push
        || p.destination.len() > 1024
        || !crate::history::refs::valid_name(&p.destination)
        || !["refs/heads/", "refs/tags/"]
            .iter()
            .any(|v| p.destination.starts_with(v))
        || p.source_ref.as_ref().is_some_and(|s| {
            s.len() > 1024
                || !crate::history::refs::valid_name(s)
                || !["refs/heads/", "refs/tags/"]
                    .iter()
                    .any(|v| s.starts_with(v))
        })
        || p.source_oid.as_ref().is_some_and(|v| !oid(v))
        || p.acknowledged_oid.as_ref().is_some_and(|v| !oid(v))
        || p.config_digest.len() != 64
        || !p.config_digest.bytes().all(|b| b.is_ascii_hexdigit())
        || p.acknowledged_at.len() > 64
        || chrono::DateTime::parse_from_rfc3339(&p.acknowledged_at).is_err()
        || p.deleted != p.acknowledged_oid.is_none()
    {
        return Err(db_error());
    }
    Ok(())
}
pub(crate) fn write_to(
    c: &rusqlite::Connection,
    id: &str,
    repo: &str,
    p: &PublicationEvidence,
) -> Result<(), AppError> {
    validate(p)?;
    let text = serde_json::to_string(p).map_err(|_| db_error())?;
    let charge = text.len() + 512;
    if charge > BYTES {
        return Err(db_error());
    }
    c.execute("DELETE FROM publication_observations WHERE id=?1", [id])
        .map_err(|_| db_error())?;
    loop {
        let (rows, bytes): (i64, i64) = c
            .query_row(
                "SELECT count(*),coalesce(sum(charged_bytes),0) FROM publication_observations",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|_| db_error())?;
        if rows < 0 || bytes < 0 {
            return Err(db_error());
        }
        if rows < ROWS as i64 && bytes + charge as i64 <= BYTES as i64 {
            break;
        }
        if c.execute("DELETE FROM publication_observations WHERE id=(SELECT id FROM publication_observations ORDER BY observed_at,id LIMIT 1)",[]).map_err(|_|db_error())?==0{return Err(db_error());}
    }
    c.execute("INSERT INTO publication_observations(id,repo_id,source_ref,source_oid,observed_at,charged_bytes,record) VALUES(?1,?2,?3,?4,?5,?6,?7)",rusqlite::params![id,repo,p.source_ref,p.source_oid,p.acknowledged_at,charge as i64,text]).map_err(|_|db_error())?;
    Ok(())
}
pub(crate) fn read(
    c: &rusqlite::Connection,
    repo: &str,
    name: &str,
    oid: &str,
) -> Result<Vec<PublicationEvidence>, AppError> {
    let mut s=c.prepare("SELECT record FROM publication_observations WHERE repo_id=?1 AND source_ref=?2 AND source_oid=?3 ORDER BY observed_at DESC,id LIMIT 4096").map_err(|_|db_error())?;
    let rows = s
        .query_map(rusqlite::params![repo, name, oid], |r| {
            r.get::<_, String>(0)
        })
        .map_err(|_| db_error())?;
    let mut result = Vec::new();
    let mut bytes = 0usize;
    for row in rows {
        let row = row.map_err(|_| db_error())?;
        bytes += row.len() + 512;
        if bytes > BYTES {
            return Err(db_error());
        }
        let p = serde_json::from_str(&row).map_err(|_| db_error())?;
        validate(&p)?;
        result.push(p);
    }
    Ok(result)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn evidence(long: bool) -> PublicationEvidence {
        PublicationEvidence {
            remote_name: if long {
                "r".repeat(256)
            } else {
                "origin".into()
            },
            role: RemoteEndpointRole::Push,
            destination: if long {
                format!("refs/heads/{}", "b".repeat(1013))
            } else {
                "refs/heads/main".into()
            },
            source_ref: Some(if long {
                format!("refs/heads/{}", "a".repeat(1013))
            } else {
                "refs/heads/main".into()
            }),
            source_oid: Some("a".repeat(40)),
            acknowledged_oid: Some("a".repeat(40)),
            acknowledged_at: "2026-10-26T00:00:00Z".into(),
            config_digest: "c".repeat(64),
            deleted: false,
        }
    }
    #[test]
    fn publication_rows_and_bytes_evict_to_unknown_and_insert_failure_rolls_back() {
        let mut c = rusqlite::Connection::open_in_memory().unwrap();
        crate::persistence::migrations::migrate(&mut c).unwrap();
        {
            let tx = c.transaction().unwrap();
            for i in 0..ROWS + 2 {
                write_to(&tx, &format!("{i:08}"), "repo", &evidence(false)).unwrap();
            }
            tx.commit().unwrap();
        }
        assert_eq!(
            c.query_row("SELECT count(*) FROM publication_observations", [], |r| r
                .get::<_, i64>(
                0
            ))
            .unwrap(),
            ROWS as i64
        );
        assert_eq!(
            c.query_row(
                "SELECT count(*) FROM publication_observations WHERE id='00000000'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        {
            let tx = c.transaction().unwrap();
            for i in 0..ROWS {
                write_to(&tx, &format!("long-{i:08}"), "repo", &evidence(true)).unwrap();
            }
            tx.commit().unwrap();
        }
        let before: (i64, i64) = c
            .query_row(
                "SELECT count(*),sum(charged_bytes) FROM publication_observations",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert!(before.0 < ROWS as i64 && before.1 <= BYTES as i64);
        c.execute_batch("CREATE TRIGGER reject_publication BEFORE INSERT ON publication_observations WHEN NEW.id='fault' BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
        {
            let tx = c.transaction().unwrap();
            assert!(write_to(&tx, "fault", "repo", &evidence(true)).is_err());
        }
        let after: (i64, i64) = c
            .query_row(
                "SELECT count(*),sum(charged_bytes) FROM publication_observations",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(before, after);
        let p = evidence(true);
        assert!(!read(
            &c,
            "repo",
            p.source_ref.as_ref().unwrap(),
            p.source_oid.as_ref().unwrap()
        )
        .unwrap()
        .is_empty());
        assert!(read(
            &c,
            "other",
            p.source_ref.as_ref().unwrap(),
            p.source_oid.as_ref().unwrap()
        )
        .unwrap()
        .is_empty());
        let mut bad = evidence(false);
        bad.remote_name = "https://token@example.invalid".into();
        assert!(validate(&bad).is_err());
    }
    #[test]
    fn schema_six_preserves_zero_through_five_and_failure_rolls_back_all_new_tables() {
        for version in 0..=5 {
            for fail in [false, true] {
                let mut c = rusqlite::Connection::open_in_memory().unwrap();
                if version >= 1 {
                    c.execute_batch(include_str!("../../migrations/0001_initial.sql"))
                        .unwrap();
                    c.execute_batch("INSERT INTO workspace(id,revision) VALUES('sentinel',13); INSERT INTO operations VALUES('op','operation-sentinel');").unwrap();
                }
                if version >= 2 {
                    c.execute_batch(include_str!("../../migrations/0002_clone_intents.sql"))
                        .unwrap();
                }
                if version >= 3 {
                    c.execute_batch(include_str!("../../migrations/0003_repository_views.sql"))
                        .unwrap();
                }
                if version >= 4 {
                    c.execute_batch(include_str!("../../migrations/0004_repository_actions.sql"))
                        .unwrap();
                    c.execute(
                        "INSERT INTO repository_actions VALUES('op','action-sentinel')",
                        [],
                    )
                    .unwrap();
                }
                if version >= 5 {
                    c.execute_batch(include_str!("../../migrations/0005_fetch_observations.sql"))
                        .unwrap();
                    c.execute(
                        "INSERT INTO fetch_actions VALUES('op',4096,'fetch-sentinel')",
                        [],
                    )
                    .unwrap();
                }
                if fail {
                    c.execute_batch("CREATE TABLE publication_observations(sentinel TEXT); INSERT INTO publication_observations VALUES('preserve');").unwrap();
                }
                c.pragma_update(None, "user_version", version).unwrap();
                assert_eq!(
                    crate::persistence::migrations::migrate(&mut c).is_err(),
                    fail
                );
                assert_eq!(
                    c.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
                        .unwrap(),
                    if fail { version } else { 6 }
                );
                if version >= 1 {
                    assert_eq!(
                        c.query_row("SELECT revision FROM workspace", [], |r| r.get::<_, i64>(0))
                            .unwrap(),
                        13
                    );
                }
                if version >= 4 {
                    assert_eq!(
                        c.query_row("SELECT record FROM repository_actions", [], |r| r
                            .get::<_, String>(0))
                            .unwrap(),
                        "action-sentinel"
                    );
                }
                if version >= 5 {
                    assert_eq!(
                        c.query_row("SELECT record FROM fetch_actions", [], |r| r
                            .get::<_, String>(0))
                            .unwrap(),
                        "fetch-sentinel"
                    );
                }
                if fail {
                    assert_eq!(
                        c.query_row("SELECT sentinel FROM publication_observations", [], |r| {
                            r.get::<_, String>(0)
                        })
                        .unwrap(),
                        "preserve"
                    );
                }
            }
        }
    }
}
