//! 解析対象の文書モデル。
//!
//! Markdown / テキストを「ブロック (段落・リスト項目・見出し・表セル)」の列にし、
//! 各ブロックの解析用テキストを文に分割したものを持つ。解析用テキストは原文から
//! 装飾・コード・URL を取り除いたもので、原文上の位置へは [`TextMap`] で戻す。

use std::ops::Range;
use std::path::Path;

use crate::code::CodeLanguage;
use crate::diagnostic::Span;
use crate::segment::{self, LineBreakMode};
use crate::text;

/// 入力の形式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceFormat {
    Markdown,
    PlainText,
    /// コードのファイル。コメントと静的な文言を読む ([`crate::code`])。
    Code(CodeLanguage),
}

impl SourceFormat {
    /// 拡張子から形式を推定する。Markdown 系の拡張子は Markdown、コードの拡張子はコード (コメントと静的な文言を
    /// 読む)、ほかはテキストとして扱う。
    pub fn from_path(path: &Path) -> Self {
        let Some(ext) = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
        else {
            return SourceFormat::PlainText;
        };
        match ext.as_str() {
            "md" | "markdown" | "mdown" | "mkd" | "mdx" => SourceFormat::Markdown,
            _ => CodeLanguage::from_extension(&ext)
                .map_or(SourceFormat::PlainText, SourceFormat::Code),
        }
    }

    /// この形式の文書の組み立て。コードはコメントの断片の集まり、ほかはひと続きの文章。
    pub fn kind(self) -> DocumentKind {
        match self {
            SourceFormat::Markdown | SourceFormat::PlainText => DocumentKind::Prose,
            SourceFormat::Code(_) => DocumentKind::Fragments,
        }
    }
}

/// 文書の組み立て。文書や段落をまたいで数えるルールを当てられるかを決める。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum DocumentKind {
    /// ひと続きの文章 (Markdown・テキスト・Google ドキュメントの本文など)。すべてのルールを当てる。
    #[default]
    Prose,
    /// 互いに独立した短い断片の集まり (コードのコメント、表計算のセルなど)。1 文ずつ判定するルール
    /// ([`RuleUnit::Sentence`](crate::rules::RuleUnit)) だけを当てる。断片をまとめて数えても、
    /// 文書の統計の母数にも、ルールを校正した条件 (ひと続きの地の文) にもならないため。
    Fragments,
}

/// 文書の読み込み方の設定。
#[derive(Debug, Clone, Copy)]
pub struct ParseOptions {
    pub line_breaks: LineBreakMode,
    /// コード内の静的な日本語の文字列と表示文言を読む。
    pub static_text: bool,
}

impl Default for ParseOptions {
    fn default() -> Self {
        Self {
            line_breaks: LineBreakMode::default(),
            static_text: true,
        }
    }
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

    /// この対応表 (解析用テキスト → 中間のテキスト) の先に、中間のテキストから原文への対応表
    /// `outer` をつないだ対応表 (解析用テキスト → 原文) を返す。
    ///
    /// コードのコメントのように、原文から記号を外して並べた中間のテキストを読み直すときに使う
    /// ([`crate::code`])。1:1 に写した区間は、`outer` の区間が切り替わる位置 (中間のテキストで行が
    /// 変わる位置など) で分け、`outer` でも 1:1 なら 1:1 のまま写す。ほかの区間は、中間のテキスト上の
    /// 範囲を `outer` で原文に戻した範囲全体に対応させる。
    pub fn compose(&self, outer: &TextMap) -> TextMap {
        let mut out = TextMap::default();
        for seg in &self.segments {
            if !seg.exact {
                let src = outer.to_source(seg.src_start..seg.src_end);
                out.push_opaque(seg.text_start, seg.text_end - seg.text_start, src);
                continue;
            }
            // 中間のテキスト上の位置 (1:1 なので解析用テキストとの差は一定)
            let mut pos = seg.src_start;
            while pos < seg.src_end {
                let text_at = seg.text_start + (pos - seg.src_start);
                let idx = outer.segments.partition_point(|o| o.text_end <= pos);
                let next = outer.segments.get(idx);
                match next {
                    Some(o) if o.text_start <= pos => {
                        let end = o.text_end.min(seg.src_end);
                        if o.exact {
                            out.push_exact(text_at, o.src_start + (pos - o.text_start), end - pos);
                        } else {
                            out.push_opaque(text_at, end - pos, Span::new(o.src_start, o.src_end));
                        }
                        pos = end;
                    }
                    // `outer` の区間の隙間 (どの区間にも入らない位置) は、次の区間の頭までをまとめて戻す
                    _ => {
                        let end = next.map_or(seg.src_end, |o| o.text_start.min(seg.src_end));
                        out.push_opaque(text_at, end - pos, outer.to_source(pos..end));
                        pos = end;
                    }
                }
            }
        }
        out
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
    /// 行をまるごと占める「ラベル: 値」の行か (「**報告日**: 2026年10月1日」のような項目の行)。
    /// 箇条書きの代わりに改行で並べた項目で、地の文の流れにはない。条件は文分割のときに判定する
    /// 関数 (`is_label_value`) の説明にある。
    pub label_value: bool,
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
    /// 文書の組み立て (ひと続きの文章か、断片の集まりか)。当てるルールを決める。
    pub kind: DocumentKind,
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
            SourceFormat::Code(language) => {
                crate::code::parse_with_options(&source, language, options)
            }
        };
        let sentences = split_sentences(&mut blocks, options);
        let lines = LineIndex::new(&source);
        Self {
            name: name.into(),
            format,
            kind: format.kind(),
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
    ///
    /// 改行で並べた「ラベル: 値」の行 ([`Sentence::label_value`]) は除く。箇条書きの代わりの項目で、
    /// 文の長さ・体言止め・文頭の統計に地の文として入れると、本文の調子と関係なく値が動くため。
    pub fn prose_sentences(&self) -> impl Iterator<Item = &Sentence> {
        self.sentences
            .iter()
            .filter(|s| s.japanese && !s.label_value && self.blocks[s.block].is_prose())
    }

    /// 文書の結びに当たる、日本語を含む地の文の段落の最後の `count` 個 (ブロックの添字、文書順)。
    ///
    /// 後ろに続く見出し・箇条書き・表・コードは数えず、その手前の段落までさかのぼる
    /// (「まとめ」の箇条書きで終わる文書の、結論の段落を拾うため)。段落が `count` 個に満たない
    /// 文書では、ある段落をすべて返す。
    pub fn closing_paragraphs(&self, count: usize) -> Vec<usize> {
        let mut closing: Vec<usize> = self
            .blocks
            .iter()
            .enumerate()
            .rev()
            .filter(|(i, b)| b.is_prose() && self.block_sentences(*i).iter().any(|s| s.japanese))
            .map(|(i, _)| i)
            .take(count)
            .collect();
        closing.reverse();
        closing
    }

    /// 原文の文字数 (Markdown 記法・改行込み)。JSON の `characters` に使う。
    pub fn char_count(&self) -> usize {
        self.source.chars().count()
    }

    /// 本文の文字数 (Markdown 記法・改行込み。文書の先頭の front matter は数えない)。
    ///
    /// 構造系のルールが 1000 字あたりの密度を測る母数と、`noslop calibrate` の 1000 字あたりの
    /// 件数に使う。front matter (題名・タグ・日付などのメタデータ) は読み手が読む本文ではないので、
    /// 含めると front matter の長い文書ほど密度が低く出る。
    pub fn body_char_count(&self) -> usize {
        let front_matter = match self.format {
            SourceFormat::Markdown => crate::markdown::front_matter_len(&self.source),
            SourceFormat::PlainText | SourceFormat::Code(_) => 0,
        };
        self.source[front_matter..].chars().count()
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

/// ブロックを文に分け、原文上の位置と文の属性を求める。
/// 各ブロックの `sentences` は、返す文の列を参照する範囲に更新する。
pub(crate) fn split_sentences(blocks: &mut [Block], options: &ParseOptions) -> Vec<Sentence> {
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
                label_value: is_label_value(block, &piece.range),
                range: piece.range,
                span,
            });
        }
        block.sentences = begin..sentences.len();
    }
    sentences
}

/// ラベルとみなす字数の上限 (コロンの前の字数)。
const MAX_LABEL_CHARS: usize = 20;

/// ブロックの解析用テキストの `range` の文が、行をまるごと占める「ラベル: 値」の行か。
///
/// 「**報告日**: 2026年10月1日」「**作成者：** 総務部」「出典：〜調査」のように、短いラベルと
/// コロンに値が続く行を、改行で並べた項目とみなす。次をすべて満たす文が当たる。
///
/// - 行の頭から始まり、行の終わりで終わる (同じ行に別の文がない)
/// - 文末記号で終わらない (値が文になった「**メリット**: 〜がいい。」は地の文として扱う)
/// - 文に含まれる行がどれも、ラベルとコロンに値が続く形 ([`labeled_line`])。ソフト改行で
///   つながった複数の行は、すべての行が項目のときだけ項目の並びとみなす (「担当：総務部」に
///   改行で続けた本文の行まで外さないため)
fn is_label_value(block: &Block, range: &Range<usize>) -> bool {
    let text = &block.text;
    let Some(body) = text.get(range.clone()) else {
        return false;
    };
    if body
        .trim_end()
        .chars()
        .next_back()
        .is_none_or(text::is_sentence_ender)
    {
        return false;
    }
    // 文の中の行の境目 (文の頭と終わりを含む)
    let mut bounds: Vec<usize> = block
        .line_breaks
        .iter()
        .copied()
        .filter(|&at| range.start < at && at < range.end)
        .collect();
    bounds.sort_unstable();
    bounds.dedup();
    bounds.insert(0, range.start);
    bounds.push(range.end);
    // 先頭の行から見る (コロンのない大半の文は、改行の位置を調べずに済む)
    if !labeled_line(block, bounds[0]..bounds[1]) {
        return false;
    }
    let line_start = block
        .line_breaks
        .iter()
        .copied()
        .filter(|&at| at <= range.start)
        .max()
        .unwrap_or(0);
    let line_end = block
        .line_breaks
        .iter()
        .copied()
        .filter(|&at| at >= range.end)
        .min()
        .unwrap_or(text.len());
    let blank = |r: Range<usize>| text.get(r).is_some_and(|s| s.trim().is_empty());
    blank(line_start..range.start)
        && blank(range.end..line_end)
        && bounds
            .windows(2)
            .skip(1)
            .all(|w| labeled_line(block, w[0]..w[1]))
}

/// ブロックの解析用テキストの `range` の 1 行 (前後の空白は除いて見る) が、ラベルとコロンに値が
/// 続く形か。
///
/// ラベルは 1〜20 字で、空白・句読点を含まず、日本語か英字を含み、括弧が閉じている (「全社（対象：
/// 250名）で〜」のような括弧の中のコロンを除く)。半角のコロンは、直後が空白か、ラベルが太字の中に
/// あるとき (「**報告日**:」「**報告日:**」「**作成部署:経営企画部**」) だけ区切りとみなす
/// (「id:cohalz」「10:00」「https://」のような語の中のコロンを除く)。
fn labeled_line(block: &Block, range: Range<usize>) -> bool {
    let Some(raw) = block.text.get(range.clone()) else {
        return false;
    };
    let line = raw.trim_start();
    let start = range.start + (raw.len() - line.len());
    let line = line.trim_end();
    let Some((at, colon)) = line
        .char_indices()
        .take(MAX_LABEL_CHARS + 1)
        .find(|&(_, c)| matches!(c, ':' | '：'))
    else {
        return false;
    };
    let label = &line[..at];
    let value = &line[at + colon.len_utf8()..];
    if label.is_empty()
        || value.trim().is_empty()
        || label
            .chars()
            .any(|c| c.is_whitespace() || text::is_comma(c) || matches!(c, '。' | '．' | ','))
        || !label
            .chars()
            .any(|c| text::is_japanese(c) || c.is_ascii_alphabetic())
        || !brackets_closed(label)
    {
        return false;
    }
    if colon == ':' && !value.starts_with(char::is_whitespace) {
        // ラベルの頭からコロンまでを太字が覆うときだけ区切りとみなす
        let at = start + at;
        return block
            .marks
            .iter()
            .any(|m| m.kind == MarkKind::Strong && m.range.start <= start && m.range.end >= at);
    }
    true
}

/// 開き括弧がすべて、対応する閉じ括弧で閉じているか (括弧の字の集合は文分割と同じ)。
fn brackets_closed(s: &str) -> bool {
    let mut open: Vec<char> = Vec::new();
    for c in s.chars() {
        // `〝` は `〟` と `〞` のどちらでも閉じる
        let c = if c == '〞' { '〟' } else { c };
        if let Some(close) = text::closing_bracket(c) {
            open.push(close);
        } else if text::is_closing_bracket(c) && open.pop() != Some(c) {
            return false;
        }
    }
    open.is_empty()
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
    fn composed_maps_split_exact_segments_at_the_outer_boundaries() {
        // 原文 "// 一行目\n// 二行目" → 中間 "一行目\n二行目" (改行は原文の "\n// " に対応)
        let src = "// 一行目\n// 二行目";
        let second = src.rfind("二行目").unwrap();
        let mut outer = TextMap::default();
        outer.push_exact(0, 3, "一行目".len());
        let nl = "一行目".len();
        outer.push_opaque(nl, 1, Span::new(3 + nl, second));
        outer.push_exact(nl + 1, second, "二行目".len());
        // 中間のテキストを 1:1 に写した区間 (行をまたぐ) と、置き換えた 1 字
        let mut inner = TextMap::default();
        inner.push_exact(0, 0, nl + 1 + "二行".len());
        let at = nl + 1 + "二行".len();
        inner.push_opaque(at, 1, Span::new(at, at + "目".len()));
        let map = inner.compose(&outer);
        let slice = |r: Range<usize>| &src[map.to_source(r).range()];
        assert_eq!(slice(0..nl), "一行目");
        assert_eq!(slice(3..nl + 1 + 3), "行目\n// 二");
        assert_eq!(slice(nl + 1..nl + 1 + "二行".len()), "二行");
        assert_eq!(slice(at..at + 1), "目");
        // 中間のテキストの改行 ("\n") は、原文で行が切り替わる部分全体に戻る
        assert_eq!(slice(nl..nl + 1), "\n// ");
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
    fn body_char_count_skips_only_the_leading_front_matter() {
        let md = Document::markdown("---\ntitle: 例\n---\n本文です。\n");
        assert_eq!(md.char_count(), 23);
        assert_eq!(md.body_char_count(), 6);
        // 文書の途中の区切り線や、テキストの先頭の `---` は本文として数える
        let middle = Document::markdown("本文です。\n\n---\nほか\n---\n");
        assert_eq!(middle.body_char_count(), middle.char_count());
        let text = Document::plain_text("---\ntitle: 例\n---\n本文です。\n");
        assert_eq!(text.body_char_count(), text.char_count());
    }

    #[test]
    fn closing_paragraphs_are_the_last_prose_paragraphs() {
        let doc = Document::markdown(
            "# 題\n\n最初の段落。\n\n二つ目の段落。\n\n三つ目の段落。\n\n## まとめ\n\n- 項目\n- 項目\n",
        );
        let texts: Vec<&str> = doc
            .closing_paragraphs(2)
            .iter()
            .map(|&i| doc.blocks[i].text.as_str())
            .collect();
        assert_eq!(texts, ["二つ目の段落。", "三つ目の段落。"]);
        // 段落が足りなければ、ある段落だけ。日本語を含まない段落・引用は数えない
        let doc = Document::markdown("> 引用の段落。\n\nEnglish only.\n\n唯一の段落。\n");
        let texts: Vec<&str> = doc
            .closing_paragraphs(2)
            .iter()
            .map(|&i| doc.blocks[i].text.as_str())
            .collect();
        assert_eq!(texts, ["唯一の段落。"]);
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
    fn sentence_ranges_and_attributes_stay_aligned_across_empty_blocks() {
        let source = "**一行目**で\n折り返す。\n\n「はい。」と答える。OK!\n";
        let (mut blocks, _) = crate::markdown::parse(source);
        assert_eq!(blocks.len(), 2);
        let mut empty = blocks[0].clone();
        empty.text.clear();
        empty.line_breaks.clear();
        empty.sentence_breaks.clear();
        blocks.insert(1, empty);

        for (mode, first_count) in [(LineBreakMode::Space, 1), (LineBreakMode::Sentence, 2)] {
            let sentences = split_sentences(
                &mut blocks,
                &ParseOptions {
                    line_breaks: mode,
                    ..ParseOptions::default()
                },
            );
            assert_eq!(blocks[0].sentences, 0..first_count);
            assert_eq!(blocks[1].sentences, first_count..first_count);
            assert_eq!(blocks[2].sentences, first_count..first_count + 2);
            assert_eq!(sentences.len(), first_count + 2);
            for (idx, block) in blocks.iter().enumerate() {
                for sentence in &sentences[block.sentences.clone()] {
                    assert_eq!(sentence.block, idx);
                }
            }
            let first = &sentences[0];
            assert_eq!(first.range.start, 0);
            assert_eq!(first.span.start, "**".len());
            assert_eq!(first.length, if first_count == 1 { 8 } else { 4 });
            assert!(first.japanese);
            assert!(!first.embedded_enders);

            let quoted = &sentences[first_count];
            assert_eq!(&source[quoted.span.range()], "「はい。」と答える。");
            assert_eq!(
                &blocks[2].text[quoted.range.clone()],
                "「はい。」と答える。"
            );
            assert!(quoted.japanese);
            assert!(quoted.embedded_enders);
            let english = &sentences[first_count + 1];
            assert_eq!(&source[english.span.range()], "OK!");
            assert_eq!(english.length, 2);
            assert!(!english.japanese);
            assert!(!english.embedded_enders);
        }
    }

    /// 文と、「ラベル: 値」の行か。
    fn label_values(src: &str) -> Vec<(String, bool)> {
        let doc = Document::markdown(src);
        doc.sentences
            .iter()
            .map(|s| (doc.sentence_text(s).to_string(), s.label_value))
            .collect()
    }

    fn all_label_values(src: &str) -> bool {
        let sentences = label_values(src);
        !sentences.is_empty() && sentences.iter().all(|(_, label)| *label)
    }

    #[test]
    fn label_value_lines_are_marked() {
        // 太字のラベルと全角のコロン (ソフト改行で並べた行)
        assert!(all_label_values(
            "**作成日：** 2026年10月1日\n**作成者：** 総務部 主任 佐藤 一郎\n**宛先：** 総務本部長\n"
        ));
        // 太字のラベルと半角のコロン (ハード改行。コロンが太字の外でも内でも、空白がなくても)
        let src = "**報告日**: 2026年10月1日  \n**作成者:** 総務部  \n**分類**:社内限定\n";
        assert_eq!(label_values(src).len(), 3);
        assert!(all_label_values(src));
        // 行全体を太字にした項目 (空白のない半角のコロンも、ラベルが太字の中にあれば区切り)
        let src = "**作成部署:経営企画部 業務改善推進課**\n**作成者:佐藤 一郎**\n";
        assert_eq!(label_values(src).len(), 2);
        assert!(all_label_values(src));
        // 太字でないラベル (全角のコロン、空白の続く半角のコロン)、括弧の閉じたラベル、記号で始まるラベル
        for src in [
            "出典：社内の調査\n",
            "メール: you@example.com\n",
            "特長（速さ）：待たずに済む\n",
            "・期限：10月末\n",
            "**第一段階：個別連絡**\n",
        ] {
            assert!(all_label_values(src), "{src}");
        }
        // ソフト改行でつながった半角のコロンの行は、どの行も項目なら全体を項目の並びとみなす
        let src = "**件名:** 研修の結果\n**報告日:** 2026年10月1日\n";
        assert_eq!(label_values(src).len(), 1);
        assert!(all_label_values(src));
    }

    #[test]
    fn prose_sentences_skip_label_value_lines() {
        let doc = Document::markdown(
            "**報告日**: 2026年10月1日  \n**宛先**: 総務本部長\n\n申請の手順を見直した。窓口は一本化する。\n",
        );
        assert_eq!(doc.sentences.len(), 4);
        let prose: Vec<_> = doc
            .prose_sentences()
            .map(|s| doc.sentence_text(s))
            .collect();
        assert_eq!(prose, ["申請の手順を見直した。", "窓口は一本化する。"]);
    }

    #[test]
    fn joined_lines_are_label_values_only_when_every_line_is_one() {
        // 項目の行に、改行で本文の行が続いて 1 文につながったものは、本文を含むので項目ではない
        let src = "担当：総務部\n今月は問い合わせ件数が増加\n対応負荷も上昇\n";
        assert_eq!(
            label_values(src),
            [(
                "担当：総務部今月は問い合わせ件数が増加対応負荷も上昇".to_string(),
                false
            )]
        );
        // 2 行目が語の中の半角コロンでも同じ
        let src = "**件名:** 研修の結果\n担当はid:cohalzさん\n";
        assert!(label_values(src).iter().all(|(_, label)| !label));
    }

    #[test]
    fn sentences_with_a_colon_are_not_always_label_values() {
        for src in [
            // 値が文になっている (文末記号で終わる)
            "**メリット**: 実装が単純で扱いやすい。\n",
            "注：中国は香港を含む。\n",
            // 括弧の中のコロン、語の中の半角コロン、時刻、数字だけのラベル
            "全社（対象：250名）で集計した結果\n",
            "担当はid:cohalzさん\n",
            "開場は10:00から\n",
            "10：00 開場\n",
            // 長すぎるラベル、値のないラベル、読点を含むラベル
            "これは二十字を超えるとても長い見出しのような書き出しです：値\n",
            "**添付資料：**\n",
            "結果、残った課題：夜間の対応\n",
        ] {
            assert!(
                label_values(src).iter().all(|(_, label)| !label),
                "{src}: {:?}",
                label_values(src)
            );
        }
        // 行の途中から始まる文は、ラベルのように書き出していても項目の行ではない
        assert_eq!(
            label_values("設定を保存した。注意：再起動が必要\n"),
            [
                ("設定を保存した。".to_string(), false),
                ("注意：再起動が必要".to_string(), false)
            ]
        );
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
