use super::runner::{argv, Cancellation, GitRunner};
use crate::domain::contracts::*;
use std::path::Path;
/// Remote configuration is untrusted. Never display credentials, query, fragment,
/// unsupported helpers or an unparseable remote value.
pub fn safe_remote(value: &str) -> Option<String> {
    if value.len() > 4096 || value.chars().any(char::is_control) {
        return None;
    }
    if value.contains("://") {
        let mut url = url::Url::parse(value).ok()?;
        if !matches!(url.scheme(), "https" | "ssh") || url.host_str().is_none() {
            return None;
        }
        url.set_username("").ok()?;
        url.set_password(None).ok()?;
        url.set_query(None);
        url.set_fragment(None);
        return Some(url.to_string());
    }
    if value.contains("://") || value.contains("::") {
        return None;
    }
    // SCP syntax has no URL parser to remove these fields for us. Apply the
    // same display policy before the string can reach UI or a journal.
    let value = value.split(['?', '#']).next()?;
    let (host, path) = value.split_once(':')?;
    let host = host.rsplit('@').next()?;
    if host.is_empty()
        || path.is_empty()
        || host.len() == 1
        || !host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".-_".contains(&b))
    {
        return None;
    }
    Some(format!("{host}:{path}"))
}
pub async fn remotes(runner: &GitRunner, cwd: &Path) -> Result<Vec<RemoteSummary>, AppError> {
    let bytes = runner
        .run(
            Some(cwd),
            &argv(&["remote"]),
            &Cancellation::default(),
            Phase::Inspection,
        )
        .await?;
    let names = String::from_utf8(bytes)
        .map_err(|_| super::error::error(ErrorCode::MalformedOutput, Phase::Inspection))?;
    let mut result = Vec::new();
    for name in names.lines() {
        if name.is_empty() || name.starts_with('-') || name.chars().any(char::is_control) {
            continue;
        }
        let mut urls = Vec::new();
        for push in [false, true] {
            let mut args = argv(&["remote", "get-url"]);
            if push {
                args.push("--push".into());
            }
            args.push("--".into());
            args.push(name.into());
            let url = runner
                .run(
                    Some(cwd),
                    &args,
                    &Cancellation::default(),
                    Phase::Inspection,
                )
                .await
                .ok()
                .and_then(|v| String::from_utf8(v).ok())
                .and_then(|s| safe_remote(s.trim_end_matches(['\r', '\n'])));
            urls.push(url);
        }
        result.push(RemoteSummary {
            name: name.into(),
            fetch_url_display: urls[0].clone(),
            push_url_display: urls[1].clone(),
        });
    }
    Ok(result)
}
