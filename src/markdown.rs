//! Markdown をブロックの列に変換する。
//!
//! 解析用テキストには地の文として読まれる文字だけを残す。
//!
//! - コードブロック・HTML ブロック・数式ブロックと、文書の先頭の front matter は捨てる
//!   (front matter は先頭にあるものだけ。途中の `---` は区切り線か見出しの下線として読む)
//! - インラインコード・数式・画像・自動リンクはプレースホルダ 1 字に畳む
//! - 太字・リンクなどの装飾記号は取り除き、装飾の範囲は [`InlineMark`] に残す
//! - ソフト改行は日本語どうしの間なら何も入れず、英数字が隣り合うなら空白 1 つにする
//!
//! 抑制コメントは HTML ブロックとインライン HTML からだけ読む (コードブロック内の
//! 記法例を誤って抑制として扱わないため)。

use std::ops::Range;
use std::sync::LazyLock;

use pulldown_cmark::{Event, HeadingLevel, LinkType, Options, Parser, Tag, TagEnd};
use regex::Regex;

use crate::diagnostic::Span;
use crate::directive;
use crate::document::{Block, BlockKind, Directive, InlineMark, MarkKind, TextMap};
use crate::text::{self, PLACEHOLDER};

/// pulldown-cmark の拡張記法。
///
/// メタデータブロック (`ENABLE_YAML_STYLE_METADATA_BLOCKS` など) は有効にしない。
/// pulldown-cmark はこれを文書の途中にも当てるため、段落の直後でない `---` の行から
/// 次の `---` か `...` の行までの本文がメタデータとして捨てられてしまう。先頭の
/// front matter は [`front_matter_len`] で見つけて解析から外す。
fn options() -> Options {
    Options::ENABLE_TABLES
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_HEADING_ATTRIBUTES
        | Options::ENABLE_MATH
        | Options::ENABLE_GFM
}

/// Markdown の記法にない箇条書きの記号か番号で始まる行 (1 行 1 項目の箇条書き)。
///
/// 番号は「1.」「(1)」「１．」の形で、直後が数字なら小数 (「3.5 倍」) とみなして外す。
static ITEM_LINE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^(?:[・･•●○◯◉◦▪■□◆◇♦※★☆◎▲△▼▽▶▷►➤➡➔→⇒✓✔✅☑☐✗✕]|[①-⑳]|[⑴-⒇]|[（(]\d{1,3}[)）]|\d{1,3}[.)．）](?:[^\d.．]|$)|[０-９]{1,3}[．）](?:[^０-９]|$))",
    )
    .expect("item line regex")
});

/// 短い見出しと全角のコロンで始まる行 (「特長（Advantage）：〜」)。
static LABEL_LINE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[^\s。、，．：:]{1,20}：").expect("label line regex"));

/// 丁寧体の文末 (句点を打たずに 1 行 1 文で書いた文の終わり)。
static POLITE_END: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:です|ます|ました|でした|ません|ましょう|でしょう|ください)$")
        .expect("polite end regex")
});

/// 段落の中の改行が、書式から文の区切りと分かるか (1 行 1 項目で書いた箇条書きとラベルの行)。
///
/// 次のどれかなら、改行の扱いの設定によらず文を切る。本文の折り返しで文の途中に改行を置く
/// 書き方は、これらに当たらないのでつないだままにする。
///
/// - 次の行が、Markdown の記法にない箇条書きの記号か番号で始まる (「・項目」「①項目」)
/// - 次の行が、短い見出しと全角のコロンで始まる (「特長：〜」)
/// - 直前の行がコロンで終わる (「目的:」)
/// - 直前の行が【】で囲んだ見出し風の行か、太字だけの行である
/// - 直前の行が日本語を含まない英文で `.` `!` `?` で終わり、次の行が日本語で始まる
/// - ハード改行 (行末の空白 2 つ・バックスラッシュ) の直前が、文末記号・読点・ひらがなのどれでも
///   ない (見出し風の行の後の改行。ひらがなや読点で終わる行は、文の途中の折り返しとみなす)
/// - 直前の行が丁寧体の文末 (「です」「ます」「ください」など) で終わり、次の行が括弧で始まらない
///   (句点を打たずに 1 行 1 文で書いた文)
fn breaks_sentence(source: &str, range: &Range<usize>, hard: bool) -> bool {
    let before = &source[..range.start];
    let after = &source[range.end..];
    lines_break_sentence(
        &before[before.rfind('\n').map_or(0, |i| i + 1)..],
        &after[..after.find('\n').unwrap_or(after.len())],
        hard,
    )
}

/// 前の行 `prev_line` から次の行 `next_line` へ移る改行が、書式から文の区切りと分かるか
/// (判定は [`breaks_sentence`] と同じ)。行は引用の記号・字下げを含んだままでよい。
///
/// コードのコメント ([`crate::code`]) でも、コメントの記号を外した行どうしで同じ判定を使う。
pub(crate) fn lines_break_sentence(prev_line: &str, next_line: &str, hard: bool) -> bool {
    /// 引用の記号・字下げを外した行。
    fn unquoted(line: &str) -> &str {
        line.trim_start_matches(|c: char| c == '>' || c.is_whitespace())
            .trim_end_matches(|c: char| c == '\\' || c.is_whitespace())
    }
    /// 太字の記号も外した行。
    fn bare(line: &str) -> &str {
        unquoted(line).trim_matches(|c: char| c == '*' || c == '_' || c.is_whitespace())
    }
    let prev_line = unquoted(prev_line);
    let prev = bare(prev_line);
    let next = bare(next_line);
    let bold_line = ["**", "__"].iter().any(|m| {
        prev_line.len() > 2 * m.len() && prev_line.starts_with(m) && prev_line.ends_with(m)
    });
    let english_line = prev.ends_with(['.', '!', '?'])
        && !prev.chars().any(text::is_japanese)
        && next.chars().next().is_some_and(text::is_japanese);
    // 長音符はカタカナ語の終わり (「サーバー」) にも付くので、ひらがなとみなさない
    let hard_after_label = hard
        && prev.chars().next_back().is_some_and(|c| {
            !(text::is_sentence_ender(c)
                || matches!(c, '\u{3041}'..='\u{309F}' | '、' | '，' | ','))
        });
    // 次の行が括弧で始まるなら、同じ文の補足 (「〜します\n(既定は〜)。」) とみなしてつなぐ
    let polite_end = POLITE_END.is_match(prev)
        && !next
            .chars()
            .next()
            .is_some_and(|c| text::closing_bracket(c).is_some());
    ITEM_LINE.is_match(next)
        || LABEL_LINE.is_match(next)
        || prev.ends_with([':', '：'])
        || (prev.starts_with('【') && prev.ends_with('】'))
        || bold_line
        || english_line
        || hard_after_label
        || polite_end
}

/// Markdown の原文をブロックと抑制コメントに分ける。
///
/// `source` は先頭の BOM を除いた原文 ([`crate::document::Document::parse`] が除く)。
/// 先頭の front matter の後ろだけを解析し、位置は原文上のバイト位置に戻す。
pub fn parse(source: &str) -> (Vec<Block>, Vec<Directive>) {
    let mut builder = Builder::new(source);
    let body = front_matter_len(source);
    for (event, range) in Parser::new_ext(&source[body..], options()).into_offset_iter() {
        builder.event(event, range.start + body..range.end + body);
    }
    builder.finish()
}

/// 文書の先頭にある front matter の長さ (閉じの行の改行まで含むバイト数)。なければ 0。
///
/// 開き・閉じの判定は pulldown-cmark のメタデータブロックと同じにして、先頭の
/// front matter の扱いを変えない。違うのは、文書の 1 行目から始まるものだけを
/// front matter とみなす点。
///
/// - 開きは 1 行目の `---` (YAML) か `+++` (TOML)。ちょうど 3 字で、後ろは空白だけ
/// - 閉じは行頭の `---` か `...` (TOML は `+++`)。ちょうど 3 字で、後ろはスペースだけ
/// - 開きの次の行が空行か閉じの行なら front matter ではない (区切り線として読む)
/// - 閉じの行がなければ front matter ではない (全体を本文として読む)
fn front_matter_len(source: &str) -> usize {
    let bytes = source.as_bytes();
    let fence = match bytes.first() {
        Some(&c @ (b'-' | b'+')) => c,
        _ => return 0,
    };
    let opening_end = next_line_start(bytes, 0);
    let opening = &bytes[..opening_end];
    if run_of(opening, fence) != 3 || !opening[3..].iter().all(u8::is_ascii_whitespace) {
        return 0;
    }
    let mut pos = opening_end;
    let mut first = true;
    while pos < bytes.len() {
        let line = &bytes[pos..];
        if let Some(len) = closing_line_len(line, fence) {
            return if first { 0 } else { pos + len };
        }
        if first && line_end_len(skip_blanks(line)).is_some() {
            return 0;
        }
        first = false;
        pos = next_line_start(bytes, pos);
    }
    0
}

/// front matter を閉じる行なら、その長さ (改行を含む)。
fn closing_line_len(line: &[u8], fence: u8) -> Option<usize> {
    let marker = if run_of(line, fence) == 3 || (fence == b'-' && run_of(line, b'.') == 3) {
        3
    } else {
        return None;
    };
    let spaces = run_of(&line[marker..], b' ');
    let eol = line_end_len(&line[marker + spaces..])?;
    Some(marker + spaces + eol)
}

/// 先頭から `c` が続く字数。
fn run_of(bytes: &[u8], c: u8) -> usize {
    bytes.iter().take_while(|&&b| b == c).count()
}

/// 行頭の空白 (スペース・タブ・垂直タブ・改ページ) を飛ばした残り。
fn skip_blanks(line: &[u8]) -> &[u8] {
    let n = line
        .iter()
        .take_while(|&&b| matches!(b, b' ' | b'\t' | 0x0b | 0x0c))
        .count();
    &line[n..]
}

/// `bytes` が行末 (改行か文書の終わり) で始まるなら、改行の長さ。
fn line_end_len(bytes: &[u8]) -> Option<usize> {
    match bytes {
        [] => Some(0),
        [b'\r', b'\n', ..] => Some(2),
        [b'\n' | b'\r', ..] => Some(1),
        _ => None,
    }
}

/// `pos` から始まる行の次の行の先頭 (最後の行なら文書の終わり)。
fn next_line_start(bytes: &[u8], pos: usize) -> usize {
    bytes[pos..]
        .iter()
        .position(|&b| b == b'\n')
        .map_or(bytes.len(), |i| pos + i + 1)
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
    sentence_breaks: Vec<usize>,
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

    /// 改行を記録する。`sentence` は、書式から文の区切りと分かる改行か ([`breaks_sentence`])。
    fn line_break(&mut self, range: Range<usize>, sentence: bool) {
        self.line_breaks.push(self.text.len());
        if sentence {
            self.sentence_breaks.push(self.text.len());
        }
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
    /// コードブロックの内側 (テキストを捨てる)。
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
            sentence_breaks: Vec::new(),
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
            sentence_breaks: cur.sentence_breaks,
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
                    let hard = matches!(event, Event::HardBreak);
                    let sentence = breaks_sentence(self.source, &range, hard);
                    cur.line_break(range, sentence);
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
            // メタデータブロックの記法は有効にしていないので来ないが、来ても中身は捨てる
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
        // 本文の折り返しは、文の区切りにしない
        assert!(b[0].sentence_breaks.is_empty());
    }

    /// 段落の中で、書式から文の区切りと分かる改行 (の直後の行の頭)。
    fn sentence_break_heads(src: &str) -> Vec<String> {
        let b = blocks(src);
        b[0].sentence_breaks
            .iter()
            .map(|&at| b[0].text[at..].trim_start().chars().take(3).collect())
            .collect()
    }

    #[test]
    fn item_lines_and_labels_break_sentences() {
        // Markdown の記法にない箇条書きの記号と番号
        let src = "機能は次の 3 つ\n・通知を減らす\n• 画面を速くする\n①設定を簡単にする\n(2) 共有する\n3. 保存する\n";
        assert_eq!(
            sentence_break_heads(src),
            ["・通知", "• 画", "①設定", "(2)", "3. "]
        );
        // コロンで終わるラベル (太字の記号は外して見る) と、【】で囲んだ見出し風の行
        let src = "**目的**：\n通知を減らす\n【背景】\n問い合わせが多い\n";
        assert_eq!(sentence_break_heads(src), ["通知を", "問い合"]);
        // 引用の中でも同じ
        let src = "> 手順:\n> ・保存する\n";
        assert_eq!(sentence_break_heads(src), ["・保存"]);
    }

    #[test]
    fn more_item_markers_and_label_lines_break_sentences() {
        // 大きな丸などの記号、短い見出しと全角コロンで始まる行
        let src =
            "振り返り\n◯計画どおりに進んだ\n▲見積もりが甘かった\n特長（速さ）：待たずに済む\n";
        assert_eq!(sentence_break_heads(src), ["◯計画", "▲見積", "特長（"]);
        // 太字だけの行のあと
        let src = "**確かめる質問**\n何を優先するかを聞く。\n";
        assert_eq!(sentence_break_heads(src), ["何を優"]);
    }

    #[test]
    fn english_lines_and_hard_breaks_after_labels_break_sentences() {
        // 英文の行の文末と、次の行の日本語
        let src = "Released under the MIT License.\nこの節は利用条件を述べる。\n";
        assert_eq!(sentence_break_heads(src), ["この節"]);
        // 見出し風の行のあとのハード改行 (行末の空白 2 つ、バックスラッシュ)
        let src = "**手順の概要（初回だけ）**  \n設定を開いて保存する。\n";
        assert_eq!(sentence_break_heads(src), ["設定を"]);
        let src = "作業の記録 ref-12\\\n次の作業に移る。\n";
        assert_eq!(sentence_break_heads(src), ["次の作"]);
        let src = "対象のサーバー  \n夜間に止める。\n";
        assert_eq!(sentence_break_heads(src), ["夜間に"]);
    }

    #[test]
    fn polite_sentence_ends_without_periods_break_sentences() {
        // 句点を打たずに 1 行 1 文で書いたメモ
        let src = "画面の説明を書いてください\n手順は短くまとめます\n図も入れたいです\n";
        assert_eq!(sentence_break_heads(src), ["手順は", "図も入"]);
        // 次の行が括弧で始まるなら、同じ文の補足としてつなぐ
        let src = "設定は起動時に読みます\n(既定は現在のディレクトリ)。\n";
        assert!(sentence_break_heads(src).is_empty());
    }

    #[test]
    fn hard_breaks_inside_sentences_and_mixed_lines_keep_sentences() {
        // ひらがな・読点で終わる行のハード改行は、文の途中の折り返し
        let src = "結果を利用者に  \n知らせるための表示で、  \nすぐ消える。\n";
        assert!(sentence_break_heads(src).is_empty());
        // 日本語を含む行の末尾の英字の略語や、英文どうしの折り返しでは切らない
        let src = "提供元は Example Inc.\nの子会社だ。\nIt runs fast.\nIt is small.\n";
        assert!(sentence_break_heads(src).is_empty());
    }

    #[test]
    fn wrapped_prose_and_decimals_do_not_break_sentences() {
        // 読点・助詞で折り返した本文と、小数で始まる行
        let src = "測った結果は、\n平均して\n3.5 倍に速くなった。値は\n１２．５ だった。\n";
        assert!(sentence_break_heads(src).is_empty());
    }

    #[test]
    fn images_and_autolinks_become_placeholders() {
        let src = "図は![代替テキスト](a.png)の通り。<https://example.com/?a=1>を見る。\n";
        let b = blocks(src);
        assert_eq!(b[0].text, "図は\u{FFFC}の通り。\u{FFFC}を見る。");
    }

    /// ブロックを「種類:解析用テキスト」で並べる (P は段落、L はリスト項目、H2 は見出し 2)。
    fn outline(src: &str) -> Vec<String> {
        blocks(src)
            .iter()
            .map(|b| {
                let kind = match b.kind {
                    BlockKind::Paragraph => "P".to_string(),
                    BlockKind::ListItem => "L".to_string(),
                    BlockKind::TableCell => "T".to_string(),
                    BlockKind::Heading(level) => format!("H{level}"),
                };
                format!("{kind}:{}", b.text)
            })
            .collect()
    }

    #[test]
    fn skips_front_matter() {
        let src = "---\ntitle: と言えるでしょう\n---\n\n本文。\n";
        let b = blocks(src);
        assert_eq!(b.len(), 1);
        assert_eq!(b[0].text, "本文。");
    }

    #[test]
    fn front_matter_at_the_top_is_skipped_in_every_form() {
        // YAML は --- か ... で閉じ、TOML は +++ で閉じる。CRLF でも、区切りの行の後ろに
        // 空白があっても同じ。本文の位置は原文上の位置のまま
        for src in [
            "---\ntitle: と言えるでしょう\n---\n本文と言えるでしょう。\n",
            "---\ntitle: と言えるでしょう\n...\n本文と言えるでしょう。\n",
            "+++\ntitle = \"と言えるでしょう\"\n+++\n本文と言えるでしょう。\n",
            "---\r\ntitle: と言えるでしょう\r\n---\r\n本文と言えるでしょう。\r\n",
            "--- \t\ntitle: と言えるでしょう\n---  \n\n本文と言えるでしょう。\n",
        ] {
            let b = blocks(src);
            assert_eq!(b.len(), 1, "{src:?}");
            assert_eq!(b[0].text, "本文と言えるでしょう。", "{src:?}");
            let pos = b[0].text.find("と言える").unwrap();
            let span = b[0].to_source(pos..pos + "と言える".len());
            assert_eq!(span.start, src.rfind("と言える").unwrap(), "{src:?}");
            assert_eq!(&src[span.range()], "と言える");
        }
        // 閉じの行で文書が終わってもよい
        assert!(blocks("---\ntitle: 表題\n---").is_empty());
    }

    /// 以前の解析 (pulldown-cmark のメタデータブロックの記法で front matter を飛ばす)。
    fn parse_with_metadata_blocks(source: &str) -> (Vec<Block>, Vec<Directive>) {
        let options = options()
            | Options::ENABLE_YAML_STYLE_METADATA_BLOCKS
            | Options::ENABLE_PLUSES_DELIMITED_METADATA_BLOCKS;
        let mut builder = Builder::new(source);
        for (event, range) in Parser::new_ext(source, options).into_offset_iter() {
            builder.event(event, range);
        }
        builder.finish()
    }

    #[test]
    fn front_matter_at_the_top_is_read_as_pulldown_cmark_did() {
        // 1 行目から始まる front matter の判定と、その後ろの本文の読み方 (ブロック・位置・
        // 抑制コメント) は、以前の pulldown-cmark のメタデータブロックと変わらない
        for src in [
            // front matter になる形とならない形
            "---\ntitle: x\n---\n本文。\n",
            "---\ntitle: x\n...\n本文。\n",
            "+++\ntitle = 1\n+++\n本文。\n",
            "+++\ntitle = 1\n...\n本文。\n",
            "---\t \ntitle: x\n---   \n本文。\n",
            "---\ntitle: x\n---\t\n本文。\n",
            "---\ntitle: x\n----\n本文。\n",
            "----\ntitle: x\n----\n本文。\n",
            "--- x\ntitle: x\n---\n本文。\n",
            " ---\ntitle: x\n---\n本文。\n",
            "---\n---\n本文。\n",
            "---\n\ntitle: x\n---\n本文。\n",
            "---\n \t\ntitle: x\n---\n本文。\n",
            "---\ntitle: x\n\n本文。\n",
            "---\ntitle: x\n---",
            "---\n",
            "---",
            // 改行が CRLF・CR だけ・閉じの行だけ CR
            "---\r\ntitle: x\r\n---\r\n本文。\r\n",
            "---\rtitle: x\r---\r本文。\r",
            "---\ntitle: x\n---\r本文。\n",
            // front matter の直後の Markdown
            "---\na: b\n---\n[ref]: /url\n\n[ref]と言えるでしょう。\n",
            "---\na: b\n---\n    code\n\n本文。\n",
            "---\na: b\n---\n| A | B |\n|---|---|\n| 文 | 字 |\n",
            "---\na: b\n---\n<!-- noslop-disable-next-line P01 -->\n本文と言えるでしょう。\n",
            "---\na: b\n---\n本文[^1]。\n\n[^1]: 注の文。\n",
            "---\na: b\n---\n- 項目。\n  続き。\n",
        ] {
            assert_eq!(
                format!("{:#?}", parse(src)),
                format!("{:#?}", parse_with_metadata_blocks(src)),
                "{src:?}"
            );
        }
    }

    #[test]
    fn body_after_front_matter_keeps_lines_and_columns() {
        // BOM は Document が取り除くので、front matter は BOM の直後から始まる
        let src =
            "\u{FEFF}---\ntitle: 表題\ntags: [メモ]\n---\n\n# 見出し\n\n本文と言えるでしょう。\n";
        let doc = crate::document::Document::markdown(src);
        let texts: Vec<_> = doc.sentences.iter().map(|s| doc.sentence_text(s)).collect();
        assert_eq!(texts, ["見出し", "本文と言えるでしょう。"]);
        assert_eq!(doc.line_col(doc.sentences[0].span.start), (6, 3));
        assert_eq!(doc.line_col(doc.sentences[1].span.start), (8, 1));
        assert_eq!(
            doc.file_offset(doc.sentences[1].span.start),
            src.find("本文").unwrap()
        );
    }

    #[test]
    fn marker_lines_that_do_not_form_front_matter_are_read_as_markdown() {
        // 閉じの行がない: 区切り線として読み、後ろは本文 (捨てない)
        assert_eq!(
            outline("---\ntitle: 表題\n\n本文。\n"),
            ["P:title: 表題", "P:本文。"]
        );
        // 開きの次の行が空行: 区切り線。閉じのつもりの行は見出しの下線になる
        assert_eq!(
            outline("---\n\ntitle: 表題\n---\n\n本文。\n"),
            ["H2:title: 表題", "P:本文。"]
        );
        // 1 行目から始まらない
        assert_eq!(
            outline("\n---\ntitle: 表題\n---\n\n本文。\n"),
            ["H2:title: 表題", "P:本文。"]
        );
        // 中身がない、区切りが 4 字
        assert_eq!(outline("---\n---\n本文。\n"), ["P:本文。"]);
        assert_eq!(
            outline("----\ntitle: 表題\n----\n\n本文。\n"),
            ["H2:title: 表題", "P:本文。"]
        );
    }

    #[test]
    fn marker_lines_in_the_middle_do_not_swallow_the_body() {
        // 以前は段落の直後でない --- の行から、次の --- か ... の行までをメタデータとして
        // 捨てていた。閉じのつもりの --- の直前の行は、CommonMark のとおり見出しになる
        let cases = [
            (
                "さて、一。\n\n---\nさて、二。\n\nさて、三。\n---\n\nさて、四。\n",
                [
                    "P:さて、一。",
                    "P:さて、二。",
                    "H2:さて、三。",
                    "P:さて、四。",
                ],
            ),
            // 1 つ目の --- の後に空行がある (以前から区切り線)
            (
                "さて、一。\n\n---\n\nさて、二。\n\nさて、三。\n---\n\nさて、四。\n",
                [
                    "P:さて、一。",
                    "P:さて、二。",
                    "H2:さて、三。",
                    "P:さて、四。",
                ],
            ),
            // 閉じの行がない
            (
                "さて、一。\n\n---\nさて、二。\n\nさて、三。\n\nさて、四。\n",
                [
                    "P:さて、一。",
                    "P:さて、二。",
                    "P:さて、三。",
                    "P:さて、四。",
                ],
            ),
            // ... の行で閉じる形 (... は段落の続き)
            (
                "さて、一。\n\n---\nさて、二。\n\nさて、三。\n...\n\nさて、四。\n",
                [
                    "P:さて、一。",
                    "P:さて、二。",
                    "P:さて、三。 ...",
                    "P:さて、四。",
                ],
            ),
            // リスト項目や見出しの直後の ---、TOML の +++ も同じ
            (
                "- 項目。\n---\nさて、二。\n\nさて、三。\n---\n\nさて、四。\n",
                ["L:項目。", "P:さて、二。", "H2:さて、三。", "P:さて、四。"],
            ),
            (
                "# 見出し\n---\nさて、二。\n\nさて、三。\n---\n\nさて、四。\n",
                ["H1:見出し", "P:さて、二。", "H2:さて、三。", "P:さて、四。"],
            ),
            (
                "さて、一。\n\n+++\nさて、二。\n\nさて、三。\n+++\n\nさて、四。\n",
                [
                    "P:さて、一。",
                    "P:+++ さて、二。",
                    "P:さて、三。 +++",
                    "P:さて、四。",
                ],
            ),
        ];
        for (src, expected) in cases {
            assert_eq!(outline(src), expected, "{src:?}");
            let b = blocks(src);
            let last = b.last().unwrap();
            assert_eq!(
                &src[last.to_source(0..last.text.len()).range()],
                "さて、四。"
            );
        }
    }

    #[test]
    fn front_matter_is_skipped_only_at_the_top() {
        let src =
            "---\ntitle: さて、零。\n---\n\nさて、一。\n\n---\nさて、二。\n---\n\nさて、三。\n";
        assert_eq!(
            outline(src),
            ["P:さて、一。", "H2:さて、二。", "P:さて、三。"]
        );
        let b = blocks(src);
        assert_eq!(
            &src[b[1].to_source(0..b[1].text.len()).range()],
            "さて、二。"
        );
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
