mod error;
mod submissions;

pub use error::{AppError, AppResult, Required};
pub use submissions::{SubmissionService, SubmitHomework};
