//! The topic tree: every topic seen so far, its message history and the flattened row list the UI draws.
//!
//! Nodes live in an arena and are addressed by [`NodeId`]. The tree is owned by one thread (the UI thread) and
//! mutated in place, so there is no locking. History is bounded twice: per topic by a message cap and globally
//! by a byte budget that evicts the oldest messages first while always keeping the latest value of every topic.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use bytes::Bytes;
use jiff::Timestamp;

use crate::config::TreeOrder;
use crate::message::{Message, Properties, QoS};

/// Handle to a node of a [`TopicTree`]. Only valid for the tree that issued it and until the node is removed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId(usize);

/// One stored message of a topic. The topic itself is implied by the node holding the entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryEntry {
    /// Raw payload.
    pub payload: Bytes,
    /// Delivery QoS.
    pub qos: QoS,
    /// Whether the broker flagged it as retained.
    pub retain: bool,
    /// When it was received.
    pub received_at: Timestamp,
    /// MQTT 5 properties, if any.
    pub properties: Option<Box<Properties>>,
    seq: u64,
}

impl HistoryEntry {
    /// Approximate number of bytes this entry keeps alive.
    fn cost(&self) -> usize {
        let properties = self.properties.as_ref().map_or(0, |properties| {
            size_of::<Properties>()
                + properties.user_properties.iter().map(|(key, value)| key.len() + value.len()).sum::<usize>()
                + properties.correlation_data.as_ref().map_or(0, Bytes::len)
        });
        size_of::<Self>() + self.payload.len() + properties
    }

    /// A number that is unique per stored message and grows with arrival order.
    #[must_use]
    pub const fn seq(&self) -> u64 {
        self.seq
    }

    /// Rebuilds the full message for `topic`.
    #[must_use]
    pub fn to_message(&self, topic: impl Into<String>) -> Message {
        Message {
            topic: topic.into(),
            payload: self.payload.clone(),
            qos: self.qos,
            retain: self.retain,
            received_at: self.received_at,
            properties: self.properties.clone(),
        }
    }
}

/// A topic level.
#[derive(Debug)]
pub struct Node {
    name: Box<str>,
    parent: Option<NodeId>,
    /// Sorted by name.
    children: Vec<NodeId>,
    history: VecDeque<HistoryEntry>,
    message_count: u64,
    subtree_messages: u64,
    subtree_topics: usize,
    expanded: bool,
    last_activity: Option<Timestamp>,
    alive: bool,
}

impl Node {
    fn new(name: &str, parent: Option<NodeId>) -> Self {
        Self {
            name: name.into(),
            parent,
            children: Vec::new(),
            history: VecDeque::new(),
            message_count: 0,
            subtree_messages: 0,
            subtree_topics: 0,
            expanded: false,
            last_activity: None,
            alive: true,
        }
    }

    /// The level name (one segment of the topic).
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Parent node; `None` for the root.
    #[must_use]
    pub const fn parent(&self) -> Option<NodeId> {
        self.parent
    }

    /// Child nodes, sorted by name.
    #[must_use]
    pub fn children(&self) -> &[NodeId] {
        &self.children
    }

    /// Stored messages of exactly this topic, oldest first.
    #[must_use]
    pub const fn history(&self) -> &VecDeque<HistoryEntry> {
        &self.history
    }

    /// The most recent message of exactly this topic.
    #[must_use]
    pub fn last(&self) -> Option<&HistoryEntry> {
        self.history.back()
    }

    /// Messages received on exactly this topic (including ones no longer in the history).
    #[must_use]
    pub const fn message_count(&self) -> u64 {
        self.message_count
    }

    /// Messages received on this topic and everything below it.
    #[must_use]
    pub const fn subtree_messages(&self) -> u64 {
        self.subtree_messages
    }

    /// Number of topics with a value in this subtree, this node included.
    #[must_use]
    pub const fn subtree_topics(&self) -> usize {
        self.subtree_topics
    }

    /// Whether the node is expanded in the UI.
    #[must_use]
    pub const fn is_expanded(&self) -> bool {
        self.expanded
    }

    /// When a message last arrived here or below.
    #[must_use]
    pub const fn last_activity(&self) -> Option<Timestamp> {
        self.last_activity
    }
}

/// One visible line of the tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    /// The node shown on this line.
    pub id: NodeId,
    /// Indentation level, 0 for top-level topics.
    pub depth: usize,
    /// Whether the node has (visible) children.
    pub has_children: bool,
    /// Whether the children are currently shown.
    pub expanded: bool,
}

/// Bounds for kept history.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HistoryLimits {
    /// Maximum number of messages kept per topic (at least 1).
    pub per_topic: usize,
    /// Maximum approximate number of bytes kept over all topics.
    pub total_bytes: usize,
}

impl Default for HistoryLimits {
    fn default() -> Self {
        Self { per_topic: 200, total_bytes: 256 * 1024 * 1024 }
    }
}

/// Restricts the visible tree to topics matching a text.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Filter {
    text: String,
    match_payload: bool,
    /// Whether `text` is an MQTT topic filter (`home/+/temperature`, `devices/#`) rather than a substring.
    wildcard: bool,
}

impl Filter {
    /// A case-insensitive filter on the topic and, when `match_payload` is set, on the latest payload.
    ///
    /// Text with a `+` or `#` level that forms a valid topic filter matches topics the way a subscription
    /// would, without looking at payloads; anything else matches as a substring of the topic.
    #[must_use]
    pub fn new(text: &str, match_payload: bool) -> Self {
        let text = text.to_lowercase();
        let wildcard =
            text.split('/').any(|level| level == "+" || level == "#") && crate::topic::validate_filter(&text).is_ok();
        Self { text, match_payload, wildcard }
    }

    /// Whether the text is used as an MQTT topic filter.
    #[must_use]
    pub const fn is_wildcard(&self) -> bool {
        self.wildcard
    }

    /// Whether the lowercased topic `path` matches.
    fn matches_topic(&self, path: &str) -> bool {
        if self.wildcard { crate::topic::matches(&self.text, path) } else { path.contains(&self.text) }
    }

    /// Whether the filter lets everything through.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    fn matches_payload(&self, payload: &[u8]) -> bool {
        let needle = self.text.as_bytes();
        payload.len() >= needle.len() && payload.windows(needle.len()).any(|window| window.eq_ignore_ascii_case(needle))
    }
}

/// Totals over the whole tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TreeStats {
    /// Topics that currently hold a value.
    pub topics: usize,
    /// Messages received since the tree was created or cleared.
    pub messages: u64,
    /// Messages currently kept in history.
    pub stored_messages: usize,
    /// Approximate bytes kept in history.
    pub stored_bytes: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Visibility {
    Hidden,
    /// Shown only because something below matches; always drawn expanded.
    ViaDescendant,
    /// Matches itself.
    Matched,
}

/// All topics seen on a connection.
#[derive(Debug)]
pub struct TopicTree {
    nodes: Vec<Node>,
    free: Vec<NodeId>,
    limits: HistoryLimits,
    order: TreeOrder,
    filter: Filter,
    rows: Vec<Row>,
    /// The rows no longer match the tree's structure or view settings; rebuilt on the next read.
    rows_dirty: bool,
    /// Only counts or payloads changed, which can reorder or re-filter rows; rebuilt at most once per
    /// [`Self::refresh_interval`].
    rows_stale: bool,
    rows_built_at: Instant,
    refresh_interval: Duration,
    /// Levels created at a depth below this start out expanded.
    auto_expand: usize,
    next_seq: u64,
    /// Arrival order of stored entries, for budget eviction. May contain entries already evicted by the cap.
    arrival: VecDeque<(NodeId, u64)>,
    stats: TreeStats,
}

impl Default for TopicTree {
    fn default() -> Self {
        Self::new(HistoryLimits::default())
    }
}

impl TopicTree {
    /// The root node; it is never shown and has an empty name.
    pub const ROOT: NodeId = NodeId(0);
    /// How often rows are re-sorted and re-filtered while only message counts and payloads change.
    pub const DEFAULT_REFRESH_INTERVAL: Duration = Duration::from_millis(250);

    /// An empty tree.
    #[must_use]
    pub fn new(limits: HistoryLimits) -> Self {
        Self {
            nodes: vec![Node::new("", None)],
            free: Vec::new(),
            limits: sanitize(limits),
            order: TreeOrder::Alphabetical,
            filter: Filter::default(),
            rows: Vec::new(),
            rows_dirty: true,
            rows_stale: false,
            rows_built_at: Instant::now(),
            refresh_interval: Self::DEFAULT_REFRESH_INTERVAL,
            auto_expand: 0,
            next_seq: 0,
            arrival: VecDeque::new(),
            stats: TreeStats::default(),
        }
    }

    /// Removes every topic.
    pub fn clear(&mut self) {
        *self = Self {
            order: self.order,
            filter: self.filter.clone(),
            refresh_interval: self.refresh_interval,
            auto_expand: self.auto_expand,
            ..Self::new(self.limits)
        };
    }

    /// The node behind `id`.
    ///
    /// # Panics
    /// Panics when `id` was not issued by this tree.
    #[must_use]
    pub fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id.0]
    }

    /// Whether `id` still refers to a node of this tree.
    #[must_use]
    pub fn contains(&self, id: NodeId) -> bool {
        self.nodes.get(id.0).is_some_and(|node| node.alive)
    }

    /// Totals over the whole tree.
    #[must_use]
    pub const fn stats(&self) -> TreeStats {
        self.stats
    }

    /// Current history bounds.
    #[must_use]
    pub const fn limits(&self) -> HistoryLimits {
        self.limits
    }

    /// Changes the history bounds and evicts whatever no longer fits.
    pub fn set_limits(&mut self, limits: HistoryLimits) {
        self.limits = sanitize(limits);
        for index in 0..self.nodes.len() {
            self.enforce_cap(NodeId(index));
        }
        self.enforce_budget();
    }

    /// Full topic name of a node.
    #[must_use]
    pub fn topic(&self, id: NodeId) -> String {
        let mut levels = Vec::new();
        let mut current = id;
        while let Some(parent) = self.nodes[current.0].parent {
            levels.push(&*self.nodes[current.0].name);
            current = parent;
        }
        levels.reverse();
        levels.join("/")
    }

    /// The node for `topic`, if that exact topic level exists.
    #[must_use]
    pub fn find(&self, topic: &str) -> Option<NodeId> {
        topic.split('/').try_fold(Self::ROOT, |parent, level| self.child(parent, level).ok())
    }

    fn child(&self, parent: NodeId, name: &str) -> Result<NodeId, usize> {
        let children = &self.nodes[parent.0].children;
        children.binary_search_by(|child| (*self.nodes[child.0].name).cmp(name)).map(|index| children[index])
    }

    fn allocate(&mut self, name: &str, parent: NodeId) -> NodeId {
        let node = Node::new(name, Some(parent));
        if let Some(id) = self.free.pop() {
            self.nodes[id.0] = node;
            id
        } else {
            self.nodes.push(node);
            NodeId(self.nodes.len() - 1)
        }
    }

    /// Adds a message, creating its topic levels as needed, and returns the topic's node.
    pub fn insert(&mut self, message: Message) -> NodeId {
        let mut current = Self::ROOT;
        for (depth, level) in message.topic.split('/').enumerate() {
            current = match self.child(current, level) {
                Ok(existing) => existing,
                Err(position) => {
                    let created = self.allocate(level, current);
                    self.nodes[created.0].expanded = depth < self.auto_expand;
                    self.nodes[current.0].children.insert(position, created);
                    self.rows_dirty = true;
                    created
                }
            };
        }
        let leaf = current;
        let is_new_topic = self.nodes[leaf.0].history.is_empty();
        let entry = HistoryEntry {
            payload: message.payload,
            qos: message.qos,
            retain: message.retain,
            received_at: message.received_at,
            properties: message.properties,
            seq: self.next_seq,
        };
        self.next_seq += 1;
        self.stats.stored_bytes += entry.cost();
        self.stats.stored_messages += 1;
        self.stats.messages += 1;
        if is_new_topic {
            self.stats.topics += 1;
        }
        self.arrival.push_back((leaf, entry.seq));
        let received_at = entry.received_at;
        self.nodes[leaf.0].history.push_back(entry);
        self.nodes[leaf.0].message_count += 1;

        let mut ancestor = Some(leaf);
        while let Some(id) = ancestor {
            let node = &mut self.nodes[id.0];
            node.subtree_messages += 1;
            node.subtree_topics += usize::from(is_new_topic);
            node.last_activity = Some(node.last_activity.map_or(received_at, |previous| previous.max(received_at)));
            ancestor = node.parent;
        }

        if self.order != TreeOrder::Alphabetical || !self.filter.is_empty() {
            // Counts drive the order and payloads the filter, so the rows may have moved. Rebuilding for every
            // message would cost a full sort per frame on a busy broker, so this is picked up on a timer.
            self.rows_stale = true;
        }
        self.enforce_cap(leaf);
        self.enforce_budget();
        leaf
    }

    fn drop_front(&mut self, id: NodeId) {
        if let Some(entry) = self.nodes[id.0].history.pop_front() {
            self.stats.stored_bytes = self.stats.stored_bytes.saturating_sub(entry.cost());
            self.stats.stored_messages = self.stats.stored_messages.saturating_sub(1);
        }
    }

    fn enforce_cap(&mut self, id: NodeId) {
        while self.nodes[id.0].history.len() > self.limits.per_topic {
            self.drop_front(id);
        }
    }

    fn enforce_budget(&mut self) {
        while self.stats.stored_bytes > self.limits.total_bytes {
            let Some((id, seq)) = self.arrival.pop_front() else { break };
            let node = &self.nodes[id.0];
            // Skip entries the cap or a removal already dropped, and never evict a topic's latest value.
            if node.alive && node.history.len() > 1 && node.history.front().is_some_and(|entry| entry.seq == seq) {
                self.drop_front(id);
            }
        }
        if self.arrival.len() > self.stats.stored_messages.saturating_mul(2) + 1024 {
            let nodes = &self.nodes;
            self.arrival.retain(|(id, seq)| {
                let node = &nodes[id.0];
                node.alive && node.history.len() > 1 && node.history.front().is_some_and(|entry| entry.seq <= *seq)
            });
        }
    }

    /// How many levels of the tree open by themselves: topics created from now on start out expanded when
    /// they are less than `depth` levels deep. 0 leaves everything collapsed.
    pub const fn set_auto_expand(&mut self, depth: usize) {
        self.auto_expand = depth;
    }

    /// Expands or collapses a node.
    pub fn set_expanded(&mut self, id: NodeId, expanded: bool) {
        if self.nodes[id.0].expanded != expanded {
            self.nodes[id.0].expanded = expanded;
            self.rows_dirty = true;
        }
    }

    /// Expands or collapses a node and everything below it.
    pub fn set_expanded_recursive(&mut self, id: NodeId, expanded: bool) {
        for descendant in self.subtree(id) {
            self.nodes[descendant.0].expanded = expanded;
        }
        self.rows_dirty = true;
    }

    /// Expands all ancestors of `id` so that it becomes visible.
    pub fn reveal(&mut self, id: NodeId) {
        let mut ancestor = self.nodes[id.0].parent;
        while let Some(current) = ancestor {
            self.set_expanded(current, true);
            ancestor = self.nodes[current.0].parent;
        }
    }

    /// Current sibling ordering.
    #[must_use]
    pub const fn order(&self) -> TreeOrder {
        self.order
    }

    /// Changes the sibling ordering.
    pub fn set_order(&mut self, order: TreeOrder) {
        if self.order != order {
            self.order = order;
            self.rows_dirty = true;
        }
    }

    /// Current filter.
    #[must_use]
    pub const fn filter(&self) -> &Filter {
        &self.filter
    }

    /// Restricts the visible rows; an empty filter shows everything.
    pub fn set_filter(&mut self, filter: Filter) {
        if self.filter != filter {
            self.filter = filter;
            self.rows_dirty = true;
        }
    }

    /// `id` followed by all of its descendants, parents before children.
    #[must_use]
    pub fn subtree(&self, id: NodeId) -> Vec<NodeId> {
        let mut out = Vec::new();
        let mut stack = vec![id];
        while let Some(current) = stack.pop() {
            out.push(current);
            stack.extend(self.nodes[current.0].children.iter().rev());
        }
        out
    }

    /// Full names of the topics holding a value at `id` or, when `recursive`, anywhere below it.
    #[must_use]
    pub fn topics_with_values(&self, id: NodeId, recursive: bool) -> Vec<String> {
        let candidates = if recursive { self.subtree(id) } else { vec![id] };
        candidates
            .into_iter()
            .filter(|candidate| !self.nodes[candidate.0].history.is_empty())
            .map(|candidate| self.topic(candidate))
            .collect()
    }

    /// Topics that hold a value and match the filter themselves (not only through a descendant), in tree order.
    /// Empty while no filter is set.
    #[must_use]
    pub fn matching_topics(&self) -> Vec<String> {
        let Some(visibility) = self.visibility() else { return Vec::new() };
        self.subtree(Self::ROOT)
            .into_iter()
            .filter(|id| visibility[id.0] == Visibility::Matched && !self.nodes[id.0].history.is_empty())
            .map(|id| self.topic(id))
            .collect()
    }

    /// Forgets the value and history of `id` (and of everything below it when `recursive`), then removes the
    /// levels that became empty.
    pub fn remove(&mut self, id: NodeId, recursive: bool) {
        if id == Self::ROOT {
            // The root holds no value of its own; removing everything below it empties the tree.
            if recursive {
                self.clear();
            }
            return;
        }
        let targets = if recursive { self.subtree(id) } else { vec![id] };
        for target in targets {
            let node = &mut self.nodes[target.0];
            let removed_messages = node.message_count;
            let had_value = !node.history.is_empty();
            let freed: usize = node.history.iter().map(HistoryEntry::cost).sum();
            let stored = node.history.len();
            node.history = VecDeque::new();
            node.message_count = 0;
            self.stats.stored_bytes = self.stats.stored_bytes.saturating_sub(freed);
            self.stats.stored_messages = self.stats.stored_messages.saturating_sub(stored);
            self.stats.topics = self.stats.topics.saturating_sub(usize::from(had_value));
            let mut ancestor = Some(target);
            while let Some(current) = ancestor {
                let node = &mut self.nodes[current.0];
                node.subtree_messages = node.subtree_messages.saturating_sub(removed_messages);
                node.subtree_topics = node.subtree_topics.saturating_sub(usize::from(had_value));
                ancestor = node.parent;
            }
        }
        self.prune(id);
        self.rows_dirty = true;
    }

    /// Removes `id` and its ancestors while they hold no value anywhere below them.
    fn prune(&mut self, id: NodeId) {
        let mut current = id;
        while let Some(parent) = self.nodes[current.0].parent {
            if self.nodes[current.0].subtree_topics > 0 {
                break;
            }
            for dead in self.subtree(current) {
                let node = &mut self.nodes[dead.0];
                node.alive = false;
                node.children = Vec::new();
                self.free.push(dead);
            }
            self.nodes[parent.0].children.retain(|child| *child != current);
            current = parent;
        }
    }

    /// The visible rows, top to bottom. Rebuilt lazily after structural changes.
    pub fn rows(&mut self) -> &[Row] {
        if self.rows_dirty || (self.rows_stale && self.rows_built_at.elapsed() >= self.refresh_interval) {
            self.rebuild_rows();
        }
        &self.rows
    }

    /// Whether the rows are waiting for their next periodic rebuild; the caller should read them again soon.
    #[must_use]
    pub const fn rows_pending(&self) -> bool {
        self.rows_stale
    }

    /// Changes how often rows follow count and payload changes. Zero means "on every read".
    pub const fn set_refresh_interval(&mut self, interval: Duration) {
        self.refresh_interval = interval;
    }

    /// The rows as of the last call to [`Self::rows`]. Lets a renderer read rows and nodes at the same time.
    #[must_use]
    pub fn cached_rows(&self) -> &[Row] {
        &self.rows
    }

    /// Position of `id` among the visible rows.
    pub fn row_of(&mut self, id: NodeId) -> Option<usize> {
        self.rows().iter().position(|row| row.id == id)
    }

    fn ordered_children(&self, id: NodeId) -> Vec<NodeId> {
        let mut children = self.nodes[id.0].children.clone();
        match self.order {
            TreeOrder::Alphabetical => {}
            // The sort is stable, so ties stay alphabetical.
            TreeOrder::Messages => {
                children.sort_by_key(|child| std::cmp::Reverse(self.nodes[child.0].subtree_messages));
            }
            TreeOrder::Topics => children.sort_by_key(|child| std::cmp::Reverse(self.nodes[child.0].subtree_topics)),
        }
        children
    }

    fn visibility(&self) -> Option<Vec<Visibility>> {
        if self.filter.is_empty() {
            return None;
        }
        let mut visibility = vec![Visibility::Hidden; self.nodes.len()];
        let mut path = String::new();
        self.mark_visible(Self::ROOT, &mut path, &mut visibility);
        Some(visibility)
    }

    fn mark_visible(&self, id: NodeId, path: &mut String, visibility: &mut [Visibility]) -> bool {
        let node = &self.nodes[id.0];
        let matched = id != Self::ROOT
            && (self.filter.matches_topic(path)
                || (!self.filter.wildcard
                    && self.filter.match_payload
                    && node.last().is_some_and(|entry| self.filter.matches_payload(&entry.payload))));
        let mut any_child = false;
        let base = path.len();
        for child in &node.children {
            if id != Self::ROOT {
                path.push('/');
            }
            path.extend(self.nodes[child.0].name.chars().flat_map(char::to_lowercase));
            any_child |= self.mark_visible(*child, path, visibility);
            path.truncate(base);
        }
        visibility[id.0] = if matched {
            Visibility::Matched
        } else if any_child {
            Visibility::ViaDescendant
        } else {
            Visibility::Hidden
        };
        matched || any_child
    }

    fn rebuild_rows(&mut self) {
        let visibility = self.visibility();
        let is_visible = |id: NodeId| visibility.as_ref().is_none_or(|states| states[id.0] != Visibility::Hidden);
        let mut rows = std::mem::take(&mut self.rows);
        rows.clear();
        let mut stack: Vec<(NodeId, usize)> =
            self.ordered_children(Self::ROOT).into_iter().rev().map(|child| (child, 0)).collect();
        while let Some((id, depth)) = stack.pop() {
            if !is_visible(id) {
                continue;
            }
            let node = &self.nodes[id.0];
            let has_children = node.children.iter().any(|child| is_visible(*child));
            let forced = visibility.as_ref().is_some_and(|states| states[id.0] == Visibility::ViaDescendant);
            let expanded = has_children && (node.expanded || forced);
            rows.push(Row { id, depth, has_children, expanded });
            if expanded {
                stack.extend(self.ordered_children(id).into_iter().rev().map(|child| (child, depth + 1)));
            }
        }
        self.rows = rows;
        self.rows_dirty = false;
        self.rows_stale = false;
        self.rows_built_at = Instant::now();
    }
}

const fn sanitize(limits: HistoryLimits) -> HistoryLimits {
    HistoryLimits {
        per_topic: if limits.per_topic == 0 { 1 } else { limits.per_topic },
        total_bytes: limits.total_bytes,
    }
}
