use crate::highlight::{Token, json_tokens};

fn tokens(text: &str) -> Vec<(Token, &str)> {
    json_tokens(text).into_iter().map(|(token, range)| (token, &text[range])).collect()
}

#[test]
fn json_is_split_into_kinds() {
    assert_eq!(
        tokens(r#"{"a": [1, -2.5e3, true, null, "x\"y"]}"#),
        [
            (Token::Punctuation, "{"),
            (Token::Key, "\"a\""),
            (Token::Punctuation, ":"),
            (Token::Other, " "),
            (Token::Punctuation, "["),
            (Token::Number, "1"),
            (Token::Punctuation, ","),
            (Token::Other, " "),
            (Token::Number, "-2.5e3"),
            (Token::Punctuation, ","),
            (Token::Other, " "),
            (Token::Literal, "true"),
            (Token::Punctuation, ","),
            (Token::Other, " "),
            (Token::Literal, "null"),
            (Token::Punctuation, ","),
            (Token::Other, " "),
            (Token::String, "\"x\\\"y\""),
            (Token::Punctuation, "]"),
            (Token::Punctuation, "}"),
        ]
    );
}

#[test]
fn keys_are_recognised_across_whitespace() {
    assert_eq!(tokens("\"k\" \n : false")[0], (Token::Key, "\"k\""));
    assert_eq!(tokens("\"k\"")[0], (Token::String, "\"k\""));
}

#[test]
fn every_byte_is_covered_even_for_non_json() {
    for text in
        ["", "plain text", "héllo wörld ✓", "\"unterminated", "\"ends with escape\\", "{\"a\":", "nul", "trueish"]
    {
        let ranges = json_tokens(text);
        let mut position = 0;
        for (_, range) in &ranges {
            assert_eq!(range.start, position, "gap in {text:?}");
            assert!(text.is_char_boundary(range.end), "split character in {text:?}");
            position = range.end;
        }
        assert_eq!(position, text.len(), "{text:?}");
    }
    assert_eq!(tokens("plain text"), [(Token::Other, "plain text")]);
    assert_eq!(tokens("trueish"), [(Token::Literal, "true"), (Token::Other, "ish")]);
}
