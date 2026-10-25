use super::{
    query::{summary, Query},
    sessions::{Cursor, Session},
    *,
};

pub(crate) async fn search(
    query: &Query<'_>,
    session: &mut Session,
    literal: &str,
    start: usize,
    size: usize,
    graph_fence: usize,
    first: bool,
) -> Result<HistorySearchPage, AppError> {
    if start > session.all_order.len() {
        return Err(invalid_handle());
    }
    let needle = literal.to_lowercase();
    let mut commits = Vec::new();
    let mut index = start;
    let mut next = None;
    // Read complete raw messages in bounded batches, independent of graph pages.
    while index < session.all_order.len() {
        let batch = query.search_batch(&session.all_order[index..]).await?;
        for raw in batch {
            let position = index;
            index += 1;
            if raw.oid.starts_with(&needle)
                || raw.message.to_lowercase().contains(&needle)
                || raw.author.name.to_lowercase().contains(&needle)
                || raw.author.email.to_lowercase().contains(&needle)
                || format!("{} <{}>", raw.author.name, raw.author.email)
                    .to_lowercase()
                    .contains(&needle)
            {
                if commits.len() == size {
                    next = Some(position);
                    break;
                }
                let outside_view = !session.positions.contains_key(&raw.oid);
                commits.push(HistorySearchHit {
                    commit: summary(session, raw, graph_fence)?,
                    outside_view,
                });
            }
        }
        if next.is_some() {
            break;
        }
    }
    let next_cursor = next
        .map(|start| {
            session.cursor(Cursor::Search {
                start,
                size,
                query: literal.into(),
                graph_fence,
            })
        })
        .transpose()?;
    let refs = if first {
        session
            .refs
            .iter()
            .filter(|r| r.name.to_lowercase().contains(&needle))
            .cloned()
            .collect()
    } else {
        Vec::new()
    };
    Ok(HistorySearchPage {
        context: session.context.clone(),
        session_id: session.id.clone(),
        query: literal.into(),
        commits,
        refs,
        next_cursor,
    })
}
