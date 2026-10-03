//! 言語ごとのリテラルと、補間で区切られた静的区間を読む。

use super::{
    Builder, CodeLanguage,
    decode::{self, Mode},
};
use crate::{diagnostic::Span, document::Block};
use tree_sitter::Node;

pub(super) fn extract<'t>(
    source: &str,
    lang: CodeLanguage,
    node: Node<'t>,
    out: &mut Vec<Block>,
) -> Option<Vec<Node<'t>>> {
    use CodeLanguage::*;
    let kind = node.kind();
    let literal = match lang {
        Rust => matches!(kind, "string_literal" | "raw_string_literal"),
        JavaScript | TypeScript | Tsx => matches!(kind, "string" | "template_string"),
        Python => kind == "string",
        Go => matches!(kind, "interpreted_string_literal" | "raw_string_literal"),
        Java | C | Cpp => matches!(kind, "string_literal" | "raw_string_literal"),
        CSharp => matches!(
            kind,
            "string_literal"
                | "verbatim_string_literal"
                | "raw_string_literal"
                | "interpolated_string_expression"
        ),
        Ruby => matches!(kind, "string" | "heredoc_body"),
        Php => matches!(kind, "string" | "encapsed_string" | "heredoc" | "nowdoc"),
        Swift => matches!(
            kind,
            "line_string_literal" | "multi_line_string_literal" | "raw_string_literal"
        ),
        Kotlin => matches!(kind, "string_literal" | "multiline_string_literal"),
        Bash => matches!(
            kind,
            "string" | "raw_string" | "ansi_c_string" | "heredoc_body"
        ),
        Yaml => matches!(
            kind,
            "string_scalar" | "single_quote_scalar" | "double_quote_scalar" | "block_scalar"
        ),
        Toml | Lua => kind == "string",
        Css => kind == "string_value",
        Html => false,
    };
    if !literal {
        return None;
    }
    if matches!(lang, JavaScript | TypeScript | Tsx)
        && node.parent().is_some_and(|p| {
            p.child_by_field_name("key")
                .or_else(|| p.child_by_field_name("name"))
                .is_some_and(|k| k.id() == node.id())
        })
    {
        return Some(Vec::new());
    }
    let raw = &source[node.byte_range()];
    if lang == Rust && raw.starts_with(['b', 'c'])
        || lang == Python
            && raw
                .split(['\'', '"'])
                .next()
                .unwrap_or("")
                .contains(['b', 'B'])
        || lang == CSharp && raw.ends_with("u8")
    {
        return Some(Vec::new());
    }
    if lang == Yaml && is_key(node) || lang == Css && !css_content(node, source) {
        return Some(Vec::new());
    }
    let Some((body, mode)) = body(source, lang, node) else {
        return Some(Vec::new());
    };
    let mut holes = Vec::new();
    let mut pending = vec![node];
    while let Some(n) = pending.pop() {
        if n != node && is_hole(lang, n.kind()) {
            holes.push(n);
            continue;
        }
        // 文字列の内容はエスケープの子を持つが、補間のない葉はたどらない。
        let mut c = n.walk();
        pending.extend(
            n.named_children(&mut c)
                .collect::<Vec<_>>()
                .into_iter()
                .rev(),
        );
    }
    holes.sort_by_key(Node::start_byte);
    let mut b = Builder::default();
    let mut at = body.start;
    for hole in &holes {
        let mut start = hole.start_byte().max(body.start);
        let mut end = hole.end_byte().min(body.end);
        // Swift の補間の開きと閉じは式ノードの外にある。
        if lang == Swift && hole.kind() == "interpolated_expression" {
            if let Some(prev) = hole
                .prev_sibling()
                .filter(|n| source[n.byte_range()].starts_with('\\'))
            {
                start = prev.start_byte();
            }
            if let Some(next) = hole
                .next_sibling()
                .filter(|n| source[n.byte_range()] == *")")
            {
                end = next.end_byte();
            }
        }
        if lang == Php {
            if let Some(prev) = hole
                .prev_sibling()
                .filter(|n| source[n.byte_range()] == *"{")
            {
                start = prev.start_byte();
            }
            if let Some(next) = hole
                .next_sibling()
                .filter(|n| source[n.byte_range()] == *"}")
            {
                end = next.end_byte();
            }
        }
        if at < start {
            append(source, lang, node, Span::new(at, start), mode, &mut b, out);
        }
        b.finish(out);
        at = end.max(at);
    }
    if at < body.end {
        append(
            source,
            lang,
            node,
            Span::new(at, body.end),
            mode,
            &mut b,
            out,
        );
    }
    b.finish(out);
    Some(holes)
}

fn is_hole(lang: CodeLanguage, kind: &str) -> bool {
    matches!(
        kind,
        "interpolation"
            | "template_substitution"
            | "interpolated_expression"
            | "raw_str_interpolation"
    ) || lang == CodeLanguage::Php
        && matches!(
            kind,
            "variable_name" | "subscript_expression" | "member_access_expression"
        )
        || lang == CodeLanguage::Bash
            && matches!(
                kind,
                "expansion" | "simple_expansion" | "command_substitution" | "arithmetic_expansion"
            )
}

fn is_key(mut node: Node<'_>) -> bool {
    while let Some(parent) = node.parent() {
        if matches!(parent.kind(), "block_mapping_pair" | "flow_pair") {
            return parent.child_by_field_name("key").is_some_and(|k| {
                k.start_byte() <= node.start_byte() && node.end_byte() <= k.end_byte()
            });
        }
        node = parent;
    }
    false
}

fn css_content(node: Node<'_>, source: &str) -> bool {
    let mut parent = node.parent();
    while let Some(p) = parent {
        if p.kind() == "declaration" {
            let mut c = p.walk();
            return p
                .named_children(&mut c)
                .find(|n| n.kind() == "property_name")
                .is_some_and(|n| source[n.byte_range()].eq_ignore_ascii_case("content"));
        }
        parent = p.parent();
    }
    false
}

fn body(source: &str, lang: CodeLanguage, node: Node<'_>) -> Option<(Span, Mode)> {
    use CodeLanguage::*;
    let s = &source[node.byte_range()];
    let start = node.start_byte();
    let kind = node.kind();
    if lang == Yaml && kind == "string_scalar" {
        return Some((Span::new(start, node.end_byte()), Mode::Raw));
    }
    if lang == Yaml && kind == "block_scalar" {
        let open = s.find('\n')? + 1;
        return Some((Span::new(start + open, node.end_byte()), Mode::Raw));
    }
    if matches!(lang, Bash | Ruby) && kind == "heredoc_body" {
        let mut c = node.walk();
        let end = node
            .named_children(&mut c)
            .find(|n| n.kind() == "heredoc_end")
            .map_or(node.end_byte(), |n| n.start_byte());
        let raw = if lang == Bash {
            node.parent().is_some_and(|p| {
                let mut c = p.walk();
                p.named_children(&mut c)
                    .find(|n| n.kind() == "heredoc_start")
                    .is_some_and(|n| source[n.byte_range()].contains(['\'', '"', '\\']))
            })
        } else {
            ruby_heredoc_raw(node, source)
        };
        return Some((
            Span::new(start, end),
            if raw {
                Mode::Raw
            } else if lang == Bash {
                Mode::BashDouble
            } else {
                Mode::Escaped(lang)
            },
        ));
    }
    if matches!(lang, Php) && matches!(kind, "heredoc_body" | "heredoc" | "nowdoc") {
        let mut c = node.walk();
        let content = node
            .named_children(&mut c)
            .find(|n| matches!(n.kind(), "heredoc_body" | "heredoc_content" | "nowdoc_body"));
        if let Some(n) = content {
            return Some((
                Span::new(n.start_byte(), n.end_byte()),
                if kind == "nowdoc" {
                    Mode::Raw
                } else {
                    Mode::Escaped(lang)
                },
            ));
        }
        if kind == "heredoc_body" {
            return Some((Span::new(start, node.end_byte()), Mode::Escaped(lang)));
        }
        return None;
    }
    if lang == Cpp && kind == "raw_string_literal" {
        let mut c = node.walk();
        let n = node
            .named_children(&mut c)
            .find(|n| n.kind() == "raw_string_content")?;
        return Some((Span::new(n.start_byte(), n.end_byte()), Mode::Raw));
    }
    if lang == Lua && s.starts_with('[') {
        let level = s.get(1..)?.bytes().take_while(|b| *b == b'=').count();
        if s.as_bytes().get(level + 1) != Some(&b'[') {
            return None;
        }
        let open = level + 2;
        let close = open;
        let mut begin = start + open;
        if source[begin..].starts_with("\r\n") {
            begin += 2;
        } else if source[begin..].starts_with('\n') {
            begin += 1;
        }
        return Some((
            Span::new(begin, node.end_byte().checked_sub(close)?),
            Mode::Raw,
        ));
    }
    if lang == Ruby && s.starts_with('%') {
        let open = node.child(0)?;
        let close = node.child(node.child_count().checked_sub(1)?)?;
        let opening = source[open.byte_range()].chars().next_back()?;
        let closing = source[close.byte_range()].chars().next()?;
        return Some((
            Span::new(open.end_byte(), close.start_byte()),
            if s.starts_with("%q") {
                Mode::Single(opening, closing)
            } else {
                Mode::Escaped(lang)
            },
        ));
    }
    let quote = s.find(['\'', '"', '`'])?;
    let c = s.as_bytes()[quote];
    let run = s[quote..].bytes().take_while(|b| *b == c).count();
    let quote_len = if run >= 3
        && c != b'`'
        && s.len() >= quote + 6
        && matches!(lang, Python | Java | CSharp | Swift | Kotlin | Toml)
    {
        if lang == CSharp {
            let mut c = node.walk();
            node.children(&mut c)
                .find(|n| matches!(n.kind(), "raw_string_start" | "interpolation_quote"))
                .map_or(run, |n| n.end_byte() - n.start_byte())
        } else {
            3
        }
    } else {
        1
    };
    let hash = if lang == Rust || lang == Swift {
        s[..quote].bytes().filter(|b| *b == b'#').count()
    } else {
        0
    };
    let close = quote_len + hash;
    let end = node.end_byte().checked_sub(close)?;
    let begin = start + quote + quote_len;
    if begin > end || !source.is_char_boundary(end) {
        return None;
    }
    let mode = if lang == Css {
        Mode::Css
    } else if lang == CSharp && quote_len >= 3 {
        Mode::Raw
    } else if lang == CSharp && s[..quote].contains('@') {
        Mode::Verbatim
    } else if lang == Swift && hash > 0 {
        Mode::SwiftRaw(hash)
    } else if lang == Bash && kind == "string" {
        Mode::BashDouble
    } else if lang == Yaml && c == b'\'' {
        Mode::YamlSingle
    } else if matches!(lang, Rust | Go | CSharp) && kind == "raw_string_literal"
        || lang == Python && s[..quote].contains(['r', 'R'])
        || lang == Bash && kind == "raw_string"
        || lang == Kotlin && kind == "multiline_string_literal"
        || lang == Toml && c == b'\''
    {
        Mode::Raw
    } else if matches!(lang, Ruby | Php) && c == b'\'' {
        Mode::Single('\'', '\'')
    } else {
        Mode::Escaped(lang)
    };
    Some((Span::new(begin, end), mode))
}

/// 同じ親にある Ruby の開始記号と本体を順に対応付け、入れ子のスコープは混ぜない。
fn ruby_heredoc_raw(node: Node<'_>, source: &str) -> bool {
    let Some(parent) = node.parent() else {
        return true;
    };
    let mut modes = std::collections::VecDeque::new();
    let mut c = parent.walk();
    for sibling in parent.named_children(&mut c) {
        if sibling.kind() == "heredoc_body" {
            let raw = modes.pop_front().unwrap_or(true);
            if sibling.id() == node.id() {
                return raw;
            }
            continue;
        }
        let mut nodes = vec![sibling];
        while let Some(n) = nodes.pop() {
            if n.kind() == "heredoc_beginning" {
                let beginning = source[n.byte_range()].strip_prefix("<<").unwrap_or("");
                let delimiter = beginning.strip_prefix(['-', '~']).unwrap_or(beginning);
                modes.push_back(delimiter.starts_with('\''));
                continue;
            }
            let mut c = n.walk();
            let children = n.named_children(&mut c).collect::<Vec<_>>();
            // 本体を持つ入れ子は、その親を起点に別途対応付ける。
            if children.iter().any(|child| child.kind() == "heredoc_body") {
                continue;
            }
            nodes.extend(children.into_iter().rev());
        }
    }
    true
}

/// C/C++ の数値エスケープは文字の形式によって意味が変わる。狭い文字列の非 ASCII バイトは推測しない。
fn append(
    source: &str,
    lang: CodeLanguage,
    node: Node<'_>,
    span: Span,
    mode: Mode,
    b: &mut Builder,
    out: &mut Vec<Block>,
) {
    let text = &source[node.byte_range()];
    let wide = text.starts_with(['L', 'u', 'U']) && !text.starts_with("u8");
    if !matches!(lang, CodeLanguage::C | CodeLanguage::Cpp) || wide || mode == Mode::Raw {
        decode::append(source, span, mode, b, out);
        return;
    }
    let mut at = span.start;
    let mut plain = at;
    while at < span.end {
        if source[at..span.end].starts_with("\\\\") {
            at += 2;
            continue;
        }
        let s = &source[at..span.end];
        let numeric = if let Some(hex) = s.strip_prefix("\\x") {
            let digits = hex.bytes().take_while(u8::is_ascii_hexdigit).count();
            Some((
                at + 2 + digits,
                u32::from_str_radix(&hex[..digits], 16).ok(),
            ))
        } else if s.starts_with('\\')
            && s.as_bytes()
                .get(1)
                .is_some_and(|b| matches!(b, b'0'..=b'7'))
        {
            let digits = s[1..]
                .bytes()
                .take(3)
                .take_while(|b| matches!(b, b'0'..=b'7'))
                .count();
            Some((
                at + 1 + digits,
                u32::from_str_radix(&s[1..1 + digits], 8).ok(),
            ))
        } else {
            None
        };
        if let Some((end, value)) = numeric
            && value.is_none_or(|v| v > 0x7f)
        {
            decode::append(source, Span::new(plain, at), mode, b, out);
            b.finish(out);
            at = end;
            plain = at;
            continue;
        }
        at += source[at..].chars().next().unwrap().len_utf8();
    }
    decode::append(source, Span::new(plain, span.end), mode, b, out);
}
