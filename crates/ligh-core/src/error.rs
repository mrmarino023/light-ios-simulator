pub type Result<T> = std::result::Result<T, LighError>;

#[derive(Debug, thiserror::Error)]
pub enum LighError {
    #[error("simctl: {0}")]
    Simctl(String),

    #[error("simulator not booted (udid={udid}); run `ligh up`")]
    SimNotBooted { udid: String },

    #[error("no active session; run `ligh device create` then `ligh up`")]
    NoSession,

    #[error("boot timed out after {seconds}s (udid={udid}) — run `ligh device create`")]
    BootTimeout { udid: String, seconds: u64 },

    #[error("simulator not ready: {0}")]
    NotReady(String),

    #[error("device not found: {0}")]
    DeviceNotFound(String),

    #[error("config error at {path}: {message}")]
    Config { path: String, message: String },

    #[error("doctor: {0}")]
    Doctor(String),

    #[error("disk space low: {available_mb} MB free — free space before using simulators")]
    DiskSpace { available_mb: u64 },

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error("json: {0}")]
    Json(#[from] serde_json::Error),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}
