use crate::{
    domain::contracts::*,
    git::{error::error, refs::safe_remote},
};
#[derive(Debug, PartialEq, Eq)]
pub enum RemoteTransport {
    Https,
    Ssh,
}
pub fn remote_transport(value: &str) -> Result<RemoteTransport, AppError> {
    remote(value)?;
    if value.contains("://") {
        match url::Url::parse(value)
            .map_err(|_| error(ErrorCode::InvalidSource, Phase::Validation))?
            .scheme()
        {
            "https" => Ok(RemoteTransport::Https),
            "ssh" => Ok(RemoteTransport::Ssh),
            _ => Err(error(ErrorCode::InvalidSource, Phase::Validation)),
        }
    } else {
        Ok(RemoteTransport::Ssh)
    }
}
pub fn directory_name(value: &str) -> Result<(), AppError> {
    let units = value.encode_utf16().count();
    let stem = value
        .split('.')
        .next()
        .unwrap_or("")
        .trim_end_matches(' ')
        .to_ascii_uppercase();
    let serial = stem
        .strip_prefix("COM")
        .or_else(|| stem.strip_prefix("LPT"));
    let device = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CLOCK$")
        || serial.is_some_and(|part| {
            matches!(
                part,
                "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
            )
        });
    if units == 0
        || units > 200
        || matches!(value, "." | "..")
        || value.ends_with(['.', ' '])
        || value
            .chars()
            .any(|c| c.is_control() || "/\\:<>\"|?*".contains(c))
        || device
    {
        return Err(error(ErrorCode::UnsafePath, Phase::Validation));
    }
    Ok(())
}
pub fn remote(value: &str) -> Result<String, AppError> {
    let invalid = || error(ErrorCode::InvalidSource, Phase::Validation);
    if value.is_empty()
        || value.len() > 4096
        || value.starts_with('-')
        || value.chars().any(char::is_control)
        || value.contains(['?', '#', '\\'])
    {
        return Err(invalid());
    }
    if value.contains("://") {
        let url = url::Url::parse(value).map_err(|_| invalid())?;
        if !matches!(url.scheme(), "https" | "ssh")
            || url.host_str().is_none()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || (url.scheme() == "https" && !url.username().is_empty())
        {
            return Err(invalid());
        }
        if url.scheme() == "ssh"
            && (!valid_user(url.username())
                || url.host_str().is_some_and(|host| host.starts_with('-')))
        {
            return Err(invalid());
        }
    } else {
        if value.contains("::") || value.contains('\\') {
            return Err(invalid());
        }
        let (host, path) = value.split_once(':').ok_or_else(invalid)?;
        let host = if let Some((user, host)) = host.split_once('@') {
            if !valid_user(user) || user.is_empty() {
                return Err(invalid());
            }
            host
        } else {
            host
        };
        if host.len() < 2
            || host.starts_with('-')
            || path.is_empty()
            || !host
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b".-_".contains(&b))
        {
            return Err(invalid());
        }
    }
    safe_remote(value).ok_or_else(invalid)
}
fn valid_user(value: &str) -> bool {
    !value.starts_with('-')
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
}
