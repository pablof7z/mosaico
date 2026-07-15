pub(crate) fn message(agent_handle: &str, detach_reason: &str) -> String {
    match detach_reason {
        "pty output closed" | "pty disconnected" => {
            format!("{agent_handle} agent session terminated")
        }
        "stdin eof" | "stdin disconnected" | "stdin closed" => {
            format!("Detached from {agent_handle}")
        }
        _ => format!("Detached from {agent_handle}: {detach_reason}"),
    }
}

#[cfg(test)]
mod tests {
    use super::message;

    #[test]
    fn distinguishes_harness_termination_from_client_detach() {
        assert_eq!(
            message("echo-codex", "pty output closed"),
            "echo-codex agent session terminated"
        );
        assert_eq!(
            message("echo-codex", "stdin eof"),
            "Detached from echo-codex"
        );
    }
}
