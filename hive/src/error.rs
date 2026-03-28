use thiserror::Error;

#[derive(Error, Debug)]
pub enum HiveError {
    #[error("Parse error: {0}")]
    Parse(String),

    #[error("Config error: {0}")]
    Config(String),

    #[error("HTTP error: {0}")]
    Http(String),

    #[error("Interpolation error: {0}")]
    Interpolation(String),

    #[error("Capture error: {0}")]
    Capture(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Validation error: {0}")]
    Validation(String),
}

impl HiveError {
    /// Map error to CLI exit code per spec:
    /// 2 = configuration/parse error
    /// 3 = runtime error (network, timeout)
    pub fn exit_code(&self) -> i32 {
        match self {
            HiveError::Parse(_) => 2,
            HiveError::Config(_) => 2,
            HiveError::Validation(_) => 2,
            HiveError::Http(_) => 3,
            HiveError::Io(_) => 3,
            HiveError::Interpolation(_) => 2,
            HiveError::Capture(_) => 3,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_error_exit_code_is_2() {
        let err = HiveError::Parse("bad yaml".into());
        assert_eq!(err.exit_code(), 2);
    }

    #[test]
    fn config_error_exit_code_is_2() {
        let err = HiveError::Config("missing field".into());
        assert_eq!(err.exit_code(), 2);
    }

    #[test]
    fn validation_error_exit_code_is_2() {
        let err = HiveError::Validation("invalid schema".into());
        assert_eq!(err.exit_code(), 2);
    }

    #[test]
    fn http_error_exit_code_is_3() {
        let err = HiveError::Http("connection refused".into());
        assert_eq!(err.exit_code(), 3);
    }

    #[test]
    fn io_error_exit_code_is_3() {
        let err = HiveError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "file not found",
        ));
        assert_eq!(err.exit_code(), 3);
    }

    #[test]
    fn interpolation_error_exit_code_is_2() {
        let err = HiveError::Interpolation("unknown var".into());
        assert_eq!(err.exit_code(), 2);
    }

    #[test]
    fn capture_error_exit_code_is_3() {
        let err = HiveError::Capture("jsonpath failed".into());
        assert_eq!(err.exit_code(), 3);
    }

    #[test]
    fn error_display_messages() {
        assert_eq!(
            HiveError::Parse("bad".into()).to_string(),
            "Parse error: bad"
        );
        assert_eq!(
            HiveError::Http("timeout".into()).to_string(),
            "HTTP error: timeout"
        );
    }
}
