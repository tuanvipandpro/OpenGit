#[cfg(windows)]
mod helper_path;
#[cfg(windows)]
pub mod pipe;
pub mod prompts;
#[cfg(windows)]
pub use pipe::{AuthBroker, AuthSession};

/// Main calls this before opening the UI. Ordinary launches return `None`.
/// Invalid explicit helper launches fail closed without starting a webview.
pub fn helper_entry() -> Option<i32> {
    std::env::var_os("OPENGIT_INTERNAL_ASKPASS")?;
    #[cfg(windows)]
    {
        Some(pipe::helper_main())
    }
    #[cfg(not(windows))]
    {
        Some(1)
    }
}
