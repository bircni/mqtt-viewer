use crate::topic::{matches, rewrite_prefix, validate_filter, validate_topic};

#[test]
fn exact_and_wildcard_filters_match() {
    assert!(matches("a/b", "a/b"));
    assert!(!matches("a/b", "a/c"));
    assert!(!matches("a/b", "a/b/c"));
    assert!(!matches("a/b/c", "a/b"));
    assert!(matches("a/+", "a/b"));
    assert!(!matches("a/+", "a/b/c"));
    assert!(matches("+/+/c", "a/b/c"));
    assert!(matches("#", "a/b/c"));
    assert!(matches("a/#", "a/b/c"));
    assert!(matches("a/#", "a"));
    assert!(matches("a/+/c", "a//c"));
}

#[test]
fn leading_wildcards_do_not_match_dollar_topics() {
    assert!(!matches("#", "$SYS/broker/uptime"));
    assert!(!matches("+/broker/uptime", "$SYS/broker/uptime"));
    assert!(matches("$SYS/#", "$SYS/broker/uptime"));
}

#[test]
fn filters_are_validated() {
    assert_eq!(validate_filter("a/+/b/#"), Ok(()));
    assert_eq!(validate_filter("#"), Ok(()));
    assert!(validate_filter("").is_err());
    assert!(validate_filter("a/#/b").is_err());
    assert!(validate_filter("a/b+").is_err());
    assert!(validate_filter("a/b#").is_err());
}

#[test]
fn publish_topics_are_validated() {
    assert_eq!(validate_topic("a/b"), Ok(()));
    assert!(validate_topic("").is_err());
    assert!(validate_topic("a/+").is_err());
    assert!(validate_topic("a/#").is_err());
}

#[test]
fn prefix_rewrite_only_touches_matching_topics() {
    assert_eq!(rewrite_prefix("home/kitchen/temp", "home/", "replay/"), "replay/kitchen/temp");
    assert_eq!(rewrite_prefix("office/temp", "home/", "replay/"), "office/temp");
    assert_eq!(rewrite_prefix("home", "", "x/"), "x/home");
}
