//! Markdown をブロックの列に変換する。
//!
//! 解析用テキストには地の文として読まれる文字だけを残す。
//!
//! - コードブロック・HTML ブロック・front matter・数式ブロックは捨てる
//! - インラインコード・数式・画像・自動リンクはプレースホルダ 1 字に畳む
//! - 太字・リンクなどの装飾記号は取り除き、装飾の範囲は [`InlineMark`] に残す
//! - ソフト改行は日本語どうしの間なら何も入れず、英数字が隣り合うなら空白 1 つにする
//!
//! 抑制コメントは HTML ブロックとインライン HTML からだけ読む (コードブロック内の
//! 記法例を誤って抑制として扱わないため)。

use std::ops::Range;

use pulldown_cmark::{Event, HeadingLevel, LinkType, Options, Parser, Tag, TagEnd};

use crate::diagnostic::Span;
use crate::directive;
use crate::document::{Block, BlockKind, Directive, InlineMark, MarkKind, TextMap};
use crate::text::{self, PLACEHOLDER};

fn options() -> Options {
    Options::ENABLE_TABLES
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_HEADING_ATTRIBUTES
        | Options::ENABLE_YAML_STYLE_METADATA_BLOCKS
        | Options::ENABLE_PLUSES_DELIMITED_METADATA_BLOCKS
        | Options::ENABLE_MATH
        | Options::ENABLE_GFM
}

/// Markdown の原文をブロックと抑制コメントに分ける。
pub fn parse(source: &str) -> (Vec<Block>, Vec<Directive>) {
    let mut builder = Builder::new(source);
    for (event, range) in Parser::new_ext(source, options()).into_offset_iter() {
        builder.event(event, range);
    }
    builder.finish()
}

/// 組み立て途中のブロック。
struct Current {
    kind: BlockKind,
    text: String,
    map: TextMap,
    span: Span,
    in_quote: bool,
    in_footnote: bool,
    line_breaks: Vec<usize>,
    marks: Vec<InlineMark>,
    /// 直前にあった改行の原文範囲 (次の文字を足すときに区切りの空白を入れるか決める)。
    pending_break: Option<Span>,
}

impl Current {
    fn extend_span(&mut self, range: &Range<usize>) {
        self.span.start = self.span.start.min(range.start);
        self.span.end = self.span.end.max(range.end);
    }

    /// 改行の直後に文字を足すとき、英数字どうしが連結しないよう空白を補う。
    fn apply_pending_break(&mut self, next: char) {
        if let Some(src) = self.pending_break.take()
            && let Some(prev) = self.text.chars().next_back()
            && !(text::is_cjk_like(prev) && text::is_cjk_like(next))
            && !prev.is_whitespace()
            && !next.is_whitespace()
        {
            let at = self.text.len();
            self.text.push(' ');
            self.map.push_opaque(at, 1, src);
        }
    }

    fn push_text(&mut self, source: &str, t: &str, range: Range<usize>) {
        let Some(first) = t.chars().next() else {
            return;
        };
        self.apply_pending_break(first);
        let at = self.text.len();
        self.text.push_str(t);
        if source.get(range.clone()) == Some(t) {
            self.map.push_exact(at, range.start, t.len());
        } else {
            self.map
                .push_opaque(at, t.len(), Span::new(range.start, range.end));
        }
        self.extend_span(&range);
    }

    fn push_placeholder(&mut self, range: Range<usize>, kind: MarkKind) {
        self.apply_pending_break(PLACEHOLDER);
        let at = self.text.len();
        self.text.push(PLACEHOLDER);
        let span = Span::new(range.start, range.end);
        self.map.push_opaque(at, PLACEHOLDER.len_utf8(), span);
        self.marks.push(InlineMark {
            kind,
            range: at..self.text.len(),
            span,
        });
        self.extend_span(&range);
    }

    fn line_break(&mut self, range: Range<usize>) {
        self.line_breaks.push(self.text.len());
        self.pending_break = Some(Span::new(range.start, range.end));
    }
}

struct Builder<'a> {
    source: &'a str,
    blocks: Vec<Block>,
    html_spans: Vec<Span>,
    current: Option<Current>,
    quote_depth: usize,
    footnote_depth: usize,
    item_depth: usize,
    /// コードブロック・front matter の内側 (テキストを捨てる)。
    skip_depth: usize,
    /// 画像・自動リンクの内側 (プレースホルダに畳んだので中身を捨てる)。
    inline_skip: usize,
    /// 開いている装飾 (種類, 解析用テキスト上の開始位置, 原文範囲)。
    mark_stack: Vec<(MarkKind, usize, Span)>,
    /// 開いているリンクが自動リンク (中身を捨てる) か。
    link_stack: Vec<bool>,
}

impl<'a> Builder<'a> {
    fn new(source: &'a str) -> Self {
        Self {
            source,
            blocks: Vec::new(),
            html_spans: Vec::new(),
            current: None,
            quote_depth: 0,
            footnote_depth: 0,
            item_depth: 0,
            skip_depth: 0,
            inline_skip: 0,
            mark_stack: Vec::new(),
            link_stack: Vec::new(),
        }
    }

    fn start_block(&mut self, kind: BlockKind, range: &Range<usize>) {
        self.flush();
        self.current = Some(Current {
            kind,
            text: String::new(),
            map: TextMap::default(),
            span: Span::new(range.start, range.end),
            in_quote: self.quote_depth > 0,
            in_footnote: self.footnote_depth > 0,
            line_breaks: Vec::new(),
            marks: Vec::new(),
            pending_break: None,
        });
    }

    /// 段落タグのないテキスト (詰まったリストの項目など) のためにブロックを用意する。
    fn ensure_block(&mut self, range: &Range<usize>) -> &mut Current {
        if self.current.is_none() {
            let kind = if self.item_depth > 0 {
                BlockKind::ListItem
            } else {
                BlockKind::Paragraph
            };
            self.start_block(kind, range);
        }
        self.current.as_mut().expect("current block")
    }

    fn flush(&mut self) {
        self.mark_stack.clear();
        let Some(cur) = self.current.take() else {
            return;
        };
        if cur.text.trim().is_empty() {
            return;
        }
        self.blocks.push(Block {
            kind: cur.kind,
            text: cur.text,
            map: cur.map,
            span: cur.span,
            in_quote: cur.in_quote,
            in_footnote: cur.in_footnote,
            line_breaks: cur.line_breaks,
            marks: cur.marks,
            sentences: 0..0,
        });
    }

    fn skipping(&self) -> bool {
        self.skip_depth > 0 || self.inline_skip > 0
    }

    fn event(&mut self, event: Event<'_>, range: Range<usize>) {
        match event {
            Event::Start(tag) => self.start(tag, range),
            Event::End(tag) => self.end(tag, range),
            Event::Text(t) => {
                if !self.skipping() {
                    let source = self.source;
                    self.ensure_block(&range).push_text(source, &t, range);
                }
            }
            Event::Code(_) => {
                if !self.skipping() {
                    self.ensure_block(&range)
                        .push_placeholder(range, MarkKind::Code);
                }
            }
            Event::InlineMath(_) | Event::DisplayMath(_) => {
                if !self.skipping() {
                    self.ensure_block(&range)
                        .push_placeholder(range, MarkKind::Math);
                }
            }
            Event::Html(_) => {
                // HTML ブロックの中身は Start(HtmlBlock) の範囲でまとめて読む
            }
            Event::InlineHtml(_) => {
                self.html_spans.push(Span::new(range.start, range.end));
            }
            Event::SoftBreak | Event::HardBreak => {
                if !self.skipping()
                    && let Some(cur) = self.current.as_mut()
                {
                    cur.line_break(range);
                }
            }
            Event::Rule => self.flush(),
            Event::FootnoteReference(_) | Event::TaskListMarker(_) => {}
        }
    }

    fn start(&mut self, tag: Tag<'_>, range: Range<usize>) {
        match tag {
            Tag::Paragraph => {
                let kind = if self.item_depth > 0 {
                    BlockKind::ListItem
                } else {
                    BlockKind::Paragraph
                };
                self.start_block(kind, &range);
            }
            Tag::Heading { level, .. } => {
                self.start_block(BlockKind::Heading(heading_level(level)), &range);
            }
            Tag::BlockQuote(_) => {
                self.flush();
                self.quote_depth += 1;
            }
            Tag::CodeBlock(_) | Tag::MetadataBlock(_) => {
                self.flush();
                self.skip_depth += 1;
            }
            Tag::HtmlBlock => {
                self.flush();
                self.html_spans.push(Span::new(range.start, range.end));
            }
            Tag::List(_) => self.flush(),
            Tag::Item => {
                self.flush();
                self.item_depth += 1;
            }
            Tag::FootnoteDefinition(_) => {
                self.flush();
                self.footnote_depth += 1;
            }
            Tag::Table(_) | Tag::TableHead | Tag::TableRow => self.flush(),
            Tag::TableCell => self.start_block(BlockKind::TableCell, &range),
            Tag::Emphasis | Tag::Strong | Tag::Strikethrough => {
                if self.skipping() {
                    return;
                }
                let kind = match tag {
                    Tag::Emphasis => MarkKind::Emphasis,
                    Tag::Strong => MarkKind::Strong,
                    _ => MarkKind::Strikethrough,
                };
                let at = self.ensure_block(&range).text.len();
                self.mark_stack
                    .push((kind, at, Span::new(range.start, range.end)));
            }
            Tag::Link { link_type, .. } => {
                if self.skipping() {
                    self.link_stack.push(false);
                    return;
                }
                let autolink = matches!(link_type, LinkType::Autolink | LinkType::Email);
                self.link_stack.push(autolink);
                if autolink {
                    self.ensure_block(&range)
                        .push_placeholder(range, MarkKind::Link);
                    self.inline_skip += 1;
                } else {
                    let at = self.ensure_block(&range).text.len();
                    self.mark_stack
                        .push((MarkKind::Link, at, Span::new(range.start, range.end)));
                }
            }
            Tag::Image { .. } => {
                if !self.skipping() {
                    self.ensure_block(&range)
                        .push_placeholder(range, MarkKind::Image);
                }
                self.inline_skip += 1;
            }
            Tag::DefinitionList
            | Tag::DefinitionListTitle
            | Tag::DefinitionListDefinition
            | Tag::Superscript
            | Tag::Subscript => {}
        }
    }

    fn end(&mut self, tag: TagEnd, _range: Range<usize>) {
        match tag {
            TagEnd::Paragraph | TagEnd::Heading(_) | TagEnd::TableCell => self.flush(),
            TagEnd::BlockQuote(_) => {
                self.flush();
                self.quote_depth = self.quote_depth.saturating_sub(1);
            }
            TagEnd::CodeBlock | TagEnd::MetadataBlock(_) => {
                self.skip_depth = self.skip_depth.saturating_sub(1);
            }
            TagEnd::HtmlBlock => {}
            TagEnd::List(_) => self.flush(),
            TagEnd::Item => {
                self.flush();
                self.item_depth = self.item_depth.saturating_sub(1);
            }
            TagEnd::FootnoteDefinition => {
                self.flush();
                self.footnote_depth = self.footnote_depth.saturating_sub(1);
            }
            TagEnd::Table | TagEnd::TableHead | TagEnd::TableRow => self.flush(),
            TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough => {
                let kind = match tag {
                    TagEnd::Emphasis => MarkKind::Emphasis,
                    TagEnd::Strong => MarkKind::Strong,
                    _ => MarkKind::Strikethrough,
                };
                self.close_mark(kind);
            }
            TagEnd::Link => {
                if self.link_stack.pop() == Some(true) {
                    self.inline_skip = self.inline_skip.saturating_sub(1);
                } else {
                    self.close_mark(MarkKind::Link);
                }
            }
            TagEnd::Image => {
                self.inline_skip = self.inline_skip.saturating_sub(1);
            }
            TagEnd::DefinitionList
            | TagEnd::DefinitionListTitle
            | TagEnd::DefinitionListDefinition
            | TagEnd::Superscript
            | TagEnd::Subscript => {}
        }
    }

    fn close_mark(&mut self, kind: MarkKind) {
        let Some(pos) = self.mark_stack.iter().rposition(|(k, _, _)| *k == kind) else {
            return;
        };
        let (kind, start, span) = self.mark_stack.remove(pos);
        if let Some(cur) = self.current.as_mut() {
            let end = cur.text.len();
            cur.marks.push(InlineMark {
                kind,
                range: start..end,
                span,
            });
        }
    }

    fn finish(mut self) -> (Vec<Block>, Vec<Directive>) {
        self.flush();
        let mut directives = Vec::new();
        self.html_spans.sort();
        self.html_spans.dedup();
        for span in &self.html_spans {
            directive::scan(self.source, *span, &mut directives);
        }
        directives.sort_by_key(|d| d.span.start);
        directives.dedup_by_key(|d| d.span.start);
        for block in &mut self.blocks {
            block.marks.sort_by_key(|m| (m.range.start, m.range.end));
        }
        (self.blocks, directives)
    }
}

fn heading_level(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::DirectiveKind;

    fn blocks(src: &str) -> Vec<Block> {
        parse(src).0
    }

    #[test]
    fn separates_block_kinds() {
        let src = "# 見出し\n\n本文の段落です。\n\n- 項目一\n- 項目二\n\n> 引用です。\n\n| 列 |\n|---|\n| セル |\n";
        let b = blocks(src);
        let kinds: Vec<_> = b.iter().map(|b| (b.kind, b.in_quote)).collect();
        assert_eq!(
            kinds,
            vec![
                (BlockKind::Heading(1), false),
                (BlockKind::Paragraph, false),
                (BlockKind::ListItem, false),
                (BlockKind::ListItem, false),
                (BlockKind::Paragraph, true),
                (BlockKind::TableCell, false),
                (BlockKind::TableCell, false),
            ]
        );
        assert_eq!(b[0].text, "見出し");
        assert_eq!(b[2].text, "項目一");
    }

    #[test]
    fn drops_code_and_keeps_offsets_exact() {
        let src = "前文です。\n\n```rust\nlet x = \"と言えるでしょう\";\n```\n\n`code`を実行すると言えるでしょう。\n";
        let b = blocks(src);
        assert_eq!(b.len(), 2);
        assert_eq!(b[1].text, "\u{FFFC}を実行すると言えるでしょう。");
        let pos = b[1].text.find("と言える").unwrap();
        let span = b[1].to_source(pos..pos + "と言える".len());
        assert_eq!(&src[span.range()], "と言える");
        let code = b[1].to_source(0..PLACEHOLDER.len_utf8());
        assert_eq!(&src[code.range()], "`code`");
    }

    #[test]
    fn strips_emphasis_and_records_marks() {
        let src = "これは**重要なポイント**です。[リンク](https://example.com)も読む。\n";
        let b = blocks(src);
        assert_eq!(b[0].text, "これは重要なポイントです。リンクも読む。");
        let strong = b[0]
            .marks
            .iter()
            .find(|m| m.kind == MarkKind::Strong)
            .unwrap();
        assert_eq!(&b[0].text[strong.range.clone()], "重要なポイント");
        assert_eq!(&src[strong.span.range()], "**重要なポイント**");
        assert!(b[0].marks.iter().any(|m| m.kind == MarkKind::Link));
    }

    #[test]
    fn joins_soft_breaks_between_japanese_and_spaces_between_words() {
        let src = "日本語の文が\n折り返される。English words\nwrap here.\n";
        let b = blocks(src);
        assert_eq!(
            b[0].text,
            "日本語の文が折り返される。English words wrap here."
        );
        assert_eq!(b[0].line_breaks.len(), 2);
    }

    #[test]
    fn images_and_autolinks_become_placeholders() {
        let src = "図は![代替テキスト](a.png)の通り。<https://example.com/?a=1>を見る。\n";
        let b = blocks(src);
        assert_eq!(b[0].text, "図は\u{FFFC}の通り。\u{FFFC}を見る。");
    }

    #[test]
    fn skips_front_matter() {
        let src = "---\ntitle: と言えるでしょう\n---\n\n本文。\n";
        let b = blocks(src);
        assert_eq!(b.len(), 1);
        assert_eq!(b[0].text, "本文。");
    }

    #[test]
    fn reads_directives_from_html_but_not_code() {
        let src = "<!-- noslop-disable-next-line P01 -- 引用 -->\n本文と言えるでしょう。\n\n```\n<!-- noslop-disable-file -->\n```\n\n文中<!-- noslop-disable-line R03 -->です。\n";
        let (_, d) = parse(src);
        assert_eq!(d.len(), 2);
        assert_eq!(d[0].kind, DirectiveKind::DisableNextLine);
        assert_eq!(d[0].rules, vec!["P01"]);
        assert_eq!(d[1].kind, DirectiveKind::DisableLine);
    }

    #[test]
    fn loose_list_paragraphs_and_nested_lists_are_list_items() {
        let src = "- 親の項目\n  - 子の項目\n\n- 段落のある項目\n\n  二段落目。\n";
        let b = blocks(src);
        assert!(b.iter().all(|b| b.kind == BlockKind::ListItem));
        let texts: Vec<_> = b.iter().map(|b| b.text.as_str()).collect();
        assert_eq!(
            texts,
            vec!["親の項目", "子の項目", "段落のある項目", "二段落目。"]
        );
    }

    #[test]
    fn entities_map_to_their_source_range() {
        let src = "A&amp;Bと言えるでしょう。\n";
        let b = blocks(src);
        assert_eq!(b[0].text, "A&Bと言えるでしょう。");
        let pos = b[0].text.find('&').unwrap();
        let span = b[0].to_source(pos..pos + 1);
        assert_eq!(&src[span.range()], "&amp;");
        let pos = b[0].text.find("と言える").unwrap();
        let span = b[0].to_source(pos..pos + "と言える".len());
        assert_eq!(&src[span.range()], "と言える");
    }
}
