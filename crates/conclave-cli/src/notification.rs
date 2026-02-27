use conclave_client::sanitize_control_chars;

const MAX_SUMMARY_LENGTH: usize = 256;
const MAX_BODY_LENGTH: usize = 1024;

fn truncate_str(input: &str, max_length: usize) -> String {
    if input.len() <= max_length {
        return input.to_string();
    }
    let mut truncated: String = input.chars().take(max_length).collect();
    truncated.push('…');
    truncated
}

/// Send a desktop notification. Best-effort: errors are logged but not propagated.
pub fn send_notification(summary: &str, body: &str) {
    let summary = truncate_str(&sanitize_control_chars(summary), MAX_SUMMARY_LENGTH);
    let body = truncate_str(&sanitize_control_chars(body), MAX_BODY_LENGTH);

    let mut notification = notify_rust::Notification::new();
    notification
        .appname("Conclave")
        .summary(&summary)
        .body(&body);

    #[cfg(target_os = "linux")]
    {
        notification
            .hint(notify_rust::Hint::DesktopEntry("conclave".to_owned()))
            .sound_name("message-new-instant");
    }

    #[cfg(target_os = "windows")]
    {
        notification.sound_name("IM");
    }

    #[cfg(target_os = "macos")]
    {
        notification.sound_name("Default");
    }

    if let Err(error) = notification.show() {
        tracing::warn!(%error, "failed to send desktop notification");
    }
}
