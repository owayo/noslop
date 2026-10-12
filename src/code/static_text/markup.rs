//! HTML・JSX の本文と表示属性。項目の境界を保ち、インライン要素だけをつなぐ。

use super::{
    Builder, CodeLanguage,
    decode::{self, Mode},
};
use crate::{
    diagnostic::Span,
    document::{Block, BlockKind},
};
use tree_sitter::Node;

const DISPLAY_ATTRIBUTES: &[&str] = &[
    "title",
    "placeholder",
    "aria-label",
    "aria-description",
    "alt",
    "label",
];
const INLINE: &[&str] = &[
    "span", "strong", "em", "a", "b", "i", "u", "s", "small", "mark", "abbr", "time", "sup", "sub",
    "wbr",
];
const EXCLUDED: &[&str] = &["script", "style", "code", "pre", "template"];

pub(super) fn is_element(n: Node<'_>) -> bool {
    matches!(
        n.kind(),
        "element"
            | "self_closing_tag"
            | "script_element"
            | "style_element"
            | "jsx_element"
            | "jsx_self_closing_element"
    )
}

enum Task<'t> {
    Node(Node<'t>),
    Boundary,
    Whitespace(Span),
}

pub(super) fn extract(
    source: &str,
    lang: CodeLanguage,
    root: Node<'_>,
    comments: &[Span],
    out: &mut Vec<Block>,
) {
    extract_with_attributes(source, lang, root, comments, out, true);
}

fn extract_with_attributes(
    source: &str,
    lang: CodeLanguage,
    root: Node<'_>,
    comments: &[Span],
    out: &mut Vec<Block>,
    display_attributes: bool,
) {
    let mut tasks = vec![Task::Node(root)];
    let mut b = Builder {
        keep_non_japanese: !display_attributes,
        ..Builder::default()
    };
    while let Some(task) = tasks.pop() {
        let n = match task {
            Task::Node(n) => n,
            Task::Boundary => {
                b.finish(out);
                continue;
            }
            Task::Whitespace(span) => {
                decode::markup(source, span, false, &mut b, out);
                continue;
            }
        };
        if super::owned(n, comments) {
            continue;
        }
        match n.kind() {
            "document" if lang == CodeLanguage::Html => {
                tasks.extend(html_children(source, n, false).into_iter().rev());
            }
            "text" | "jsx_text" | "entity" => {
                decode::markup(
                    source,
                    Span::new(n.start_byte(), n.end_byte()),
                    n.kind() == "jsx_text",
                    &mut b,
                    out,
                );
            }
            "jsx_expression" => {
                b.finish(out);
                super::walk(source, lang, n, comments, out);
            }
            _ if is_element(n) => {
                let Some(open) = opening(n) else {
                    b.finish(out);
                    continue;
                };
                let tag = tag_name(open, source);
                let lower = tag.to_ascii_lowercase();
                let intrinsic = lang == CodeLanguage::Html || tag == lower;
                let line_break = intrinsic && lower == "br";
                let is_hidden = hidden(open, source);
                if intrinsic
                    && (EXCLUDED.contains(&lower.as_str())
                        || !display_attributes && lower == "textarea")
                    || is_hidden && !line_break
                {
                    b.finish(out);
                    continue;
                }
                if intrinsic && lower == "head" {
                    b.finish(out);
                    // head のメタデータは読まず、title 要素だけを独立して読む。
                    let mut c = n.walk();
                    let titles: Vec<_> = n
                        .named_children(&mut c)
                        .filter(|child| {
                            is_element(*child)
                                && opening(*child).is_some_and(|o| {
                                    tag_name(o, source).eq_ignore_ascii_case("title")
                                })
                        })
                        .collect();
                    tasks.extend(titles.into_iter().rev().map(Task::Node));
                    continue;
                }
                if display_attributes && !is_hidden {
                    attributes(source, lang, open, comments, out);
                }
                if line_break {
                    if is_hidden {
                        b.finish(out);
                    } else {
                        let at = b.text.len();
                        b.opaque("\n", Span::new(open.start_byte(), open.end_byte()));
                        b.sentence_breaks.push(at);
                    }
                    // HTML 文法は br の後の本文を子にするため、改行を足した後も読み進める。
                }
                let boundary = !line_break && (!intrinsic || !INLINE.contains(&lower.as_str()));
                if boundary {
                    b.finish(out);
                    tasks.push(Task::Boundary);
                }
                let mut c = n.walk();
                let children: Vec<_> = n
                    .named_children(&mut c)
                    .filter(|child| {
                        !matches!(
                            child.kind(),
                            "start_tag"
                                | "end_tag"
                                | "self_closing_tag"
                                | "jsx_opening_element"
                                | "jsx_closing_element"
                                | "jsx_attribute"
                                | "identifier"
                                | "member_expression"
                                | "nested_identifier"
                        )
                    })
                    .collect();
                if lang == CodeLanguage::Html {
                    tasks.extend(html_children(source, n, line_break).into_iter().rev());
                } else {
                    tasks.extend(children.into_iter().rev().map(Task::Node));
                }
            }
            "start_tag"
            | "end_tag"
            | "self_closing_tag"
            | "jsx_opening_element"
            | "jsx_closing_element"
            | "comment"
            | "tag_name"
            | "attribute_name" => {}
            _ => {
                b.finish(out);
                if display_attributes {
                    super::walk(source, lang, n, comments, out);
                } else {
                    let mut c = n.walk();
                    tasks.extend(
                        n.named_children(&mut c)
                            .collect::<Vec<_>>()
                            .into_iter()
                            .rev()
                            .map(Task::Node),
                    );
                }
            }
        }
    }
    b.finish(out);
}

/// HTML 文法がノードの外に置く空白を補い、本文を原文の順に並べる。
fn html_children<'t>(source: &str, n: Node<'t>, line_break: bool) -> Vec<Task<'t>> {
    let mut c = n.walk();
    let all: Vec<_> = n.named_children(&mut c).collect();
    let mut ordered = Vec::new();
    for (i, child) in all.iter().enumerate() {
        if i > 0 {
            let span = Span::new(all[i - 1].end_byte(), child.start_byte());
            if span.start < span.end
                && source[span.range()]
                    .bytes()
                    .all(|b| matches!(b, b' ' | b'\t' | b'\r' | b'\n' | 12))
            {
                ordered.push(Task::Whitespace(span));
            }
        }
        // 要素のタグは処理済み。文書直下の自己終了タグは独立した要素として読む。
        if !matches!(child.kind(), "start_tag" | "end_tag")
            && !(is_element(n) && child.kind() == "self_closing_tag")
        {
            ordered.push(Task::Node(*child));
        }
    }
    // br の暗黙の終端までの空白は、最後の子にも親の隙間にも含まれない。
    if line_break && let Some(last) = all.last() {
        let span = Span::new(last.end_byte(), n.end_byte());
        if span.start < span.end
            && source[span.range()]
                .bytes()
                .all(|b| matches!(b, b' ' | b'\t' | b'\r' | b'\n' | 12))
        {
            ordered.push(Task::Whitespace(span));
        }
    }
    ordered
}

/// 本文のブロックと、次の Markdown ブロックにも続く HTML 要素を返す。
pub(super) fn body(source: &str, root: Node<'_>) -> (Vec<Block>, String, bool) {
    let mut blocks = Vec::new();
    extract_with_attributes(source, CodeLanguage::Html, root, &[], &mut blocks, false);
    for block in &mut blocks {
        let mut node = root.descendant_for_byte_range(block.span.start, block.span.end);
        let mut kind = None;
        while let Some(n) = node {
            if is_element(n)
                && let Some(open) = opening(n)
            {
                let tag = tag_name(open, source).to_ascii_lowercase();
                let k = match tag.as_str() {
                    "h1" => Some(BlockKind::Heading(1)),
                    "h2" => Some(BlockKind::Heading(2)),
                    "h3" => Some(BlockKind::Heading(3)),
                    "h4" => Some(BlockKind::Heading(4)),
                    "h5" => Some(BlockKind::Heading(5)),
                    "h6" => Some(BlockKind::Heading(6)),
                    "li" => Some(BlockKind::ListItem),
                    "td" | "th" => Some(BlockKind::TableCell),
                    _ => None,
                };
                kind = kind.or(k);
                block.in_quote |= tag == "blockquote";
            }
            node = n.parent();
        }
        block.kind = kind.unwrap_or(BlockKind::Paragraph);
    }
    // 不完全なタグは ERROR の下にも現れる。タグのノードを順に読み、閉じるまで持ち越す。
    let mut nodes = vec![root];
    let mut stack: Vec<(&str, &str, bool)> = Vec::new();
    while let Some(n) = nodes.pop() {
        if n.kind() == "end_tag" {
            let name = tag_name(n, source);
            if let Some(at) = stack
                .iter()
                .rposition(|(open, _, _)| open.eq_ignore_ascii_case(name))
            {
                stack.truncate(at);
            }
            continue;
        }
        if matches!(n.kind(), "start_tag" | "self_closing_tag") {
            let name = tag_name(n, source);
            let tag = name.to_ascii_lowercase();
            let raw = &source[n.byte_range()];
            let does_not_carry = [
                "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta",
                "param", "source", "track", "wbr", "p", "li", "td", "th", "dt", "dd", "option",
                "optgroup", "thead", "tbody", "tfoot", "tr", "html", "head", "body",
            ]
            .contains(&tag.as_str());
            if !does_not_carry && !raw.ends_with("/>") {
                stack.push((
                    name,
                    raw,
                    EXCLUDED.contains(&tag.as_str()) || tag == "textarea" || hidden(n, source),
                ));
            }
            continue;
        }
        let mut c = n.walk();
        nodes.extend(
            n.named_children(&mut c)
                .collect::<Vec<_>>()
                .into_iter()
                .rev(),
        );
    }
    let excluded = stack.iter().any(|(_, _, excluded)| *excluded);
    let pending = stack.into_iter().map(|(_, raw, _)| raw).collect();
    (blocks, pending, excluded)
}

fn opening(n: Node<'_>) -> Option<Node<'_>> {
    if matches!(n.kind(), "jsx_self_closing_element" | "self_closing_tag") {
        return Some(n);
    }
    let mut c = n.walk();
    n.named_children(&mut c).find(|n| {
        matches!(
            n.kind(),
            "start_tag" | "self_closing_tag" | "jsx_opening_element"
        )
    })
}

fn tag_name<'s>(n: Node<'_>, source: &'s str) -> &'s str {
    let mut c = n.walk();
    n.named_children(&mut c)
        .find(|n| {
            matches!(
                n.kind(),
                "tag_name" | "identifier" | "member_expression" | "nested_identifier"
            )
        })
        .map_or("", |n| &source[n.byte_range()])
}

fn attribute_name<'s>(n: Node<'_>, source: &'s str) -> &'s str {
    n.named_child(0).map_or("", |n| &source[n.byte_range()])
}

fn hidden(open: Node<'_>, source: &str) -> bool {
    let mut c = open.walk();
    open.named_children(&mut c)
        .filter(|n| matches!(n.kind(), "attribute" | "jsx_attribute"))
        .any(|a| {
            if !attribute_name(a, source).eq_ignore_ascii_case("hidden") {
                return false;
            }
            if a.kind() == "attribute" {
                return true;
            } // HTML の真偽属性は値にかかわらず有効。
            let value = a.named_child(1);
            value.is_none_or(|v| {
                if v.kind() == "string" {
                    return true;
                }
                if v.kind() != "jsx_expression" {
                    return false;
                }
                let mut c = v.walk();
                let mut children = v.named_children(&mut c).filter(|n| n.kind() != "comment");
                children.next().is_some_and(|n| n.kind() == "true") && children.next().is_none()
            })
        })
}

fn attributes(
    source: &str,
    lang: CodeLanguage,
    open: Node<'_>,
    comments: &[Span],
    out: &mut Vec<Block>,
) {
    let mut c = open.walk();
    for a in open
        .named_children(&mut c)
        .filter(|n| matches!(n.kind(), "attribute" | "jsx_attribute"))
    {
        if !DISPLAY_ATTRIBUTES.contains(&attribute_name(a, source).to_ascii_lowercase().as_str()) {
            continue;
        }
        let Some(value) = a.named_child(1) else {
            continue;
        };
        if value.kind() == "jsx_expression" {
            super::walk(source, lang, value, comments, out);
        } else {
            let span = match value.kind() {
                "string" | "quoted_attribute_value" => {
                    let s = &source[value.byte_range()];
                    let close =
                        usize::from(s.len() >= 2 && s.as_bytes().first() == s.as_bytes().last());
                    Span::new(
                        value.start_byte() + 1,
                        value.end_byte().saturating_sub(close),
                    )
                }
                "attribute_value" => Span::new(value.start_byte(), value.end_byte()),
                _ => continue,
            };
            if span.start > span.end {
                continue;
            }
            let mut b = Builder::default();
            decode::append(source, span, Mode::Html, &mut b, out);
            b.finish(out);
        }
    }
}
