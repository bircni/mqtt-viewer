//! Pretty-printing JSON text without parsing it into a value first.
//!
//! Going through [`serde_json::Value`] would reorder or merge what the sender wrote: duplicate keys collapse to
//! the last one and numbers like `20.50` lose their trailing zero. Reformatting the text keeps every byte of
//! content and only changes whitespace.

/// Re-indents `text` with two spaces per level, the layout of [`serde_json::to_string_pretty`]. Expects valid
/// JSON; anything else comes out reformatted but otherwise unchanged.
#[must_use]
pub fn pretty(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + text.len() / 2);
    let mut depth = 0_usize;
    let mut chars = text.chars().peekable();
    let newline = |out: &mut String, depth: usize| {
        out.push('\n');
        for _ in 0..depth {
            out.push_str("  ");
        }
    };
    while let Some(character) = chars.next() {
        match character {
            '"' => {
                out.push('"');
                while let Some(inner) = chars.next() {
                    out.push(inner);
                    match inner {
                        '\\' => out.extend(chars.next()),
                        '"' => break,
                        _ => {}
                    }
                }
            }
            '{' | '[' => {
                out.push(character);
                skip_whitespace(&mut chars);
                if matches!(chars.peek(), Some('}' | ']')) {
                    out.extend(chars.next());
                } else {
                    depth += 1;
                    newline(&mut out, depth);
                }
            }
            '}' | ']' => {
                depth = depth.saturating_sub(1);
                newline(&mut out, depth);
                out.push(character);
            }
            ',' => {
                out.push(',');
                newline(&mut out, depth);
            }
            ':' => out.push_str(": "),
            whitespace if whitespace.is_whitespace() => {}
            other => out.push(other),
        }
    }
    out
}

fn skip_whitespace(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) {
    while chars.next_if(|character| character.is_whitespace()).is_some() {}
}
