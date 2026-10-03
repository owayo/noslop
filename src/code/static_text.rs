//! ソースで確定する文言だけを読む。式の評価やリテラル同士の連結は行わない。

mod decode;
mod markup;
mod strings;

use tree_sitter::Node;

use super::CodeLanguage;
use crate::diagnostic::Span;
use crate::document::{Block, BlockKind, TextMap};

/// 文字列、本文、表示属性のそれぞれを独立した断片として取り出す。
pub(super) fn extract(
    source: &str,
    language: CodeLanguage,
    root: Node<'_>,
    comments: &[Span],
) -> Vec<Block> {
    let mut out = Vec::new();
    walk(source, language, root, comments, &mut out);
    out
}

fn walk(
    source: &str,
    language: CodeLanguage,
    root: Node<'_>,
    comments: &[Span],
    out: &mut Vec<Block>,
) {
    let mut nodes = vec![root];
    while let Some(node) = nodes.pop() {
        if owned(node, comments) {
            continue;
        }
        if markup::is_element(node) {
            markup::extract(source, language, node, comments, out);
            continue;
        }
        if matches!(
            node.kind(),
            "regex"
                | "regex_literal"
                | "regex_pattern"
                | "regular_expression"
                | "comment"
                | "line_comment"
                | "block_comment"
        ) {
            continue;
        }
        if let Some(holes) = strings::extract(source, language, node, out) {
            nodes.extend(holes.into_iter().rev());
            continue;
        }
        let mut cursor = node.walk();
        nodes.extend(
            node.named_children(&mut cursor)
                .collect::<Vec<_>>()
                .into_iter()
                .rev(),
        );
    }
}

fn owned(node: Node<'_>, comments: &[Span]) -> bool {
    let at = comments.partition_point(|s| s.start <= node.start_byte());
    at > 0 && node.end_byte() <= comments[at - 1].end
}

#[derive(Default)]
struct Builder {
    text: String,
    map: TextMap,
    span: Option<Span>,
    breaks: Vec<usize>,
    sentence_breaks: Vec<usize>,
}

impl Builder {
    fn exact(&mut self, source: &str, span: Span) {
        if span.start >= span.end {
            return;
        }
        let at = self.text.len();
        self.text.push_str(&source[span.range()]);
        self.map.push_exact(at, span.start, span.end - span.start);
        self.extend_span(span);
        for (i, c) in source[span.range()].char_indices() {
            if c == '\n' {
                self.breaks.push(at + i);
            }
        }
    }

    fn opaque(&mut self, value: &str, span: Span) {
        let at = self.text.len();
        self.text.push_str(value);
        self.map.push_opaque(at, value.len(), span);
        self.extend_span(span);
        for (i, c) in value.char_indices() {
            if c == '\n' {
                self.breaks.push(at + i);
            }
        }
    }

    fn extend_span(&mut self, span: Span) {
        self.span = Some(Span::new(
            self.span.map_or(span.start, |s| s.start),
            span.end,
        ));
    }

    fn finish(&mut self, out: &mut Vec<Block>) {
        let b = std::mem::take(self);
        let trimmed = b.text.trim();
        if !crate::text::contains_japanese(trimmed)
            || ["http://", "https://", "ftp://"]
                .iter()
                .any(|prefix| trimmed.starts_with(prefix))
                && !trimmed.chars().any(char::is_whitespace)
        {
            return;
        }
        out.push(Block {
            kind: BlockKind::Paragraph,
            text: b.text,
            map: b.map,
            span: b.span.unwrap_or(Span::new(0, 0)),
            in_quote: false,
            in_footnote: false,
            line_breaks: b.breaks,
            sentence_breaks: b.sentence_breaks,
            marks: Vec::new(),
            sentences: 0..0,
        });
    }
}

#[cfg(test)]
mod tests;
