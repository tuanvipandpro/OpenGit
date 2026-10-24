use crate::{domain::contracts::*, git::error::error};
pub fn invalid() -> AppError {
    error(ErrorCode::InvalidHandle, Phase::Authentication)
}
/// Untrusted Git/OpenSSH prompt text is classified, never forwarded wholesale.
pub fn classify(raw: &str) -> Result<(AuthPromptKind, String), AppError> {
    if raw.len() > 8192 || raw.contains('\0') {
        return Err(invalid());
    }
    let lower = raw.to_ascii_lowercase();
    if lower.contains("are you sure you want to continue connecting") {
        let fingerprint = raw
            .split_whitespace()
            .find(|part| part.starts_with("SHA256:"))
            .map(|s| s.trim_end_matches('.'))
            .filter(|value| {
                value.len() >= 20
                    && value.len() <= 100
                    && value[7..]
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"+/=".contains(&b))
            })
            .ok_or_else(invalid)?;
        let host = raw
            .split("host '")
            .nth(1)
            .and_then(|s| s.split('\'').next())
            .filter(|s| {
                !s.is_empty()
                    && s.len() <= 512
                    && s.bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b".-_:[](), ".contains(&b))
            })
            .ok_or_else(invalid)?;
        return Ok((
            AuthPromptKind::Confirmation,
            format!("Verify SSH host {host}. Fingerprint {fingerprint}. Trust this host?"),
        ));
    }
    if lower.contains("passphrase") {
        return Ok((
            AuthPromptKind::Passphrase,
            "Enter the passphrase for your SSH key.".into(),
        ));
    }
    if lower.contains("password") {
        return Ok((
            AuthPromptKind::Password,
            "Enter the repository password or access token.".into(),
        ));
    }
    if lower.starts_with("username for ") {
        return Ok((
            AuthPromptKind::Username,
            "Enter the repository username.".into(),
        ));
    }
    Err(error(
        ErrorCode::AuthenticationRequired,
        Phase::Authentication,
    ))
}
pub fn validate_value(kind: &AuthPromptKind, value: &str) -> Result<(), AppError> {
    if value.len() > 8192
        || value.contains(['\0', '\r', '\n'])
        || (*kind == AuthPromptKind::Confirmation && !matches!(value, "yes" | "no"))
    {
        return Err(error(ErrorCode::InvalidRequest, Phase::Authentication));
    }
    Ok(())
}
