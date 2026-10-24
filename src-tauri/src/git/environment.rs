use super::runner::{argv, Cancellation, GitRunner};
use crate::domain::contracts::*;
pub async fn identity(runner: &GitRunner, cwd: Option<&std::path::Path>) -> IdentityPresence {
    let mut result = IdentityPresence {
        name_configured: false,
        email_configured: false,
    };
    for (key, target) in [
        ("user.name", &mut result.name_configured),
        ("user.email", &mut result.email_configured),
    ] {
        *target = runner
            .run(
                cwd,
                &argv(&["config", "--get", key]),
                &Cancellation::default(),
                Phase::Discovery,
            )
            .await
            .is_ok_and(|v| v.iter().any(|b| !b.is_ascii_whitespace()));
    }
    result
}
pub async fn environment(runner: Option<&GitRunner>) -> Environment {
    let unavailable = || ToolInfo {
        available: false,
        version: None,
    };
    let mut value = Environment {
        contract_version: EnvironmentContractVersion::Value0,
        app_version: env!("CARGO_PKG_VERSION").into(),
        platform: EnvironmentPlatform::Windows,
        git: unavailable(),
        credential_manager: unavailable(),
        open_ssh: unavailable(),
        web_view2: unavailable(),
        identity: IdentityPresence {
            name_configured: false,
            email_configured: false,
        },
    };
    if let Some(runner) = runner {
        if let Ok(version) = super::version::probe(runner).await {
            value.git = ToolInfo {
                available: version.supported(),
                version: Some(version.to_string()),
            };
        }
        for (args, target) in [(
            argv(&["credential-manager", "--version"]),
            &mut value.credential_manager,
        )] {
            if let Ok(bytes) = runner
                .run(None, &args, &Cancellation::default(), Phase::Discovery)
                .await
            {
                // Only a numeric version is surfaced, never untrusted version banners.
                let text = String::from_utf8_lossy(&bytes);
                if let Some(version) = text.split_whitespace().find_map(numeric_version) {
                    *target = ToolInfo {
                        available: true,
                        version: Some(version),
                    };
                }
            }
        }
        if value.git.available {
            value.identity = identity(runner, None).await;
        }
    }
    #[cfg(windows)]
    {
        let mut candidates = Vec::new();
        if let Some(root) = std::env::var_os("SystemRoot") {
            candidates.push(std::path::PathBuf::from(root).join("System32/OpenSSH/ssh.exe"));
        }
        if let Some(runner) = runner {
            if let Some(root) = runner.executable().parent().and_then(|p| p.parent()) {
                candidates.push(root.join("usr/bin/ssh.exe"));
            }
        }
        for path in candidates {
            if let Ok(probe) = GitRunner::trusted(path) {
                if let Ok(output) = probe
                    .output(
                        None,
                        &argv(&["-V"]),
                        &Cancellation::default(),
                        Phase::Discovery,
                    )
                    .await
                {
                    let text = String::from_utf8_lossy(&output.stderr);
                    if output.success {
                        if let Some(version) = text
                            .split_whitespace()
                            .next()
                            .and_then(|s| {
                                s.strip_prefix("OpenSSH_for_Windows_")
                                    .or_else(|| s.strip_prefix("OpenSSH_"))
                            })
                            .and_then(numeric_version)
                        {
                            value.open_ssh = ToolInfo {
                                available: true,
                                version: Some(version),
                            };
                            break;
                        }
                    }
                }
            }
        }
    }
    value
}
fn numeric_version(value: &str) -> Option<String> {
    let prefix: String = value
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .take(48)
        .collect();
    let prefix = prefix.trim_end_matches('.');
    if prefix.split('.').count() < 2 || prefix.split('.').any(|part| part.is_empty()) {
        return None;
    }
    Some(prefix.into())
}
