use anyhow::Result;

#[cfg(test)]
#[path = "notices/tests.rs"]
mod tests;

pub(super) fn print_send_notices(result: &serde_json::Value) -> Result<()> {
    for line in send_notice_lines(result)? {
        println!("{line}");
    }
    Ok(())
}

pub(super) fn send_notice_lines(result: &serde_json::Value) -> Result<Vec<String>> {
    crate::channel_render::send_notice_lines(result, crate::channel_render::NoticeHint::Cli)
}
