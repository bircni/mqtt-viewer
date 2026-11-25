//! Line-based differences between two payloads.

use similar::{ChangeTag, TextDiff};

use crate::decode::Decoded;

/// What happened to a line between the old and the new text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    /// Present in both.
    Equal,
    /// Only in the new text.
    Added,
    /// Only in the old text.
    Removed,
}

/// One line of a diff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLine {
    /// Whether the line was kept, added or removed.
    pub kind: ChangeKind,
    /// The line without its terminator.
    pub text: String,
}

/// Diffs two texts line by line.
#[must_use]
pub fn diff_lines(old: &str, new: &str) -> Vec<DiffLine> {
    TextDiff::from_lines(old, new)
        .iter_all_changes()
        .map(|change| DiffLine {
            kind: match change.tag() {
                ChangeTag::Equal => ChangeKind::Equal,
                ChangeTag::Insert => ChangeKind::Added,
                ChangeTag::Delete => ChangeKind::Removed,
            },
            text: change.value().trim_end_matches(['\n', '\r']).to_owned(),
        })
        .collect()
}

/// Diffs two decoded payloads by their displayed form, so JSON is compared pretty-printed and key by key.
#[must_use]
pub fn diff_decoded(old: &Decoded, new: &Decoded) -> Vec<DiffLine> {
    diff_lines(&old.display(), &new.display())
}

/// Number of added and removed lines.
#[must_use]
pub fn summary(lines: &[DiffLine]) -> (usize, usize) {
    lines.iter().fold((0, 0), |(added, removed), line| match line.kind {
        ChangeKind::Equal => (added, removed),
        ChangeKind::Added => (added + 1, removed),
        ChangeKind::Removed => (added, removed + 1),
    })
}
