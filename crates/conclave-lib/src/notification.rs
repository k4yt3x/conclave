use tracing::warn;

/// Send a desktop notification. Best-effort: errors are logged but not propagated.
pub fn send_notification(summary: &str, body: &str) {
    if let Err(error) = notify_rust::Notification::new()
        .summary(summary)
        .body(body)
        .sound_name("message-new-instant")
        .show()
    {
        warn!("failed to send desktop notification: {error}");
    }
}
