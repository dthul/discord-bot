// Re-export eyre::Report as BoxedError for better tracing support

/// Enhanced error type that captures both the underlying error and tracing context
pub type BoxedError = eyre::Report;
