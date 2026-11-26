use std::time::{Duration, Instant};

use bytes::Bytes;
use jiff::Timestamp;

use crate::config::TreeOrder;
use crate::message::{Message, Properties, QoS};
use crate::tree::{Filter, HistoryLimits, NodeId, TopicTree};

fn tree_with(topics: &[&str]) -> TopicTree {
    let mut tree = TopicTree::default();
    // Most tests read the rows right after inserting and expect them to be current.
    tree.set_refresh_interval(Duration::ZERO);
    for topic in topics {
        tree.insert(Message::new(*topic, "v"));
    }
    tree
}

/// Renders the visible rows as indented names, `+`/`-` marking collapsed/expanded parents.
fn render(tree: &mut TopicTree) -> String {
    let rows = tree.rows().to_vec();
    rows.iter()
        .map(|row| {
            let marker = match (row.has_children, row.expanded) {
                (false, _) => ' ',
                (true, false) => '+',
                (true, true) => '-',
            };
            format!("{}{marker}{}", "  ".repeat(row.depth), tree.node(row.id).name())
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn expand_all(tree: &mut TopicTree) {
    tree.set_expanded_recursive(TopicTree::ROOT, true);
}

#[test]
fn topics_become_nested_levels() {
    let mut tree = tree_with(&["home/kitchen/temp", "home/kitchen/humidity", "home/hall/light", "office/temp"]);
    assert_eq!(render(&mut tree), "+home\n+office");
    expand_all(&mut tree);
    insta::assert_snapshot!(render(&mut tree), @r"
    -home
      -hall
         light
      -kitchen
         humidity
         temp
    -office
       temp
    ");
}

#[test]
fn counters_roll_up_to_ancestors() {
    let tree = tree_with(&["a/b", "a/b", "a/c", "a"]);
    let a = tree.find("a").unwrap();
    let b = tree.find("a/b").unwrap();
    assert_eq!(tree.node(a).message_count(), 1);
    assert_eq!(tree.node(a).subtree_messages(), 4);
    assert_eq!(tree.node(a).subtree_topics(), 3);
    assert_eq!(tree.node(b).message_count(), 2);
    assert_eq!(tree.node(b).subtree_topics(), 1);
    assert_eq!(tree.node(b).history().len(), 2);
    assert_eq!(tree.node(b).parent(), Some(a));
    assert_eq!(tree.node(a).children().len(), 2);
    let stats = tree.stats();
    assert_eq!((stats.topics, stats.messages, stats.stored_messages), (3, 4, 4));
    assert!(stats.stored_bytes > 0);
    assert_eq!(tree.node(TopicTree::ROOT).subtree_messages(), 4);
}

#[test]
fn topic_names_round_trip_including_empty_levels() {
    let mut tree = tree_with(&["/leading", "trailing/", "a//b", "$SYS/broker"]);
    for topic in ["/leading", "trailing/", "a//b", "$SYS/broker"] {
        let id = tree.find(topic).unwrap();
        assert_eq!(tree.topic(id), topic);
        assert!(tree.node(id).last().is_some(), "{topic}");
    }
    assert_eq!(tree.find("nope"), None);
    assert_eq!(tree.find("a/x"), None);
    assert_eq!(tree.topic(TopicTree::ROOT), "");
    expand_all(&mut tree);
    assert_eq!(tree.rows().len(), 9);
}

#[test]
fn last_value_and_activity_are_tracked() {
    let mut tree = TopicTree::default();
    let early = Timestamp::from_second(1_000).unwrap();
    let late = Timestamp::from_second(2_000).unwrap();
    tree.insert(Message { received_at: late, ..Message::new("a/b", "new") });
    tree.insert(Message { received_at: early, ..Message::new("a/c", "old") });
    let a = tree.find("a").unwrap();
    assert_eq!(tree.node(a).last_activity(), Some(late), "activity never moves backwards");
    assert!(tree.node(a).last().is_none());
    let b = tree.find("a/b").unwrap();
    let entry = tree.node(b).last().unwrap();
    assert_eq!(&entry.payload[..], b"new");
    let message = entry.to_message("a/b");
    assert_eq!((message.topic.as_str(), message.received_at), ("a/b", late));
}

#[test]
fn expansion_controls_visible_rows() {
    let mut tree = tree_with(&["a/b/c", "a/d", "e"]);
    let a = tree.find("a").unwrap();
    assert!(!tree.node(a).is_expanded());
    tree.set_expanded(a, true);
    assert_eq!(render(&mut tree), "-a\n  +b\n   d\n e");
    tree.set_expanded(a, true);
    tree.set_expanded_recursive(a, true);
    assert_eq!(render(&mut tree), "-a\n  -b\n     c\n   d\n e");
    tree.set_expanded_recursive(a, false);
    assert_eq!(render(&mut tree), "+a\n e");

    let c = tree.find("a/b/c").unwrap();
    assert_eq!(tree.row_of(c), None);
    tree.reveal(c);
    assert_eq!(tree.row_of(c), Some(2));
    assert_eq!(tree.rows()[2].depth, 2);
}

#[test]
fn new_topics_appear_without_touching_expansion() {
    let mut tree = tree_with(&["a/b"]);
    let a = tree.find("a").unwrap();
    tree.set_expanded(a, true);
    assert_eq!(render(&mut tree), "-a\n   b");
    tree.insert(Message::new("a/a", "v"));
    tree.insert(Message::new("0", "v"));
    assert_eq!(render(&mut tree), " 0\n-a\n   a\n   b");
}

#[test]
fn new_levels_open_by_themselves_down_to_the_configured_depth() {
    let mut tree = TopicTree::default();
    tree.set_auto_expand(2);
    for topic in ["a/b/c/d", "e"] {
        tree.insert(Message::new(topic, "v"));
    }
    assert_eq!(render(&mut tree), "-a\n  -b\n    +c\n e");

    // Collapsing by hand sticks, and the setting survives a clear.
    let a = tree.find("a").unwrap();
    tree.set_expanded(a, false);
    tree.insert(Message::new("a/x", "v"));
    assert_eq!(render(&mut tree), "+a\n e");
    tree.clear();
    tree.insert(Message::new("f/g/h", "v"));
    assert_eq!(render(&mut tree), "-f\n  -g\n     h");

    tree.set_auto_expand(0);
    tree.insert(Message::new("i/j", "v"));
    assert_eq!(render(&mut tree), "-f\n  -g\n     h\n+i");
}

#[test]
fn ordering_by_messages_and_topics() {
    let mut tree = tree_with(&["a/1", "b/1", "b/1", "b/1", "c/1", "c/2", "c/3", "c/4"]);
    assert_eq!(tree.order(), TreeOrder::Alphabetical);
    assert_eq!(render(&mut tree), "+a\n+b\n+c");
    tree.set_order(TreeOrder::Messages);
    assert_eq!(render(&mut tree), "+c\n+b\n+a");
    tree.set_order(TreeOrder::Topics);
    assert_eq!(render(&mut tree), "+c\n+a\n+b", "ties stay alphabetical");
    // Counts change with every message, so the order follows.
    tree.set_order(TreeOrder::Messages);
    for _ in 0..10 {
        tree.insert(Message::new("a/1", "v"));
    }
    assert_eq!(render(&mut tree), "+a\n+c\n+b");
}

#[test]
fn filter_keeps_matches_and_their_ancestors() {
    let mut tree = tree_with(&["home/kitchen/temp", "home/hall/light", "office/Temp", "garage/door"]);
    tree.set_filter(Filter::new("TEMP", false));
    assert!(!tree.filter().is_empty());
    assert_eq!(render(&mut tree), "-home\n  -kitchen\n     temp\n-office\n   Temp");

    // A level that matches by itself keeps its own expansion state, and everything below it stays reachable.
    tree.set_filter(Filter::new("home", false));
    assert_eq!(render(&mut tree), "+home");
    let home = tree.find("home").unwrap();
    tree.set_expanded(home, true);
    assert_eq!(render(&mut tree), "-home\n  +hall\n  +kitchen");

    tree.set_filter(Filter::new("zzz", false));
    assert_eq!(render(&mut tree), "");
    tree.set_filter(Filter::default());
    assert_eq!(tree.rows().len(), 5);
}

#[test]
fn filter_can_search_payloads() {
    let mut tree = TopicTree::default();
    tree.set_refresh_interval(Duration::ZERO);
    tree.insert(Message::new("a/b", r#"{"state":"OPEN"}"#));
    tree.insert(Message::new("a/c", "closed"));
    tree.insert(Message::new("d", "x"));
    tree.set_filter(Filter::new("open", false));
    assert_eq!(render(&mut tree), "");
    tree.set_filter(Filter::new("open", true));
    assert_eq!(render(&mut tree), "-a\n   b");
    // The filter follows the latest value.
    tree.insert(Message::new("a/c", "open now"));
    assert_eq!(render(&mut tree), "-a\n   b\n   c");
    tree.set_filter(Filter::new("a-needle-longer-than-any-payload", true));
    assert_eq!(render(&mut tree), "");
}

#[test]
fn per_topic_cap_drops_the_oldest() {
    let mut tree = TopicTree::new(HistoryLimits { per_topic: 3, total_bytes: usize::MAX });
    for index in 0..10 {
        tree.insert(Message::new("t", index.to_string()));
    }
    let node = tree.node(tree.find("t").unwrap());
    let kept: Vec<&[u8]> = node.history().iter().map(|entry| &entry.payload[..]).collect();
    assert_eq!(kept, [b"7", b"8", b"9"]);
    assert_eq!(node.message_count(), 10, "the counter keeps counting evicted messages");
    assert_eq!(tree.stats().stored_messages, 3);

    tree.set_limits(HistoryLimits { per_topic: 0, total_bytes: usize::MAX });
    assert_eq!(tree.limits().per_topic, 1, "a cap of zero would lose the current value");
    assert_eq!(tree.node(tree.find("t").unwrap()).history().len(), 1);
}

#[test]
fn byte_budget_evicts_oldest_first_but_keeps_latest_values() {
    let payload = vec![b'x'; 1000];
    let mut tree = TopicTree::new(HistoryLimits { per_topic: 1000, total_bytes: 20_000 });
    for round in 0..50 {
        for topic in ["a", "b", "c"] {
            tree.insert(Message::new(topic, Bytes::from([payload.as_slice(), round.to_string().as_bytes()].concat())));
        }
    }
    let stats = tree.stats();
    assert!(stats.stored_bytes <= 20_000, "{stats:?}");
    assert!(stats.stored_messages >= 15, "{stats:?}");
    for topic in ["a", "b", "c"] {
        let node = tree.node(tree.find(topic).unwrap());
        assert!(node.last().unwrap().payload.ends_with(b"49"), "{topic} lost its latest value");
        assert!(node.history().len() >= 5, "eviction should be spread evenly, {topic} has {}", node.history().len());
    }

    // Even an impossible budget keeps one value per topic.
    tree.set_limits(HistoryLimits { per_topic: 1000, total_bytes: 1 });
    assert_eq!(tree.stats().stored_messages, 3);
}

#[test]
fn properties_count_towards_the_budget() {
    let mut plain = TopicTree::default();
    plain.insert(Message::new("t", "v"));
    let mut rich = TopicTree::default();
    let properties = Properties {
        user_properties: vec![("key".into(), "value".into())],
        correlation_data: Some(Bytes::from_static(b"1234")),
        ..Properties::default()
    };
    rich.insert(Message { properties: Some(Box::new(properties)), qos: QoS::AtLeastOnce, ..Message::new("t", "v") });
    assert!(rich.stats().stored_bytes > plain.stats().stored_bytes);
    let entry = rich.node(rich.find("t").unwrap()).last().unwrap().clone();
    assert_eq!(entry.qos, QoS::AtLeastOnce);
    assert!(entry.properties.is_some());
}

#[test]
fn removing_a_topic_prunes_empty_levels() {
    let mut tree = tree_with(&["a/b/c", "a/b/c", "a/b/d", "a/e", "f"]);
    expand_all(&mut tree);
    let c = tree.find("a/b/c").unwrap();
    assert_eq!(tree.topics_with_values(c, false), ["a/b/c"]);
    tree.remove(c, false);
    assert!(!tree.contains(c));
    assert_eq!(render(&mut tree), "-a\n  -b\n     d\n   e\n f");
    let a = tree.find("a").unwrap();
    assert_eq!(tree.node(a).subtree_messages(), 2);
    assert_eq!(tree.node(a).subtree_topics(), 2);
    assert_eq!(tree.stats().topics, 3);
    assert_eq!(tree.stats().stored_messages, 3);

    // Removing the last topic below `b` removes `b` as well.
    tree.remove(tree.find("a/b/d").unwrap(), false);
    assert_eq!(tree.find("a/b"), None);
    assert_eq!(render(&mut tree), "-a\n   e\n f");
}

#[test]
fn removing_a_value_keeps_levels_that_still_have_children() {
    let mut tree = tree_with(&["a", "a/b"]);
    let a = tree.find("a").unwrap();
    tree.remove(a, false);
    assert!(tree.contains(a));
    assert!(tree.node(a).last().is_none());
    assert_eq!(tree.node(a).message_count(), 0);
    assert_eq!(tree.topics_with_values(a, false), Vec::<String>::new());
    assert_eq!(tree.topics_with_values(a, true), ["a/b"]);
    assert_eq!(tree.stats().topics, 1);
}

#[test]
fn recursive_removal_takes_the_whole_subtree() {
    let mut tree = tree_with(&["a/b/c", "a/b/d", "a", "f"]);
    let a = tree.find("a").unwrap();
    assert_eq!(tree.topics_with_values(a, true), ["a", "a/b/c", "a/b/d"]);
    tree.remove(a, true);
    assert_eq!(render(&mut tree), " f");
    assert_eq!(tree.stats().topics, 1);
    assert_eq!(tree.stats().stored_messages, 1);
    assert!(!tree.contains(a));

    // Freed slots are reused, and stale handles from before do not resurrect anything.
    tree.insert(Message::new("g/h", "v"));
    assert_eq!(render(&mut tree), " f\n+g");
    assert_eq!(tree.stats().topics, 2);

    tree.remove(TopicTree::ROOT, false);
    assert_eq!(tree.stats().topics, 2, "the root itself holds no value");
    tree.remove(TopicTree::ROOT, true);
    assert_eq!(tree.rows().len(), 0);
    assert_eq!(tree.stats().stored_bytes, 0);
}

#[test]
fn budget_eviction_survives_removed_topics() {
    let mut tree = TopicTree::new(HistoryLimits { per_topic: 100, total_bytes: 4_000 });
    for index in 0..20 {
        tree.insert(Message::new("gone", vec![b'x'; 100]));
        tree.insert(Message::new("kept", index.to_string()));
    }
    tree.remove(tree.find("gone").unwrap(), false);
    for index in 0..200 {
        tree.insert(Message::new("kept", vec![b'y'; 100 + index % 3]));
    }
    assert!(tree.stats().stored_bytes <= 4_000);
    assert!(tree.node(tree.find("kept").unwrap()).history().len() > 1);
}

#[test]
fn clear_keeps_settings() {
    let mut tree = TopicTree::new(HistoryLimits { per_topic: 5, total_bytes: 999 });
    tree.insert(Message::new("a", "v"));
    tree.set_order(TreeOrder::Messages);
    tree.set_filter(Filter::new("a", false));
    tree.clear();
    assert_eq!(tree.stats().messages, 0);
    assert_eq!(tree.rows().len(), 0);
    assert_eq!(tree.order(), TreeOrder::Messages);
    assert!(!tree.filter().is_empty());
    assert_eq!(tree.limits(), HistoryLimits { per_topic: 5, total_bytes: 999 });
    assert!(tree.contains(TopicTree::ROOT));
}

#[test]
fn subtree_lists_parents_before_children() {
    let tree = tree_with(&["a/b/c", "a/d"]);
    let names: Vec<String> = tree.subtree(tree.find("a").unwrap()).into_iter().map(|id| tree.topic(id)).collect();
    assert_eq!(names, ["a", "a/b", "a/b/c", "a/d"]);
}

/// The scale target: 50k topics and a million messages stay within the memory budget and finish quickly.
#[test]
fn fifty_thousand_topics_and_a_million_messages_stay_within_budget() {
    const TOPICS: usize = 50_000;
    const ROUNDS: usize = 20;
    let budget = 48 * 1024 * 1024;
    let topics: Vec<String> =
        (0..TOPICS).map(|index| format!("site/{}/device/{}/sensor/{}", index % 10, index % 500, index)).collect();
    let payload = Bytes::from_static(br#"{"value":21.5,"unit":"C","ok":true}"#);
    let now = Timestamp::now();

    let started = Instant::now();
    let mut tree = TopicTree::new(HistoryLimits { per_topic: 100, total_bytes: budget });
    let mut last = TopicTree::ROOT;
    for _ in 0..ROUNDS {
        for topic in &topics {
            last = tree.insert(Message { received_at: now, ..Message::new(topic.clone(), payload.clone()) });
        }
    }
    tree.set_expanded_recursive(TopicTree::ROOT, true);
    let rows = tree.rows().len();
    let elapsed = started.elapsed();

    let stats = tree.stats();
    assert_eq!(stats.topics, TOPICS);
    assert_eq!(stats.messages, (TOPICS * ROUNDS) as u64);
    assert!(stats.stored_bytes <= budget, "{stats:?}");
    assert!(stats.stored_messages >= TOPICS, "every topic keeps its latest value: {stats:?}");
    assert_eq!(rows, TOPICS + 1 + 10 + 10 + 500 + 500, "all levels are visible when expanded");
    assert_ne!(last, NodeId::clone(&TopicTree::ROOT));
    assert!(elapsed.as_secs() < 120, "took {elapsed:?}");
}

#[test]
fn count_driven_reordering_is_throttled() {
    let mut tree = TopicTree::default();
    tree.set_refresh_interval(Duration::from_secs(3600));
    tree.set_order(TreeOrder::Messages);
    tree.insert(Message::new("a", "v"));
    tree.insert(Message::new("b", "v"));
    assert_eq!(render(&mut tree), " a\n b");
    assert!(!tree.rows_pending());

    for _ in 0..5 {
        tree.insert(Message::new("b", "v"));
    }
    assert!(tree.rows_pending());
    assert_eq!(render(&mut tree), " a\n b", "the order is not recomputed for every message");
    assert_eq!(tree.cached_rows().len(), 2);

    // New topics and view changes still show up at once.
    tree.insert(Message::new("c", "v"));
    assert_eq!(render(&mut tree), " b\n a\n c");
    assert!(!tree.rows_pending());

    tree.insert(Message::new("c", "v"));
    tree.insert(Message::new("c", "v"));
    tree.set_refresh_interval(Duration::ZERO);
    assert_eq!(render(&mut tree), " b\n c\n a");

    tree.set_refresh_interval(Duration::from_secs(3600));
    tree.clear();
    tree.insert(Message::new("x", "v"));
    tree.insert(Message::new("x", "v"));
    assert!(tree.rows_pending(), "clearing keeps the interval");
}
