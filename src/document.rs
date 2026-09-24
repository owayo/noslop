//! 解析対象の文書モデル。
//!
//! Markdown / テキストを「ブロック (段落・リスト項目・見出し・表セル)」の列にし、
//! 各ブロックの解析用テキストを文に分割したものを持つ。解析用テキストは原文から
//! 装飾・コード・URL を取り除いたもので、原文上の位置へは [`TextMap`] で戻す。

use std::ops::Range;
use std::path::Path;

use crate::diagnostic::Span;
use crate::segment::{self, LineBreakMode};
use crate::text;

/// 入力の形式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceFormat {
    Markdown,
    PlainText,
}

impl SourceFormat {
    /// 拡張子から形式を推定する。Markdown 系の拡張子以外はテキストとして扱う。
    pub fn from_path(path: &Path) -> Self {
        match path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .as_deref()
        {
            Some("md" | "markdown" | "mdown" | "mkd" | "mdx") => SourceFormat::Markdown,
            _ => SourceFormat::PlainText,
        }
    }
}

/// 文書の読み込み方の設定。
#[derive(Debug, Clone, Copy, Default)]
pub struct ParseOptions {
    pub line_breaks: LineBreakMode,
}

/// ブロックの種類。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockKind {
    /// 地の文の段落。
    Paragraph,
    /// 箇条書きの項目 (項目内の段落を含む)。
    ListItem,
    /// 見出し (レベル 1〜6)。テキスト入力で推定した見出しはレベル 0。
    Heading(u8),
    /// 表のセル。
    TableCell,
}

/// インラインの装飾の種類。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkKind {
    Strong,
    Emphasis,
    Strikethrough,
    Link,
    Code,
    Image,
    Math,
}

/// ブロック内のインライン装飾 (太字・リンクなど)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InlineMark {
    pub kind: MarkKind,
    /// 解析用テキスト上の範囲。
    pub range: Range<usize>,
    /// 原文上の範囲 (装飾記号を含む)。
    pub span: Span,
}

/// 解析用テキストの一区間と原文の対応。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Segment {
    text_start: usize,
    text_end: usize,
    src_start: usize,
    src_end: usize,
    /// 解析用テキストと原文が 1 バイトずつ対応する (そのまま写した) 区間か。
    /// `false` は実体参照・エスケープ・プレースホルダなどで、内部の位置は区間の端に丸める。
    exact: bool,
}

/// 解析用テキスト上のバイト位置を原文上のバイト位置に戻す対応表。
///
/// 解析用テキストは原文の連続した区間を並べたもので、区間ごとに対応を持つ。
/// 範囲が複数の区間をまたぐ場合は、始点を始点側の区間で、終点を終点側の区間で変換する。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TextMap {
    segments: Vec<Segment>,
}

impl TextMap {
    /// 原文の `src_start..src_start+len` をそのまま写した区間を追加する。
    pub fn push_exact(&mut self, text_start: usize, src_start: usize, len: usize) {
        if len == 0 {
            return;
        }
        self.segments.push(Segment {
            text_start,
            text_end: text_start + len,
            src_start,
            src_end: src_start + len,
            exact: true,
        });
    }

    /// 解析用テキストの `text_start..text_start+text_len` が原文の `src` 全体に対応する区間を追加する。
    pub fn push_opaque(&mut self, text_start: usize, text_len: usize, src: Span) {
        if text_len == 0 {
            return;
        }
        self.segments.push(Segment {
            text_start,
            text_end: text_start + text_len,
            src_start: src.start,
            src_end: src.end,
            exact: false,
        });
    }

    /// 解析用テキスト上の範囲を原文上の範囲に変換する。
    pub fn to_source(&self, range: Range<usize>) -> Span {
        if self.segments.is_empty() {
            return Span::new(0, 0);
        }
        let start = self.map_start(range.start);
        let end = self.map_end(range.end).max(start);
        Span::new(start, end)
    }

    fn map_start(&self, pos: usize) -> usize {
        // pos を含む区間 (text_start <= pos < text_end) を探す
        let idx = self.segments.partition_point(|s| s.text_end <= pos);
        match self.segments.get(idx) {
            Some(seg) if seg.text_start <= pos => {
                if seg.exact {
                    seg.src_start + (pos - seg.text_start)
                } else {
                    seg.src_start
                }
            }
            Some(seg) => seg.src_start,
            None => self.segments.last().map_or(0, |s| s.src_end),
        }
    }

    fn map_end(&self, pos: usize) -> usize {
        // pos を終点に含む区間 (text_start < pos <= text_end) を探す
        let idx = self.segments.partition_point(|s| s.text_end < pos);
        match self.segments.get(idx) {
            Some(seg) if seg.text_start < pos => {
                if seg.exact {
                    seg.src_start + (pos - seg.text_start)
                } else {
                    seg.src_end
                }
            }
            Some(seg) => seg.src_start,
            None => self.segments.last().map_or(0, |s| s.src_end),
        }
    }
}

/// 文書の 1 ブロック。
#[derive(Debug, Clone)]
pub struct Block {
    pub kind: BlockKind,
    /// 解析用テキスト (装飾記号・コード・画像・HTML を除いたもの)。
    pub text: String,
    pub map: TextMap,
    /// 原文上の範囲。
    pub span: Span,
    /// 引用 (`>`) の内側か。
    pub in_quote: bool,
    /// 脚注定義の内側か。
    pub in_footnote: bool,
    /// 原文の改行 (ソフト改行・ハード改行) があった解析用テキスト上の位置。
    pub line_breaks: Vec<usize>,
    /// 原文の改行のうち、書式から文の区切りと分かるものの位置 (`line_breaks` の一部)。
    /// 1 行 1 項目で書いた箇条書きやラベルの行の後の改行で、改行の扱いの設定によらず文を切る。
    pub sentence_breaks: Vec<usize>,
    /// インライン装飾。
    pub marks: Vec<InlineMark>,
    /// このブロックに属する文 ([`Document::sentences`] の添字範囲)。
    pub sentences: Range<usize>,
}

impl Block {
    /// 地の文 (統計系ルールの対象) か。
    ///
    /// 校正は「見出し・リスト・引用・表・コードを除いた地の文」で行われたため、
    /// 統計量はこの範囲だけで計算する。
    pub fn is_prose(&self) -> bool {
        self.kind == BlockKind::Paragraph && !self.in_quote && !self.in_footnote
    }

    pub fn is_heading(&self) -> bool {
        matches!(self.kind, BlockKind::Heading(_))
    }

    /// 解析用テキスト上の範囲を原文上の範囲に変換する。
    pub fn to_source(&self, range: Range<usize>) -> Span {
        self.map.to_source(range)
    }
}

/// 文。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sentence {
    /// 属するブロックの添字。
    pub block: usize,
    /// ブロックの解析用テキスト上の範囲 (前後の空白を除く、文末記号を含む)。
    pub range: Range<usize>,
    /// 原文上の範囲。
    pub span: Span,
    /// 読み手が読む文字数の近似 ([`text::reading_length`])。
    pub length: usize,
    /// 日本語の文字を含むか。含まない文 (英文・記号だけ) は統計から外す。
    pub japanese: bool,
    /// 括弧の内側に文末記号があり、括弧ごと 1 文として扱ったか。
    pub embedded_enders: bool,
}

/// 抑制コメントの種類。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirectiveKind {
    /// `noslop-disable-next-line`: コメントの次の物理行。
    DisableNextLine,
    /// `noslop-disable-line`: コメントのある行。
    DisableLine,
    /// `noslop-disable`: 対応する `noslop-enable` まで (なければ文書末まで)。
    Disable,
    /// `noslop-enable`: `noslop-disable` の範囲を閉じる。
    Enable,
    /// `noslop-disable-file`: 文書全体。
    DisableFile,
}

/// 抑制コメント (`<!-- noslop-disable-next-line P01 -- 理由 -->`)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Directive {
    pub kind: DirectiveKind,
    /// 対象ルール (ID か名前)。空なら全ルール。
    pub rules: Vec<String>,
    /// `--` 以降に書かれた理由。
    pub reason: Option<String>,
    /// コメント全体の原文上の範囲。
    pub span: Span,
}

/// 行番号と列番号の計算。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineIndex {
    line_starts: Vec<usize>,
}

impl LineIndex {
    pub fn new(source: &str) -> Self {
        let mut line_starts = vec![0];
        for (i, b) in source.bytes().enumerate() {
            if b == b'\n' {
                line_starts.push(i + 1);
            }
        }
        Self { line_starts }
    }

    /// 行数。
    pub fn len(&self) -> usize {
        self.line_starts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.line_starts.is_empty()
    }

    /// バイト位置の行番号 (1 始まり)。
    pub fn line(&self, offset: usize) -> usize {
        self.line_starts.partition_point(|&s| s <= offset)
    }

    /// バイト位置の (行, 列)。どちらも 1 始まりで、列は Unicode スカラー値の個数で数える。
    pub fn line_col(&self, source: &str, offset: usize) -> (usize, usize) {
        let offset = floor_char_boundary(source, offset.min(source.len()));
        let line = self.line(offset);
        let start = self.line_starts[line - 1];
        let col = source[start..offset].chars().filter(|&c| c != '\r').count() + 1;
        (line, col)
    }

    /// 行 (1 始まり) の原文上の範囲 (改行を含まない)。
    pub fn line_span(&self, source: &str, line: usize) -> Span {
        let start = self.line_starts[(line - 1).min(self.line_starts.len() - 1)];
        let end = self
            .line_starts
            .get(line)
            .map_or(source.len(), |&next| next - 1);
        let end = if end > start && source.as_bytes().get(end - 1) == Some(&b'\r') {
            end - 1
        } else {
            end
        };
        Span::new(start, end.max(start))
    }
}

fn floor_char_boundary(s: &str, mut i: usize) -> usize {
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// 解析対象の文書。
#[derive(Debug, Clone)]
pub struct Document {
    /// 表示用の名前 (パス。標準入力なら `<stdin>` か `--stdin-filename`)。
    pub name: String,
    pub format: SourceFormat,
    /// 原文 (先頭の BOM は除く)。
    ///
    /// 診断の [`Span`] はこの文字列上のバイト位置で、ファイル上のバイト位置は
    /// [`Document::file_offset`] で求める。
    pub source: String,
    /// 読み込むときに取り除いた先頭の BOM のバイト数 (なければ 0)。
    pub bom_len: usize,
    pub lines: LineIndex,
    pub blocks: Vec<Block>,
    pub sentences: Vec<Sentence>,
    pub directives: Vec<Directive>,
}

impl Document {
    /// 原文を読み込んでブロックと文に分ける。
    pub fn parse(
        name: impl Into<String>,
        source: impl Into<String>,
        format: SourceFormat,
        options: &ParseOptions,
    ) -> Self {
        let mut source: String = source.into();
        let bom_len = if source.starts_with('\u{FEFF}') {
            source.drain(..'\u{FEFF}'.len_utf8());
            '\u{FEFF}'.len_utf8()
        } else {
            0
        };
        let (mut blocks, directives) = match format {
            SourceFormat::Markdown => crate::markdown::parse(&source),
            SourceFormat::PlainText => crate::plaintext::parse(&source),
        };
        let mut sentences = Vec::new();
        for (idx, block) in blocks.iter_mut().enumerate() {
            let begin = sentences.len();
            for piece in segment::split(
                &block.text,
                &block.line_breaks,
                &block.sentence_breaks,
                options.line_breaks,
            ) {
                let span = block.to_source(piece.range.clone());
                let body = &block.text[piece.range.clone()];
                sentences.push(Sentence {
                    block: idx,
                    length: text::reading_length(body),
                    japanese: text::contains_japanese(body),
                    embedded_enders: piece.embedded_enders,
                    range: piece.range,
                    span,
                });
            }
            block.sentences = begin..sentences.len();
        }
        let lines = LineIndex::new(&source);
        Self {
            name: name.into(),
            format,
            source,
            bom_len,
            lines,
            blocks,
            sentences,
            directives,
        }
    }

    /// Markdown として読み込む (テスト・ライブラリ利用向けの近道)。
    pub fn markdown(source: impl Into<String>) -> Self {
        Self::parse(
            "<input>.md",
            source,
            SourceFormat::Markdown,
            &ParseOptions::default(),
        )
    }

    /// テキストとして読み込む (テスト・ライブラリ利用向けの近道)。
    pub fn plain_text(source: impl Into<String>) -> Self {
        Self::parse(
            "<input>.txt",
            source,
            SourceFormat::PlainText,
            &ParseOptions::default(),
        )
    }

    /// 文の解析用テキスト。
    pub fn sentence_text(&self, sentence: &Sentence) -> &str {
        &self.blocks[sentence.block].text[sentence.range.clone()]
    }

    /// ブロックに属する文。
    pub fn block_sentences(&self, block: usize) -> &[Sentence] {
        &self.sentences[self.blocks[block].sentences.clone()]
    }

    /// 地の文にあり、日本語を含む文 (統計系ルールの母集団)。
    pub fn prose_sentences(&self) -> impl Iterator<Item = &Sentence> {
        self.sentences
            .iter()
            .filter(|s| s.japanese && self.blocks[s.block].is_prose())
    }

    /// 原文の文字数 (Markdown 記法・改行込み)。スコアの正規化に使う。
    pub fn char_count(&self) -> usize {
        self.source.chars().count()
    }

    /// 原文上の範囲の文字列。
    pub fn slice(&self, span: Span) -> &str {
        &self.source[span.range()]
    }

    /// バイト位置の (行, 列)。どちらも 1 始まり。
    ///
    /// BOM は画面に出ない文字なので、列は BOM を除いて数える。
    pub fn line_col(&self, offset: usize) -> (usize, usize) {
        self.lines.line_col(&self.source, offset)
    }

    /// [`Document::source`] 上のバイト位置を、ファイル上のバイト位置 (BOM を含む) に直す。
    pub fn file_offset(&self, offset: usize) -> usize {
        offset + self.bom_len
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_map_translates_exact_and_opaque_segments() {
        let mut map = TextMap::default();
        // 原文 "ab`x`cd" → 解析用 "ab\u{FFFC}cd"
        map.push_exact(0, 0, 2);
        map.push_opaque(2, 3, Span::new(2, 5));
        map.push_exact(5, 5, 2);
        assert_eq!(map.to_source(0..2), Span::new(0, 2));
        assert_eq!(map.to_source(2..5), Span::new(2, 5));
        assert_eq!(map.to_source(1..6), Span::new(1, 6));
        assert_eq!(map.to_source(5..7), Span::new(5, 7));
    }

    #[test]
    fn line_index_counts_chars_and_handles_crlf() {
        let src = "一行目\r\n二行目です\n三";
        let idx = LineIndex::new(src);
        assert_eq!(idx.len(), 3);
        let pos = src.find("です").unwrap();
        assert_eq!(idx.line_col(src, pos), (2, 4));
        assert_eq!(idx.line_col(src, 0), (1, 1));
        assert_eq!(idx.line_span(src, 1), Span::new(0, "一行目".len()));
        let third = src.find('三').unwrap();
        assert_eq!(idx.line(third), 3);
    }

    #[test]
    fn plain_text_documents_are_split_into_prose_sentences() {
        let doc = Document::plain_text(
            "第一章\n\u{3000}最初の文です。「会話。」と言った。\n\u{3000}次の段落です。\n",
        );
        let kinds: Vec<_> = doc.blocks.iter().map(|b| b.kind).collect();
        assert_eq!(
            kinds,
            vec![
                BlockKind::Heading(0),
                BlockKind::Paragraph,
                BlockKind::Paragraph
            ]
        );
        let prose: Vec<_> = doc
            .prose_sentences()
            .map(|s| doc.sentence_text(s))
            .collect();
        assert_eq!(
            prose,
            vec!["最初の文です。", "「会話。」と言った。", "次の段落です。"]
        );
        let quoted = doc.prose_sentences().nth(1).unwrap();
        assert!(quoted.embedded_enders);
        assert_eq!(doc.slice(quoted.span), "「会話。」と言った。");
    }

    #[test]
    fn format_from_extension() {
        assert_eq!(
            SourceFormat::from_path(Path::new("a/b.MD")),
            SourceFormat::Markdown
        );
        assert_eq!(
            SourceFormat::from_path(Path::new("note.txt")),
            SourceFormat::PlainText
        );
    }
}
