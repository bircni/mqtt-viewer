//! Topic name and topic filter helpers.

/// Whether `topic` matches the MQTT topic filter `filter` (`+` and `#` wildcards).
///
/// Follows the MQTT rule that a filter starting with a wildcard does not match topics starting with `$`.
#[must_use]
pub fn matches(filter: &str, topic: &str) -> bool {
    if topic.starts_with('$') && (filter.starts_with('+') || filter.starts_with('#')) {
        return false;
    }
    let mut filter_levels = filter.split('/');
    let mut topic_levels = topic.split('/');
    loop {
        match (filter_levels.next(), topic_levels.next()) {
            (Some("#"), _) | (None, None) => return true,
            (Some(expected), Some(actual)) if expected == "+" || expected == actual => {}
            _ => return false,
        }
    }
}

/// Checks that `filter` is a well-formed subscription filter.
///
/// # Errors
/// Returns a human-readable reason when it is not.
pub fn validate_filter(filter: &str) -> Result<(), String> {
    if filter.is_empty() {
        return Err("topic filter must not be empty".to_owned());
    }
    let levels: Vec<&str> = filter.split('/').collect();
    for (index, level) in levels.iter().enumerate() {
        if *level == "#" {
            if index + 1 != levels.len() {
                return Err("'#' is only allowed as the last level".to_owned());
            }
        } else if level.len() > 1 && (level.contains('#') || level.contains('+')) {
            return Err(format!("wildcard must occupy a whole level: '{level}'"));
        }
    }
    Ok(())
}

/// Checks that `topic` can be published to.
///
/// # Errors
/// Returns a human-readable reason when it cannot.
pub fn validate_topic(topic: &str) -> Result<(), String> {
    if topic.is_empty() {
        return Err("topic must not be empty".to_owned());
    }
    if topic.contains(['#', '+']) {
        return Err("topic must not contain wildcards".to_owned());
    }
    Ok(())
}

/// Rewrites the leading `from` prefix of `topic` to `to`; topics without that prefix are returned unchanged.
#[must_use]
pub fn rewrite_prefix(topic: &str, from: &str, to: &str) -> String {
    topic.strip_prefix(from).map_or_else(|| topic.to_owned(), |rest| format!("{to}{rest}"))
}
