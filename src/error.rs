#![allow(dead_code)]

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn io_error_converts_via_from() {
        let io = std::io::Error::new(std::io::ErrorKind::NotFound, "missing");
        let err: Error = io.into();
        assert!(matches!(err, Error::Io(_)));
    }

    #[test]
    fn io_error_display_includes_inner_message() {
        let io = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied");
        let err = Error::Io(io);
        let rendered = err.to_string();
        assert!(rendered.starts_with("I/O error:"), "got {rendered:?}");
        assert!(rendered.contains("denied"), "got {rendered:?}");
    }
}
