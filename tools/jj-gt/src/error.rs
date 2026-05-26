use thiserror::Error;

#[derive(Debug, Error)]
pub enum JjGtError {
    #[error("jj exited with status {status}: {stderr}")]
    JjFailed { status: i32, stderr: String },

    #[error("gt exited with status {status}: {stderr}")]
    GtFailed { status: i32, stderr: String },

    #[error("gh exited with status {status}: {stderr}")]
    GhFailed { status: i32, stderr: String },

    #[error("failed to read gt repo config: {0}")]
    GtRepoConfig(String),

    #[error("could not derive parent bookmark for `{bookmark}`: {reason}")]
    ParentDerivation { bookmark: String, reason: String },

    #[error("selection is non-linear: {0}")]
    NonLinearStack(String),

    #[error("no bookmarks selected")]
    NoBookmarksSelected,

    #[error("invalid input: {0}")]
    Invalid(String),

    #[error(transparent)]
    Hooks(#[from] jj_hooks::error::JjHooksError),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, JjGtError>;
