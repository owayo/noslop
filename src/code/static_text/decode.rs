//! エスケープと HTML 実体参照を一件ずつ復号し、原文の範囲を保つ。

use super::{Builder, CodeLanguage};
use crate::diagnostic::Span;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Mode {
    Raw,
    Escaped(CodeLanguage),
    BashDouble,
    SwiftRaw(usize),
    YamlSingle,
    Single(char, char),
    Verbatim,
    Html,
    Css,
}

pub(super) fn append(
    source: &str,
    span: Span,
    mode: Mode,
    builder: &mut Builder,
    out: &mut Vec<crate::document::Block>,
) {
    let s = &source[span.range()];
    let mut at = 0;
    let mut plain = 0;
    while at < s.len() {
        let c = s[at..].chars().next().unwrap();
        let special = match mode {
            Mode::Raw => false,
            Mode::Html => c == '&',
            Mode::Verbatim => s[at..].starts_with("\"\""),
            Mode::YamlSingle => s[at..].starts_with("''"),
            Mode::SwiftRaw(hash) => s[at..].starts_with(&format!("\\{}", "#".repeat(hash))),
            _ => c == '\\',
        };
        if !special {
            at += c.len_utf8();
            continue;
        }
        builder.exact(source, Span::new(span.start + plain, span.start + at));
        if mode == Mode::Escaped(CodeLanguage::Go) && byte_escape(&s[at..]).is_some() {
            let mut bytes = Vec::new();
            let mut spans = Vec::new();
            while let Some((len, byte)) = byte_escape(&s[at..]) {
                bytes.push(byte);
                spans.push(Span::new(span.start + at, span.start + at + len));
                at += len;
            }
            append_bytes(&bytes, &spans, builder, out);
            plain = at;
            continue;
        }
        let decoded = match mode {
            Mode::Html => entity(&s[at..]),
            Mode::Verbatim => Some((2, "\"".to_string())),
            Mode::YamlSingle => Some((2, "'".to_string())),
            Mode::SwiftRaw(hash) => {
                let rest = format!("\\{}", &s[at + 1 + hash..]);
                escape(&rest, CodeLanguage::Swift).map(|(len, v)| (len + hash, v))
            }
            Mode::BashDouble => s[at + 1..]
                .chars()
                .next()
                .filter(|c| matches!(c, '$' | '`' | '"' | '\\' | '\n'))
                .map(|c| {
                    (
                        1 + c.len_utf8(),
                        if c == '\n' {
                            String::new()
                        } else {
                            c.to_string()
                        },
                    )
                }),
            Mode::Single(open, close) => {
                let next = s[at + 1..].chars().next();
                next.filter(|c| *c == '\\' || *c == open || *c == close)
                    .map(|c| (1 + c.len_utf8(), c.to_string()))
            }
            Mode::Css => css_escape(&s[at..]),
            Mode::Escaped(lang) => escape(&s[at..], lang),
            Mode::Raw => None,
        };
        if let Some((len, value)) = decoded {
            builder.opaque(&value, Span::new(span.start + at, span.start + at + len));
            at += len;
        } else if matches!(mode, Mode::Html | Mode::Single(_, _) | Mode::BashDouble) {
            builder.exact(
                source,
                Span::new(span.start + at, span.start + at + c.len_utf8()),
            );
            at += c.len_utf8();
        } else {
            // 不明なエスケープを越えて語句を作らない。
            builder.finish(out);
            at += c.len_utf8();
            if let Some(next) = s[at..].chars().next() {
                at += next.len_utf8();
            }
        }
        plain = at;
    }
    builder.exact(source, Span::new(span.start + plain, span.end));
}

fn entity(s: &str) -> Option<(usize, String)> {
    let end = s.find(';')? + 1;
    if end > 64 || s[..end].contains(char::is_whitespace) {
        return None;
    }
    let entity = &s[..end];
    // Markdown の実体参照だけを渡すため、タグや装飾として読む副作用はない。
    let text: String = pulldown_cmark::Parser::new(entity)
        .filter_map(|e| match e {
            pulldown_cmark::Event::Text(t) => Some(t.into_string()),
            _ => None,
        })
        .collect();
    (text != entity).then_some((end, text))
}

fn hex(s: &str, n: usize) -> Option<u32> {
    let value = s.get(..n)?;
    value
        .chars()
        .all(|c| c.is_ascii_hexdigit())
        .then(|| u32::from_str_radix(value, 16).ok())
        .flatten()
}

fn escape(s: &str, lang: CodeLanguage) -> Option<(usize, String)> {
    use CodeLanguage::*;
    let c = s.get(1..)?.chars().next()?;
    let basic = match c {
        'n' => Some('\n'),
        'r' => Some('\r'),
        't' => Some('\t'),
        'b' => Some('\u{8}'),
        'f' => Some('\u{c}'),
        'v' => Some('\u{b}'),
        'a' => Some('\u{7}'),
        'e' => Some('\u{1b}'),
        '\\' | '\'' | '"' | '`' | '/' | '$' => Some(c),
        _ => None,
    };
    let allowed = match lang {
        Rust => "nrt0\\\"'",
        Java | CSharp | Kotlin => "nrtbf\\\"'",
        Toml => "nrtbf\\\"",
        Go => "abfnrtv\\\"'",
        Swift => "nrt0\\\"'",
        Php => "nrtve\\\"$",
        Python => "abfnrtv\\\"'",
        _ => "abefnrtv\\\"'`/$",
    };
    if let Some(value) = basic.filter(|_| allowed.contains(c)) {
        return Some((2, value.to_string()));
    }
    if c == '0' && matches!(lang, Rust | Swift | JavaScript | TypeScript | Tsx) {
        return Some((2, "\0".to_string()));
    }
    if c == '\n' {
        return Some((2, String::new()));
    }
    if c == '\r' && s.starts_with("\\\r\n") {
        return Some((3, String::new()));
    }
    if c == 'u'
        && s.starts_with("\\u{")
        && matches!(
            lang,
            Rust | JavaScript | TypeScript | Tsx | Ruby | Php | Swift | Lua
        )
    {
        let end = s.find('}')?;
        let value = u32::from_str_radix(s.get(3..end)?, 16).ok()?;
        return Some((end + 1, char::from_u32(value)?.to_string()));
    }
    let (digits, start) = match c {
        'u' if !matches!(lang, Rust | Php | Swift | Lua) => (4, 2),
        'U' if matches!(
            lang,
            Python | Go | C | Cpp | CSharp | Ruby | Bash | Yaml | Toml
        ) =>
        {
            (8, 2)
        }
        'x' if matches!(
            lang,
            Rust | JavaScript
                | TypeScript
                | Tsx
                | Python
                | Go
                | C
                | Cpp
                | CSharp
                | Ruby
                | Php
                | Bash
                | Yaml
                | Lua
        ) =>
        {
            (2, 2)
        }
        _ => (0, 0),
    };
    if digits > 0 {
        let digits = if c == 'x' && matches!(lang, CSharp | C | Cpp | Php | Ruby | Bash) {
            let max = if lang == CSharp {
                4
            } else if matches!(lang, C | Cpp) {
                usize::MAX
            } else {
                2
            };
            let n = s[start..]
                .bytes()
                .take(max)
                .take_while(u8::is_ascii_hexdigit)
                .count();
            if n == 0 {
                return None;
            }
            n
        } else {
            digits
        };
        let value = hex(s.get(start..)?, digits)?;
        let mut len = digits + start;
        let value = if (0xD800..=0xDBFF).contains(&value)
            && matches!(lang, JavaScript | TypeScript | Tsx | Java | CSharp)
            && s.get(len..)?.starts_with("\\u")
        {
            let low = hex(s.get(len + 2..)?, 4)?;
            if !(0xDC00..=0xDFFF).contains(&low) {
                return None;
            }
            len += 6;
            0x10000 + ((value - 0xD800) << 10) + low - 0xDC00
        } else {
            value
        };
        return Some((len, char::from_u32(value)?.to_string()));
    }
    if matches!(c, '0'..='7') && matches!(lang, Python | C | Cpp | Java | Ruby | Php | Bash) {
        let digits = s[1..]
            .bytes()
            .take(3)
            .take_while(|b| matches!(b, b'0'..=b'7'))
            .count();
        let value = u32::from_str_radix(&s[1..1 + digits], 8).ok()?;
        return Some((1 + digits, char::from_u32(value)?.to_string()));
    }
    None
}

fn css_escape(s: &str) -> Option<(usize, String)> {
    let digits = s[1..]
        .bytes()
        .take(6)
        .take_while(u8::is_ascii_hexdigit)
        .count();
    if digits > 0 {
        let value = u32::from_str_radix(&s[1..1 + digits], 16).ok()?;
        let mut len = 1 + digits;
        if let Some(c) = s[len..].chars().next().filter(|c| c.is_whitespace()) {
            len += c.len_utf8();
        }
        return Some((len, char::from_u32(value)?.to_string()));
    }
    let c = s[1..].chars().next()?;
    if c == '\n' {
        return Some((2, String::new()));
    }
    if c == '\r' && s.starts_with("\\\r\n") {
        return Some((3, String::new()));
    }
    Some((1 + c.len_utf8(), c.to_string()))
}

/// HTML は ASCII の空白を折り畳む。JSX は行ごとの字下げと空行を外す。
pub(super) fn markup(
    source: &str,
    span: Span,
    jsx: bool,
    builder: &mut Builder,
    out: &mut Vec<crate::document::Block>,
) {
    if !jsx {
        html_space(source, span, builder, out);
        return;
    }
    let s = &source[span.range()];
    let lines: Vec<_> = s.split('\n').collect();
    let last_nonempty = lines
        .iter()
        .rposition(|l| !l.trim_matches([' ', '\t', '\r']).is_empty());
    let mut offset = span.start;
    for (i, line) in lines.iter().enumerate() {
        let line = line.trim_end_matches('\r');
        let left = if i == 0 {
            0
        } else {
            line.len() - line.trim_start_matches([' ', '\t']).len()
        };
        let right = if i + 1 == lines.len() {
            line.len()
        } else {
            line.trim_end_matches([' ', '\t']).len()
        };
        if left < right {
            let mut at = offset + left;
            for (j, c) in source[at..offset + right].char_indices() {
                if c == '\t' {
                    let end = offset + left + j;
                    append(source, Span::new(at, end), Mode::Html, builder, out);
                    builder.opaque(" ", Span::new(end, end + 1));
                    at = end + 1;
                }
            }
            append(
                source,
                Span::new(at, offset + right),
                Mode::Html,
                builder,
                out,
            );
            if Some(i) != last_nonempty {
                builder.opaque(
                    " ",
                    Span::new(offset + right, (offset + line.len() + 1).min(span.end)),
                );
            }
        }
        offset += lines[i].len() + 1;
    }
}

fn html_space(source: &str, span: Span, b: &mut Builder, out: &mut Vec<crate::document::Block>) {
    let mut at = span.start;
    let mut plain = at;
    while at < span.end {
        let c = source[at..span.end].chars().next().unwrap();
        if !matches!(c, ' ' | '\t' | '\r' | '\n' | '\u{c}') {
            at += c.len_utf8();
            continue;
        }
        append(source, Span::new(plain, at), Mode::Html, b, out);
        let start = at;
        while at < span.end && matches!(source.as_bytes()[at], b' ' | b'\t' | b'\r' | b'\n' | 12) {
            at += 1;
        }
        if !b.text.ends_with(' ') {
            b.opaque(" ", Span::new(start, at));
        }
        plain = at;
    }
    append(source, Span::new(plain, span.end), Mode::Html, b, out);
}

/// Go の数値エスケープは Unicode 符号位置ではなく文字列のバイトを指定する。
fn byte_escape(s: &str) -> Option<(usize, u8)> {
    if s.starts_with("\\x") {
        return Some((4, hex(s.get(2..)?, 2)?.try_into().ok()?));
    }
    if s.starts_with('\\') {
        let digits = s.get(1..4)?;
        if digits.bytes().all(|b| matches!(b, b'0'..=b'7')) {
            return Some((4, u8::from_str_radix(digits, 8).ok()?));
        }
    }
    None
}

fn append_bytes(
    bytes: &[u8],
    spans: &[Span],
    b: &mut Builder,
    out: &mut Vec<crate::document::Block>,
) {
    let mut at = 0;
    while at < bytes.len() {
        let (valid, skip) = match std::str::from_utf8(&bytes[at..]) {
            Ok(s) => (s, 0),
            Err(e) => (
                std::str::from_utf8(&bytes[at..at + e.valid_up_to()]).unwrap(),
                e.error_len().unwrap_or(bytes.len() - at - e.valid_up_to()),
            ),
        };
        for (i, c) in valid.char_indices() {
            b.opaque(
                &c.to_string(),
                Span::new(spans[at + i].start, spans[at + i + c.len_utf8() - 1].end),
            );
        }
        at += valid.len();
        if skip > 0 {
            b.finish(out);
            at += skip;
        }
    }
}
