use crate::decode::{Decoder, Format};
use crate::diff::{ChangeKind, diff_decoded, diff_lines, summary};

#[test]
fn identical_texts_have_no_changes() {
    let lines = diff_lines("a\nb", "a\nb");
    assert!(lines.iter().all(|line| line.kind == ChangeKind::Equal));
    assert_eq!(summary(&lines), (0, 0));
}

#[test]
fn changed_lines_are_marked() {
    let lines = diff_lines("a\nb\nc\n", "a\nx\nc\nd\n");
    let rendered: Vec<(ChangeKind, &str)> = lines.iter().map(|line| (line.kind, line.text.as_str())).collect();
    assert_eq!(
        rendered,
        [
            (ChangeKind::Equal, "a"),
            (ChangeKind::Removed, "b"),
            (ChangeKind::Added, "x"),
            (ChangeKind::Equal, "c"),
            (ChangeKind::Added, "d"),
        ]
    );
    assert_eq!(summary(&lines), (2, 1));
}

#[test]
fn json_is_compared_key_by_key() {
    let decoder = Decoder::new();
    let old = decoder.decode("t", br#"{"temp":21.5,"unit":"C"}"#, Format::Auto);
    let new = decoder.decode("t", br#"{"temp":22.0,"unit":"C"}"#, Format::Auto);
    let lines = diff_decoded(&old, &new);
    assert_eq!(summary(&lines), (1, 1));
    assert!(lines.iter().any(|line| line.kind == ChangeKind::Added && line.text.contains("22.0")));
    assert!(lines.iter().any(|line| line.kind == ChangeKind::Equal && line.text.contains("unit")));
}
