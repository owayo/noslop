//! ソースで確定する文言だけを読む。式の評価やリテラル同士の連結は行わない。

mod decode;
mod markup;
mod strings;

use tree_sitter::Node;

use super::CodeLanguage;
use crate::diagnostic::Span;
use crate::document::Block;
use crate::html::text::Builder;

/// 文字列、本文、表示属性のそれぞれを独立した断片として取り出す。
pub(super) fn extract(
    source: &str,
    language: CodeLanguage,
    root: Node<'_>,
    comments: &[Span],
) -> Vec<Block> {
    let mut out = Vec::new();
    if language == CodeLanguage::Html {
        markup::extract(source, language, root, comments, &mut out);
    } else {
        walk(source, language, root, comments, &mut out);
    }
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

#[cfg(test)]
mod tests;
