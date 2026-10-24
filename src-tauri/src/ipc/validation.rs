use crate::{domain::contracts::*, git::error::error};
pub fn request_size<T: serde::Serialize>(value: &T) -> Result<(), AppError> {
    // Count the actual JSON wire size without retaining a second secret copy.
    struct Limit(usize);
    impl std::io::Write for Limit {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if self.0.saturating_add(bytes.len()) > 16 * 1024 {
                return Err(std::io::Error::other("Request exceeds limit"));
            }
            self.0 += bytes.len();
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    if serde_json::to_writer(Limit(0), value).is_err() {
        Err(error(ErrorCode::InvalidRequest, Phase::Validation))
    } else {
        Ok(())
    }
}
pub fn counter(value: &str) -> Result<u64, AppError> {
    if value.is_empty() || value.len() > 20 || !value.bytes().all(|c| c.is_ascii_digit()) {
        return Err(error(ErrorCode::InvalidRequest, Phase::Validation));
    }
    value
        .parse()
        .map_err(|_| error(ErrorCode::InvalidRequest, Phase::Validation))
}
pub fn scope(window: &str) -> Result<(), AppError> {
    if window.is_empty() || window.len() > 256 || window.contains('\0') {
        Err(error(ErrorCode::InvalidHandle, Phase::Validation))
    } else {
        Ok(())
    }
}
