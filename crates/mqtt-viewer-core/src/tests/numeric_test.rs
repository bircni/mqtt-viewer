use serde_json::json;

use crate::decode::{Decoder, Format};
use crate::numeric::{chartable_paths, extract, numeric_paths, parse_number, value_as_number};

#[test]
fn plain_numbers_and_switch_words_parse() {
    assert_eq!(parse_number(" 21.5\n"), Some(21.5));
    assert_eq!(parse_number("-3"), Some(-3.0));
    assert_eq!(parse_number("1e3"), Some(1000.0));
    assert_eq!(parse_number("ON"), Some(1.0));
    assert_eq!(parse_number("false"), Some(0.0));
    assert_eq!(parse_number("warm"), None);
    assert_eq!(parse_number("NaN"), None);
    assert_eq!(parse_number("inf"), None);
    assert_eq!(parse_number(""), None);
}

#[test]
fn json_values_convert() {
    assert_eq!(value_as_number(&json!(4)), Some(4.0));
    assert_eq!(value_as_number(&json!(true)), Some(1.0));
    assert_eq!(value_as_number(&json!("7.5")), Some(7.5));
    assert_eq!(value_as_number(&json!(null)), None);
    assert_eq!(value_as_number(&json!([1])), None);
    assert_eq!(value_as_number(&json!({"a": 1})), None);
}

#[test]
fn paths_list_every_numeric_leaf() {
    let value = json!({"a": 1, "b": {"c": [2, "x", "3"], "d/e": true, "f~g": null}, "s": "text"});
    assert_eq!(numeric_paths(&value, 100), ["/a", "/b/c/0", "/b/c/2", "/b/d~1e"]);
    assert_eq!(numeric_paths(&value, 2), ["/a", "/b/c/0"]);
    for path in numeric_paths(&value, 100) {
        assert!(value.pointer(&path).is_some(), "{path} must resolve");
    }
}

#[test]
fn extraction_follows_pointers() {
    let decoder = Decoder::new();
    let json = decoder.decode("t", br#"{"env":{"temp":21.5},"list":[1,2]}"#, Format::Auto);
    assert_eq!(extract(&json, "/env/temp"), Some(21.5));
    assert_eq!(extract(&json, "/list/1"), Some(2.0));
    assert_eq!(extract(&json, "/missing"), None);
    assert_eq!(extract(&json, ""), None);
    assert_eq!(chartable_paths(&json, 10), ["/env/temp", "/list/0", "/list/1"]);

    let text = decoder.decode("t", b"42", Format::Auto);
    assert_eq!(extract(&text, ""), Some(42.0));
    assert_eq!(extract(&text, "/a"), None);
    assert_eq!(chartable_paths(&text, 10), [""]);

    let words = decoder.decode("t", b"hello", Format::Auto);
    assert_eq!(chartable_paths(&words, 10).len(), 0);
    let hex = decoder.decode("t", b"\x00\x01", Format::Auto);
    assert_eq!(extract(&hex, ""), None);
    assert_eq!(chartable_paths(&hex, 10).len(), 0);
}
