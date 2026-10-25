//! Markdown の HTML 本文。コードの文法を使わず、原文の位置と要素の文脈を保つ。

pub(crate) mod text;

use html5gum::{DefaultEmitter, Token, Tokenizer};

use crate::diagnostic::Span;
use crate::document::{Block, BlockKind, TextMap};
use text::Builder;

const INLINE: &[&str] = &[
    "span", "strong", "em", "a", "b", "i", "u", "s", "small", "mark", "abbr", "time", "sup", "sub",
    "wbr",
];
const VOID: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source",
    "track", "wbr",
];
const OPTIONAL: &[&str] = &[
    "p", "li", "td", "th", "dt", "dd", "option", "optgroup", "thead", "tbody", "tfoot", "tr",
    "html", "head", "body",
];
const EXCLUDED: &[&str] = &["script", "style", "code", "pre", "template", "textarea"];

/// 空行で分かれた HTML の要素を、Markdown の解析中だけ引き継ぐ。
#[derive(Default)]
pub(crate) struct HtmlBody {
    pending: String,
    excluded: bool,
}

impl HtmlBody {
    pub(crate) fn skipping(&self) -> bool {
        self.excluded
    }

    pub(crate) fn read(&mut self, source: &str) -> Vec<Block> {
        let prefix = self.pending.len();
        let joined = format!("{}{source}", self.pending);
        let (mut blocks, stack) = body(&joined);
        self.excluded = stack
            .iter()
            .any(|element| !OPTIONAL.contains(&element.name.as_str()) && element.excluded_here);
        self.pending = stack
            .iter()
            .filter(|element| !OPTIONAL.contains(&element.name.as_str()))
            .map(|element| &joined[element.span.range()])
            .collect();
        let mut map = TextMap::default();
        map.push_exact(prefix, 0, source.len());
        for block in &mut blocks {
            block.map = block.map.compose(&map);
            block.span = Span::new(
                block.span.start.saturating_sub(prefix),
                block.span.end.saturating_sub(prefix),
            );
        }
        blocks
    }
}

struct Element {
    name: String,
    span: Span,
    excluded: bool,
    excluded_here: bool,
    in_head: bool,
    in_title: bool,
    kind: BlockKind,
    in_quote: bool,
}

fn visible(stack: &[Element]) -> bool {
    stack
        .last()
        .is_none_or(|e| !e.excluded && (!e.in_head || e.in_title))
}

fn finish(builder: &mut Builder, out: &mut Vec<Block>, stack: &[Element]) {
    let start = out.len();
    builder.finish(out);
    if let Some(element) = stack.last() {
        for block in &mut out[start..] {
            block.kind = element.kind;
            block.in_quote = element.in_quote;
        }
    }
}

fn kind(name: &str) -> Option<BlockKind> {
    Some(match name {
        "h1" => BlockKind::Heading(1),
        "h2" => BlockKind::Heading(2),
        "h3" => BlockKind::Heading(3),
        "h4" => BlockKind::Heading(4),
        "h5" => BlockKind::Heading(5),
        "h6" => BlockKind::Heading(6),
        "li" => BlockKind::ListItem,
        "td" | "th" => BlockKind::TableCell,
        _ => return None,
    })
}

/// 終了タグを省略できる兄弟を、次の開始タグで閉じる。
fn implicitly_closes(open: &str, next: &str) -> bool {
    match open {
        "li" => next == "li",
        "dt" | "dd" => matches!(next, "dt" | "dd"),
        "td" | "th" => matches!(next, "td" | "th" | "tr"),
        "tr" => next == "tr",
        "option" => matches!(next, "option" | "optgroup"),
        "optgroup" => next == "optgroup",
        "thead" | "tbody" | "tfoot" => matches!(next, "thead" | "tbody" | "tfoot"),
        "p" => matches!(
            next,
            "address"
                | "article"
                | "aside"
                | "blockquote"
                | "div"
                | "details"
                | "figure"
                | "figcaption"
                | "dl"
                | "fieldset"
                | "footer"
                | "form"
                | "h1"
                | "h2"
                | "h3"
                | "h4"
                | "h5"
                | "h6"
                | "header"
                | "hr"
                | "main"
                | "nav"
                | "ol"
                | "p"
                | "pre"
                | "section"
                | "table"
                | "ul"
        ),
        _ => false,
    }
}

fn body(source: &str) -> (Vec<Block>, Vec<Element>) {
    let emitter = DefaultEmitter::<usize>::new_with_span();
    // 文書として検査する HTML の生テキスト要素だけ、開始タグを読んだ後に状態を替える。
    let mut stack: Vec<Element> = Vec::new();
    let mut out = Vec::new();
    let mut builder = Builder {
        keep_non_japanese: true,
        ..Builder::default()
    };
    let mut tokenizer = Tokenizer::new_with_emitter(source, emitter);
    while let Some(token) = tokenizer.next() {
        match token.unwrap() {
            Token::String(value) if visible(&stack) => {
                text::markup(
                    source,
                    Span::new(value.span.start, value.span.end),
                    &mut builder,
                );
            }
            Token::StartTag(tag) => {
                let name = String::from_utf8_lossy(&tag.name).into_owned();
                // 自己終了のタグでは、後続の本文まで生テキストとして飲み込まない。
                // noscript などの代替コンテンツは通常のタグとして読む。
                if !tag.self_closing
                    && matches!(name.as_str(), "script" | "style" | "textarea" | "title")
                    && let Some(state) = html5gum::naive_next_state(&tag.name)
                {
                    tokenizer.set_state(state);
                }
                if let Some(at) = stack
                    .iter()
                    .rposition(|e| implicitly_closes(&e.name, &name))
                {
                    // リスト・表の子孫を新しい兄弟と取り違えない。
                    let nested = stack[at + 1..].iter().any(|e| {
                        e.excluded_here || matches!(e.name.as_str(), "ul" | "ol" | "dl" | "table")
                    });
                    if !nested {
                        finish(&mut builder, &mut out, &stack);
                        stack.truncate(at);
                    }
                }
                let hidden = tag.attributes.contains_key(b"hidden".as_slice());
                let excluded = hidden || EXCLUDED.contains(&name.as_str());
                let span = Span::new(tag.span.start, tag.span.end);
                if name == "br" && visible(&stack) && !hidden {
                    let at = builder.text.len();
                    builder.opaque("\n", span);
                    builder.sentence_breaks.push(at);
                } else if !INLINE.contains(&name.as_str()) || excluded {
                    finish(&mut builder, &mut out, &stack);
                }
                if !tag.self_closing && !VOID.contains(&name.as_str()) {
                    let parent = stack.last();
                    stack.push(Element {
                        excluded: excluded || parent.is_some_and(|e| e.excluded),
                        excluded_here: excluded,
                        in_head: name == "head" || parent.is_some_and(|e| e.in_head),
                        in_title: name == "title" || parent.is_some_and(|e| e.in_title),
                        kind: kind(&name)
                            .unwrap_or_else(|| parent.map_or(BlockKind::Paragraph, |e| e.kind)),
                        in_quote: name == "blockquote" || parent.is_some_and(|e| e.in_quote),
                        name,
                        span,
                    });
                }
            }
            Token::EndTag(tag) => {
                let name = String::from_utf8_lossy(&tag.name);
                if let Some(at) = stack.iter().rposition(|e| e.name == name) {
                    if !INLINE.contains(&name.as_ref()) || stack[at].excluded {
                        finish(&mut builder, &mut out, &stack);
                    }
                    stack.truncate(at);
                }
            }
            _ => {}
        }
    }
    finish(&mut builder, &mut out, &stack);
    (out, stack)
}

#[cfg(test)]
mod tests;
