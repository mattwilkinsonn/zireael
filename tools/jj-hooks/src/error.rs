use thiserror::Error;

#[derive(Debug, Error)]
pub enum JjHooksError {
    #[error("jj exited with status {status}: {stderr}")]
    JjFailed { status: i32, stderr: String },

    #[error("setup step `{name}` exited with status {status}")]
    SetupFailed { name: String, status: i32 },

    #[error("could not parse `jj git push --dry-run` output: {0}")]
    Parse(String),

    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, JjHooksError>;
