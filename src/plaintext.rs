//! テキストファイルをブロックの列に変換する。
//!
//! - 空行で段落を区切る。ただし空行がほとんどない文書 (1 行 1 段落で書く書式) は、
//!   行ごとに段落とみなす
//! - 全角空白の字下げ・かぎ括弧で始まる行は新しい段落とみなす (小説・随筆の書式)
//! - 「・」「- 」「1. 」「①」などで始まる行は箇条書きの項目として 1 行 1 ブロックにする
//! - 文末記号のない短い 1 行だけの段落 (「第一章」「議題」など) は見出しとみなす
//! - HTML コメント (`<!-- ... -->`) は本文から除き、抑制コメントとして読む

use std::sync::LazyLock;

use regex::Regex;

use crate::diagnostic::Span;
use crate::directive;
use crate::document::{Block, BlockKind, Directive, TextMap};
use crate::text;

static BULLET_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:[・•●○◦▪■□◆◇※]|[-*+]\s|\d{1,3}[.)．）]\s?|[①-⑳]|[（(]\d{1,3}[)）])")
        .expect("bullet regex")
});

/// 見出しとみなす 1 行段落の最大文字数。
const HEADING_MAX_CHARS: usize = 20;

/// 1 行分の読み取り結果。
struct Line {
    /// HTML コメントを除いた部分 (原文のバイト範囲)。
    pieces: Vec<(usize, usize)>,
    /// 行頭の半角空白を除いた本文。
    content: String,
    /// 原文で空白だけの行か (段落の区切り)。
    blank: bool,
}

impl Line {
    /// 段落を区切る空行か。原文で空白だけの行に限る。
    fn is_blank(&self) -> bool {
        self.blank
    }

    /// コメントを除くと何も残らない行か。コメントだけの行は段落を区切らず、読み飛ばす。
    fn is_invisible(&self) -> bool {
        !self.blank && self.content.trim().is_empty()
    }

    fn has_text(&self) -> bool {
        !self.blank && !self.is_invisible()
    }
}

/// テキストの原文をブロックと抑制コメントに分ける。
pub fn parse(source: &str) -> (Vec<Block>, Vec<Directive>) {
    let mut directives = Vec::new();
    directive::scan(source, Span::new(0, source.len()), &mut directives);
    let comments = directive::comment_spans(source);

    let mut lines = Vec::new();
    let mut offset = 0usize;
    // コメントは昇順で重ならないので、この行より前で終わったコメントは以後の行にも関係しない。
    // 先頭を進めていけば、各コメントを見るのは合わせて 1 回で済む。
    let mut pending_comments = 0usize;
    for raw_line in source.split_inclusive('\n') {
        let line_start = offset;
        offset += raw_line.len();
        let line = raw_line.trim_end_matches(['\n', '\r']);
        while comments
            .get(pending_comments)
            .is_some_and(|c| c.end <= line_start)
        {
            pending_comments += 1;
        }
        let pieces = visible_pieces(line_start, line, &comments[pending_comments..]);
        let visible: String = pieces.iter().map(|&(s, e)| &source[s..e]).collect();
        let content = visible.trim_start_matches([' ', '\t']).to_string();
        lines.push(Line {
            pieces,
            content,
            blank: line.trim().is_empty(),
        });
    }
    let line_per_paragraph = uses_line_per_paragraph(&lines);

    let mut blocks = Vec::new();
    let mut current: Option<Draft> = None;
    for line in &lines {
        if line.is_blank() {
            if let Some(d) = current.take() {
                blocks.push(d.finish());
            }
            continue;
        }
        if line.is_invisible() {
            continue;
        }

        let bullet = BULLET_RE.is_match(&line.content);
        let starts_paragraph =
            line_per_paragraph || line.content.starts_with(['\u{3000}', '「', '『']);
        if (bullet || starts_paragraph)
            && let Some(d) = current.take()
        {
            blocks.push(d.finish());
        }
        let draft = current.get_or_insert_with(|| {
            Draft::new(if bullet {
                BlockKind::ListItem
            } else {
                BlockKind::Paragraph
            })
        });
        draft.push_line(source, &line.pieces);
        if bullet && let Some(d) = current.take() {
            blocks.push(d.finish());
        }
    }
    if let Some(d) = current.take() {
        blocks.push(d.finish());
    }
    for block in &mut blocks {
        if looks_like_heading(block) {
            block.kind = BlockKind::Heading(0);
        }
    }
    (blocks, directives)
}

/// 空行による段落区切りがほとんどなく、1 行を 1 段落として書く書式か。
///
/// 本文の行が 3 行以上あり、本文にはさまれた空行 (連続は 1 つと数える) が
/// 本文の行数の 5% 以下なら、その書式とみなす。
fn uses_line_per_paragraph(lines: &[Line]) -> bool {
    let text_lines = lines.iter().filter(|l| l.has_text()).count();
    if text_lines < 3 {
        return false;
    }
    let mut separators = 0usize;
    let mut seen_text = false;
    let mut in_gap = false;
    for line in lines {
        if line.is_blank() {
            in_gap = seen_text;
        } else if line.has_text() {
            if in_gap {
                separators += 1;
            }
            in_gap = false;
            seen_text = true;
        }
    }
    separators * 20 <= text_lines
}

/// 文末記号のない短い 1 行だけの段落 (「第一章」「議題」) か。
fn looks_like_heading(block: &Block) -> bool {
    let text = block.text.trim();
    block.kind == BlockKind::Paragraph
        && block.line_breaks.is_empty()
        && text.chars().count() <= HEADING_MAX_CHARS
        && !text.chars().any(text::is_sentence_ender)
        && !text.ends_with(text::is_closing_bracket)
}

/// 行のうち、HTML コメントを除いた部分 (原文のバイト範囲) を返す。
///
/// `comments` は昇順で重ならず、この行より前で終わるものを取り除いてあること。
/// 行の終わりより後ろで始まるコメントに達したら打ち切る。
fn visible_pieces(line_start: usize, line: &str, comments: &[Span]) -> Vec<(usize, usize)> {
    let line_end = line_start + line.len();
    let mut pieces = Vec::new();
    let mut cursor = line_start;
    for c in comments
        .iter()
        .take_while(|c| c.start < line_end)
        .filter(|c| c.end > line_start)
    {
        if c.start > cursor {
            pieces.push((cursor, c.start));
        }
        cursor = cursor.max(c.end);
    }
    if cursor < line_end {
        pieces.push((cursor, line_end));
    }
    pieces
}

struct Draft {
    kind: BlockKind,
    text: String,
    map: TextMap,
    span: Option<Span>,
    line_breaks: Vec<usize>,
}

impl Draft {
    fn new(kind: BlockKind) -> Self {
        Self {
            kind,
            text: String::new(),
            map: TextMap::default(),
            span: None,
            line_breaks: Vec::new(),
        }
    }

    fn push_line(&mut self, source: &str, pieces: &[(usize, usize)]) {
        let mut first_piece = true;
        for &(start, end) in pieces {
            let mut start = start;
            if first_piece {
                // 行頭の字下げ (半角・全角空白) は本文に含めない
                let body = &source[start..end];
                start += body.len() - body.trim_start_matches([' ', '\t', '\u{3000}']).len();
                if start >= end {
                    continue;
                }
                if !self.text.is_empty() {
                    self.line_breaks.push(self.text.len());
                    let prev = self.text.chars().next_back();
                    let next = source[start..end].chars().next();
                    if let (Some(p), Some(n)) = (prev, next)
                        && !(text::is_cjk_like(p) && text::is_cjk_like(n))
                        && !p.is_whitespace()
                    {
                        let at = self.text.len();
                        self.text.push(' ');
                        self.map.push_opaque(at, 1, Span::new(start, start));
                    }
                }
                first_piece = false;
            }
            let at = self.text.len();
            self.text.push_str(&source[start..end]);
            self.map.push_exact(at, start, end - start);
            self.span = Some(match self.span {
                Some(s) => Span::new(s.start.min(start), s.end.max(end)),
                None => Span::new(start, end),
            });
        }
    }

    fn finish(self) -> Block {
        Block {
            kind: self.kind,
            text: self.text,
            map: self.map,
            span: self.span.unwrap_or(Span::new(0, 0)),
            in_quote: false,
            in_footnote: false,
            line_breaks: self.line_breaks,
            // 箇条書きの行は、行ごとに別のブロックにしてある
            sentence_breaks: Vec::new(),
            marks: Vec::new(),
            sentences: 0..0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(src: &str) -> Vec<(BlockKind, String)> {
        parse(src).0.into_iter().map(|b| (b.kind, b.text)).collect()
    }

    #[test]
    fn splits_paragraphs_on_blank_lines_and_joins_lines() {
        let src = "一段落目の\n続きです。\n\n二段落目です。\n";
        assert_eq!(
            texts(src),
            vec![
                (BlockKind::Paragraph, "一段落目の続きです。".to_string()),
                (BlockKind::Paragraph, "二段落目です。".to_string()),
            ]
        );
    }

    #[test]
    fn indentation_and_dialog_start_new_paragraphs() {
        let src = "　最初の段落。\n「会話だ」\n　次の段落。\n";
        let t = texts(src);
        assert_eq!(t.len(), 3);
        assert_eq!(t[0].1, "最初の段落。");
        assert_eq!(t[1].1, "「会話だ」");
        assert_eq!(t[2].1, "次の段落。");
    }

    #[test]
    fn bullets_become_list_items_and_short_label_becomes_heading() {
        let src = "議題\n・予算の確認\n・日程の調整\n1. 次回の担当\n";
        let t = texts(src);
        assert_eq!(t[0], (BlockKind::Heading(0), "議題".to_string()));
        assert!(t[1..].iter().all(|(k, _)| *k == BlockKind::ListItem));
        assert_eq!(t.len(), 4);
    }

    #[test]
    fn documents_without_blank_lines_use_one_paragraph_per_line() {
        let src =
            "一\n最初の段落の文です。二つ目の文です。\n次の段落の文です。\n最後の段落です。\n";
        let t = texts(src);
        assert_eq!(
            t,
            vec![
                (BlockKind::Heading(0), "一".to_string()),
                (
                    BlockKind::Paragraph,
                    "最初の段落の文です。二つ目の文です。".to_string()
                ),
                (BlockKind::Paragraph, "次の段落の文です。".to_string()),
                (BlockKind::Paragraph, "最後の段落です。".to_string()),
            ]
        );
    }

    #[test]
    fn hard_wrapped_paragraphs_with_blank_lines_are_joined() {
        let src = "一行目の途中で\n折り返した段落です。\n\n二つ目の段落も\n折り返しています。\n\n三つ目です。\n";
        let t = texts(src);
        assert_eq!(t.len(), 3);
        assert_eq!(t[0].1, "一行目の途中で折り返した段落です。");
        assert_eq!(t[1].1, "二つ目の段落も折り返しています。");
    }

    #[test]
    fn comments_spanning_lines_and_many_comments_are_removed() {
        let src = "前の文。<!-- 複数行に\nまたがる\nコメント -->後の文。\n";
        let (blocks, _) = parse(src);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].text, "前の文。後の文。");

        // 行ごとにコメントがある長い文書でも、コメントの照合が行数の 2 乗にならない
        let many: String = (0..20_000)
            .map(|i| format!("文{i}です。<!-- m{i} -->\n"))
            .collect();
        let (blocks, _) = parse(&many);
        assert_eq!(blocks.len(), 20_000);
        assert_eq!(blocks[19_999].text, "文19999です。");
    }

    #[test]
    fn comments_are_removed_from_text_and_read_as_directives() {
        let src =
            "<!-- noslop-disable-next-line P01 -->\n本文と言えるでしょう。<!-- メモ -->続き。\n";
        let (blocks, directives) = parse(src);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].text, "本文と言えるでしょう。続き。");
        assert_eq!(directives.len(), 1);
        let pos = blocks[0].text.find("続き").unwrap();
        let span = blocks[0].to_source(pos..pos + "続き".len());
        assert_eq!(&src[span.range()], "続き");
    }
}
