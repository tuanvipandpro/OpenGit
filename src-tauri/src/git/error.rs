use crate::domain::contracts::*;
pub const RETENTION_NOTICE:&str="Temporary clone files were retained in a .opengit-stage-* folder beside the destination. Inspect them before manual removal.";
pub const SSH_PROMPT_NOTICE:&str="SSH prompts are unavailable from this installation path. Install OpenGit in an ASCII path or use a configured agent with a trusted host.";
pub fn retained_error(primary: Option<AppError>) -> AppError {
    let mut safe = primary
        .map(|value| sanitized_error(&value))
        .unwrap_or_else(|| error(ErrorCode::Io, Phase::Recovery));
    if !safe.message.ends_with(RETENTION_NOTICE) {
        safe.message.push(' ');
        safe.message.push_str(RETENTION_NOTICE);
    }
    safe
}
pub fn sanitized_error(value: &AppError) -> AppError {
    let mut tail = value.message.as_str();
    let (mut retained, mut ssh) = (false, false);
    loop {
        if let Some(prefix) = tail.strip_suffix(RETENTION_NOTICE) {
            retained = true;
            tail = prefix.trim_end();
        } else if let Some(prefix) = tail.strip_suffix(SSH_PROMPT_NOTICE) {
            ssh = true;
            tail = prefix.trim_end();
        } else {
            break;
        }
    }
    let mut safe = error(value.code.clone(), value.phase.clone());
    if ssh {
        safe.message.push(' ');
        safe.message.push_str(SSH_PROMPT_NOTICE);
    }
    if retained && !safe.message.ends_with(RETENTION_NOTICE) {
        safe.message.push(' ');
        safe.message.push_str(RETENTION_NOTICE);
    }
    safe
}
pub fn ssh_prompt_unavailable(value: AppError) -> AppError {
    if !matches!(
        value.code,
        ErrorCode::SshHostKey
            | ErrorCode::SshAuthentication
            | ErrorCode::AuthenticationRequired
            | ErrorCode::AuthenticationFailed
    ) {
        return value;
    }
    let mut safe = sanitized_error(&value);
    if !safe.message.contains(SSH_PROMPT_NOTICE) {
        safe.message.push(' ');
        safe.message.push_str(SSH_PROMPT_NOTICE);
    }
    sanitized_error(&safe)
}

/// Messages are static templates: never format subprocess output into a sink.
pub fn error(code: ErrorCode, phase: Phase) -> AppError {
    let (message, retry) = match code {
        ErrorCode::GitUnavailable => ("Git executable is unavailable.", AppErrorRetry::Retry),
        ErrorCode::NotRepository => ("Choose a Git repository.", AppErrorRetry::ReopenRepository),
        ErrorCode::RepositoryUnavailable | ErrorCode::ExternalChange => (
            "Repository changed or is unavailable. Reopen it.",
            AppErrorRetry::ReopenRepository,
        ),
        ErrorCode::Database if phase==Phase::Recovery => ("Clone data may already be published. Restart OpenGit to verify the destination and restore the workspace.",AppErrorRetry::None),
        ErrorCode::Database => (
            "Workspace database could not be read or updated.",
            AppErrorRetry::None,
        ),
        ErrorCode::Timeout => ("Git exceeded the time limit.", AppErrorRetry::Retry),
        ErrorCode::OutputLimit => ("Git output exceeded the safety limit.", AppErrorRetry::None),
        ErrorCode::Interrupted => ("Operation was interrupted.", AppErrorRetry::None),
        ErrorCode::InvalidHandle => (
            "Selection handle is invalid or expired.",
            AppErrorRetry::None,
        ),
        ErrorCode::InvalidRequest => ("Request is invalid.", AppErrorRetry::None),
        ErrorCode::DestinationExists => ("The destination already exists. Choose another folder name.",AppErrorRetry::ChooseAnotherDestination),
        ErrorCode::DestinationBusy => ("Another clone is using this destination. Wait or choose another folder name.",AppErrorRetry::ChooseAnotherDestination),
        ErrorCode::InvalidSource => ("Choose a local repository or enter an HTTPS or SSH repository address without embedded credentials, query or fragment.",AppErrorRetry::None),
        ErrorCode::UnsafePath => ("Choose a safe folder and a valid Windows folder name.",AppErrorRetry::ChooseAnotherDestination),
        ErrorCode::Io if phase==Phase::Recovery => (RETENTION_NOTICE,AppErrorRetry::None),
        ErrorCode::Io => ("The operation could not access its files. Check storage and permissions; inspect any remaining staging data before retrying.",AppErrorRetry::Retry),
        ErrorCode::UnsupportedGit => (
            "Git 2.31.0 or newer is required. Update Git and retry.",
            AppErrorRetry::Retry,
        ),
        ErrorCode::AuthenticationRequired | ErrorCode::AuthenticationFailed => (
            "Check your repository credentials and access.",
            AppErrorRetry::CheckCredentials,
        ),
        ErrorCode::SshAuthentication => (
            "Check your SSH key and agent configuration.",
            AppErrorRetry::CheckCredentials,
        ),
        ErrorCode::Certificate => (
            "Certificate verification failed. Check the configured certificate trust.",
            AppErrorRetry::CheckTrust,
        ),
        ErrorCode::SshHostKey => (
            "SSH host verification failed. Verify the host key before retrying.",
            AppErrorRetry::CheckTrust,
        ),
        ErrorCode::Network => (
            "The remote could not be reached. Check network and proxy settings.",
            AppErrorRetry::CheckNetwork,
        ),
        ErrorCode::PermissionDenied => (
            "Access was denied. Check repository and filesystem permissions.",
            AppErrorRetry::None,
        ),
        ErrorCode::UnsafeRepository => (
            "Git rejected repository ownership. Review its ownership in Git.",
            AppErrorRetry::ReopenRepository,
        ),
        ErrorCode::DiskFull => (
            "Storage is full. Free space before retrying.",
            AppErrorRetry::Retry,
        ),
        ErrorCode::MalformedOutput => {
            ("Git returned an unsupported response.", AppErrorRetry::None)
        }
        _ => (
            "The operation could not be completed safely.",
            AppErrorRetry::None,
        ),
    };
    AppError {
        code,
        message: message.into(),
        phase,
        retry,
    }
}

pub fn git_failure(stderr: &[u8], phase: Phase) -> AppError {
    // Classify a bounded buffer only; raw text is deliberately discarded.
    let text = String::from_utf8_lossy(stderr).to_ascii_lowercase();
    let code = if text.contains("not a git repository") {
        ErrorCode::NotRepository
    } else if text.contains("dubious ownership") {
        ErrorCode::UnsafeRepository
    } else if text.contains("certificate") {
        ErrorCode::Certificate
    } else if text.contains("host key verification failed") {
        ErrorCode::SshHostKey
    } else if text.contains("permission denied (publickey") {
        ErrorCode::SshAuthentication
    } else if text.contains("authentication failed") {
        ErrorCode::AuthenticationFailed
    } else if text.contains("permission denied") {
        ErrorCode::PermissionDenied
    } else if text.contains("no space left on device") || text.contains("disk full") {
        ErrorCode::DiskFull
    } else if text.contains("could not resolve host")
        || text.contains("failed to connect")
        || text.contains("could not resolve proxy")
        || text.contains("connect tunnel failed")
        || text.contains("received http code 407")
        || text.contains("proxy authentication required")
    {
        ErrorCode::Network
    } else {
        ErrorCode::Internal
    };
    error(code, phase)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ssh_limit_notice_preserves_code_and_both_safe_suffixes_without_other_error_changes() {
        let original = error(ErrorCode::SshHostKey, Phase::Transfer);
        let warned = retained_error(Some(ssh_prompt_unavailable(original.clone())));
        for message in [
            warned.message.clone(),
            format!("REVIEW_SYNTHETIC_CANARY {RETENTION_NOTICE} {SSH_PROMPT_NOTICE}"),
        ] {
            let safe = sanitized_error(&AppError {
                message,
                ..original.clone()
            });
            assert_eq!(safe.code, ErrorCode::SshHostKey);
            assert!(safe.message.contains(SSH_PROMPT_NOTICE));
            assert!(safe.message.ends_with(RETENTION_NOTICE));
            assert!(!safe.message.contains("REVIEW_SYNTHETIC_CANARY"));
        }
        for code in [
            ErrorCode::Certificate,
            ErrorCode::Network,
            ErrorCode::PermissionDenied,
            ErrorCode::Internal,
        ] {
            let original = error(code, Phase::Transfer);
            assert_eq!(ssh_prompt_unavailable(original.clone()), original);
        }
        assert_eq!(
            crate::clone::request::remote_transport("https://example.invalid/ssh"),
            Ok(crate::clone::request::RemoteTransport::Https)
        );
        assert_eq!(
            crate::clone::request::remote_transport("ssh://git@example.invalid/repo"),
            Ok(crate::clone::request::RemoteTransport::Ssh)
        );
        assert_eq!(
            crate::clone::request::remote_transport("git@example.invalid:repo"),
            Ok(crate::clone::request::RemoteTransport::Ssh)
        );
        assert!(crate::clone::request::remote_transport("file:///ssh").is_err());
    }
    #[test]
    fn configured_proxy_failures_return_safe_actionable_templates() {
        for diagnostic in [
            "Could not resolve proxy: REVIEW_SYNTHETIC_CANARY",
            "CONNECT tunnel failed, response 407 REVIEW_SYNTHETIC_CANARY",
            "Received HTTP code 407 from proxy after CONNECT REVIEW_SYNTHETIC_CANARY",
        ] {
            let failure = git_failure(diagnostic.as_bytes(), Phase::Transfer);
            assert_eq!(failure.code, ErrorCode::Network);
            assert_eq!(failure.retry, AppErrorRetry::CheckNetwork);
            assert!(!serde_json::to_string(&failure)
                .unwrap()
                .contains("REVIEW_SYNTHETIC_CANARY"));
        }
    }
}
