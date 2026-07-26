#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MemberClass {
    Ignore,
    Agent,
    Human,
    Unknown,
}

pub(crate) fn classify(
    role: &str,
    is_backend: bool,
    has_profile: bool,
    is_named_agent: bool,
) -> MemberClass {
    if role == "admin" || is_backend {
        MemberClass::Ignore
    } else if is_named_agent {
        MemberClass::Agent
    } else if has_profile {
        MemberClass::Human
    } else {
        MemberClass::Unknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_known_profile_without_an_agent_handle_is_a_human() {
        assert_eq!(classify("member", false, true, false), MemberClass::Human);
        assert_eq!(
            classify("member", false, false, false),
            MemberClass::Unknown
        );
    }
}
