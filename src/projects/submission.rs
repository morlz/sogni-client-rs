use std::fmt;

use crate::Error;

/// Phase reached by one call to [`super::ProjectsApi::create_with_id_detailed`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubmissionPhase {
    Prepare,
    AssetUpload,
    Send,
}

impl SubmissionPhase {
    /// Only `Send` can have transmitted this call's generation request.
    /// This says nothing about earlier calls made with the same project ID.
    #[must_use]
    pub const fn request_may_have_been_sent(self) -> bool {
        matches!(self, Self::Send)
    }
}

/// Submission failure with an explicit boundary before the generation request.
/// Default formatting omits the underlying error, which may contain private URLs.
pub struct ProjectSubmissionError {
    phase: SubmissionPhase,
    cause: Error,
}

impl ProjectSubmissionError {
    pub(crate) const fn new(phase: SubmissionPhase, cause: Error) -> Self {
        Self { phase, cause }
    }

    #[must_use]
    pub const fn phase(&self) -> SubmissionPhase {
        self.phase
    }

    /// Inspect typed status/timeout fields without logging raw error text.
    #[must_use]
    pub const fn cause(&self) -> &Error {
        &self.cause
    }

    #[must_use]
    pub fn into_cause(self) -> Error {
        self.cause
    }
}

impl fmt::Debug for ProjectSubmissionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProjectSubmissionError")
            .field("phase", &self.phase)
            .finish_non_exhaustive()
    }
}

impl fmt::Display for ProjectSubmissionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "project submission failed during {:?}",
            self.phase
        )
    }
}

impl std::error::Error for ProjectSubmissionError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostics_preserve_the_typed_cause_without_default_secret_output() {
        let failure = ProjectSubmissionError::new(
            SubmissionPhase::AssetUpload,
            Error::Transport("https://private.example/secret-marker".into()),
        );
        assert!(!format!("{failure:?} {failure}").contains("secret-marker"));
        assert!(matches!(failure.cause(), Error::Transport(_)));
        assert!(!failure.phase().request_may_have_been_sent());
        assert!(SubmissionPhase::Send.request_may_have_been_sent());
    }
}
