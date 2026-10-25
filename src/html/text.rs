//! 抽出した本文の組み立てと原文の位置の対応。

use crate::diagnostic::Span;
use crate::document::{Block, BlockKind, TextMap};

#[derive(Default)]
pub(crate) struct Builder {
    pub(crate) text: String,
    pub(crate) map: TextMap,
    pub(crate) span: Option<Span>,
    pub(crate) breaks: Vec<usize>,
    pub(crate) sentence_breaks: Vec<usize>,
    /// Markdown の HTML 本文では、英語だけの文章も残す。
    pub(crate) keep_non_japanese: bool,
}

impl Builder {
    pub(crate) fn exact(&mut self, source: &str, span: Span) {
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

    pub(crate) fn opaque(&mut self, value: &str, span: Span) {
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

    pub(crate) fn finish(&mut self, out: &mut Vec<Block>) {
        let b = std::mem::take(self);
        self.keep_non_japanese = b.keep_non_japanese;
        let trimmed = b.text.trim();
        if trimmed.is_empty()
            || !b.keep_non_japanese && !crate::text::contains_japanese(trimmed)
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

pub(crate) fn entity(s: &str) -> Option<(usize, String)> {
    // 採用する長さの上限までで打ち切り、未知の & が続いても残り全文を繰り返し走査しない。
    let end = s.bytes().take(64).position(|b| b == b';')? + 1;
    if !s[1..end - 1]
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'#')
    {
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

pub(crate) fn markup(source: &str, span: Span, b: &mut Builder) {
    let mut at = span.start;
    let mut plain = at;
    while at < span.end {
        let c = source[at..span.end].chars().next().unwrap();
        if !matches!(c, ' ' | '\t' | '\r' | '\n' | '\u{c}') {
            at += c.len_utf8();
            continue;
        }
        append(source, Span::new(plain, at), b);
        let start = at;
        while at < span.end && matches!(source.as_bytes()[at], b' ' | b'\t' | b'\r' | b'\n' | 12) {
            at += 1;
        }
        if !b.text.ends_with(' ') {
            b.opaque(" ", Span::new(start, at));
        }
        plain = at;
    }
    append(source, Span::new(plain, span.end), b);
}

/// 実体参照を元の一件の範囲に対応させて追加する。
fn append(source: &str, span: Span, b: &mut Builder) {
    let mut plain = span.start;
    let mut at = plain;
    while at < span.end {
        if source.as_bytes()[at] == b'&'
            && let Some((len, value)) = entity(&source[at..span.end])
        {
            b.exact(source, Span::new(plain, at));
            b.opaque(&value, Span::new(at, at + len));
            at += len;
            plain = at;
        } else {
            at += source[at..span.end].chars().next().unwrap().len_utf8();
        }
    }
    b.exact(source, Span::new(plain, span.end));
}
