//! 記号の一致を、原文の同じ行の範囲と文脈に戻す共通処理。

use std::ops::Range;

use crate::diagnostic::Span;
use crate::document::{Block, Document};

/// 解析用テキストの各行と、その行の先頭のバイト位置。
/// CRLF の `\r` と末尾の空行も保ち、呼び出し側へ `\n` を含めずに渡す。
pub(super) fn line_offsets(text: &str) -> impl Iterator<Item = (usize, &str)> {
    text.split('\n').scan(0, |offset, line| {
        let start = *offset;
        *offset += line.len() + 1;
        Some((start, line))
    })
}

/// Markdown の折り返しが空白になっていても、原文の複数行をまたぐ一致は外す。
pub(super) fn single_line_span(doc: &Document, block: &Block, range: Range<usize>) -> Option<Span> {
    let span = block.to_source(range);
    (!span.is_empty() && doc.lines.line(span.start) == doc.lines.line(span.end - 1)).then_some(span)
}

/// 一致の先頭を含む文の範囲。文がなければ、そのブロック全体を文脈にする。
pub(super) fn sentence_context(doc: &Document, block: usize, at: usize) -> Span {
    doc.block_sentences(block)
        .iter()
        .find(|sentence| sentence.range.start <= at && at < sentence.range.end)
        .map_or(doc.blocks[block].span, |sentence| sentence.span)
}
