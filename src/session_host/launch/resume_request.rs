use super::LaunchIntent;

#[derive(Clone, Copy)]
pub(crate) struct ResumeRequest<'a> {
    pub(super) intent: LaunchIntent,
    pub(super) extra_args: &'a [String],
}

impl<'a> ResumeRequest<'a> {
    pub(crate) fn with_args(intent: LaunchIntent, extra_args: &'a [String]) -> Self {
        Self { intent, extra_args }
    }

    pub(crate) fn without_args(intent: LaunchIntent) -> Self {
        Self {
            intent,
            extra_args: &[],
        }
    }
}
