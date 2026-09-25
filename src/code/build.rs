//! コメントのまとまりから文書のブロックを組み立てる。
//!
//! 本文の行を改行でつないだ中間のテキストを作り、読み方ごとにブロックに組む。
//!
//! - 普通のコメント: 空いた行で段落を分け、Markdown の箇条書きの記号 (`-`・`*`・`+`・`1.`) で
//!   始まる行を箇条書きの項目にする (Markdown の文書と同じく、語句ルールは既定で段落だけを見る)。
//!   インラインコード (`` `x` ``) はプレースホルダ 1 字に畳み、`=====` のような飾りだけの行は
//!   空いた行とみなす。行の間の改行は、書式から文の区切りと分かるとき (文末記号で終わる、Markdown の
//!   文書と同じ判定、`@param` のようなタグで始まる行) だけ文を切る
//! - 文書のコメント・docstring: Markdown として読む (コードブロック・インラインコード・HTML は
//!   Markdown の文書と同じく解析から外れる。HTML コメントの抑制も読む)
//! - XML・HTML で書く文書のコメント: タグを外し、段落の単位のタグ (`<p>`・`<summary>`・`<param>`
//!   など) で段落を切る。コードの要素 (`<pre>`・`<code>`・`<c>`・`{@code}`) と参照 (`{@link}`・
//!   `<see/>`) はプレースホルダに畳むか、段落の切れ目にする
//!
//! ブロックの位置は中間のテキスト上で組み、最後に原文上の位置へ写し直す ([`TextMap::compose`])。

use std::ops::Range;
use std::sync::LazyLock;

use regex::Regex;

use super::body::Group;
use super::extract::Reading;
use crate::diagnostic::Span;
use crate::document::{Block, BlockKind, Directive, InlineMark, MarkKind, TextMap};
use crate::text::{self, PLACEHOLDER};

/// Markdown の箇条書きの記号 (`-`・`*`・`+`・`1.`) で始まる行。
static LIST_ITEM: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?:[-*+]|\d{1,9}[.)])(?:\s|$)").expect("list item regex"));

/// 文書のコメントのタグ (`@param`・`\brief`・reST の `:param x:`) で始まる行。
static TAG_LINE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:[@\\][A-Za-z]|:[A-Za-z][^:\n]*:(?:\s|$))").expect("tag line regex")
});

/// XML・HTML のタグ。
static MARKUP_TAG: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"</?([A-Za-z][A-Za-z0-9]*)(?:\s[^<>]*)?/?>").expect("markup tag regex")
});

/// 中身ごとコードとして扱う XML・HTML 要素の開きタグ。
static CODE_ELEMENT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)<(pre|code|c)(?:\s[^<>]*)?>").expect("code element regex"));

/// Javadoc のインラインのタグ (`{@code ...}`・`{@link ...}`) の始まり。
static INLINE_TAG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\{@([A-Za-z]+)").expect("inline tag regex"));

/// 段落の単位の XML・HTML のタグ (外して、そこで段落を切る)。C# の XML ドキュメントコメントと
/// Javadoc の HTML の要素。
const BLOCK_TAGS: &[&str] = &[
    "p",
    "br",
    "hr",
    "li",
    "ul",
    "ol",
    "dl",
    "dt",
    "dd",
    "table",
    "tr",
    "td",
    "th",
    "thead",
    "tbody",
    "tfoot",
    "caption",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "blockquote",
    "div",
    "section",
    "summary",
    "remarks",
    "param",
    "typeparam",
    "returns",
    "value",
    "exception",
    "example",
    "para",
    "list",
    "listheader",
    "item",
    "term",
    "description",
    "permission",
    "include",
    "inheritdoc",
];

/// 語の代わりに置く参照のタグ (`<see cref="X"/>`)。自分で閉じるものだけをプレースホルダに畳む。
const REFERENCE_TAGS: &[&str] = &["see", "seealso", "paramref", "typeparamref"];

/// 本文の行を改行でつないだ中間のテキストと、その原文への対応表。
struct Virtual {
    text: String,
    map: TextMap,
}

impl Virtual {
    fn new(source: &str, lines: &[Span]) -> Self {
        let mut text = String::new();
        let mut map = TextMap::default();
        let mut prev_end = None;
        for line in lines {
            if let Some(end) = prev_end {
                // 改行は、原文の行の境目 (改行とコメントの記号・字下げ) 全体に対応させる
                let at = text.len();
                text.push('\n');
                map.push_opaque(at, 1, Span::new(end, line.start));
            }
            let at = text.len();
            text.push_str(&source[line.range()]);
            map.push_exact(at, line.start, line.end - line.start);
            prev_end = Some(line.end);
        }
        Self { text, map }
    }

    /// 中間のテキスト上で組んだブロックの位置を、原文上の位置に写し直す。
    fn to_source(&self, block: &mut Block) {
        block.map = block.map.compose(&self.map);
        block.span = self.map.to_source(block.span.range());
        for mark in &mut block.marks {
            mark.span = self.map.to_source(mark.span.range());
        }
    }
}

/// コメントのまとまりのブロックを `blocks` に、中の抑制コメント (Markdown で読むものの HTML
/// コメント) を `directives` に足す。日本語を含まないまとまりのブロックは作らない。
pub(super) fn build(
    source: &str,
    group: &Group,
    blocks: &mut Vec<Block>,
    directives: &mut Vec<Directive>,
) {
    let virt = Virtual::new(source, &group.lines);
    let japanese = text::contains_japanese(&virt.text);
    let mut built = match group.reading {
        Reading::Markdown => {
            // 日本語を含まない文書のコメントも、抑制コメントだけは読む
            if !japanese && !virt.text.contains("noslop-") {
                return;
            }
            let (built, found) = crate::markdown::parse(&virt.text);
            directives.extend(found.into_iter().map(|mut d| {
                d.span = virt.map.to_source(d.span.range());
                d
            }));
            if !japanese {
                return;
            }
            built
        }
        Reading::Plain if japanese => paragraphs(&virt.text, &code_spans(&virt.text)),
        Reading::Markup if japanese => paragraphs(&virt.text, &markup(&virt.text)),
        Reading::Plain | Reading::Markup => return,
    };
    for block in &mut built {
        if group.reading == Reading::Markdown {
            tag_line_breaks(block);
        }
        virt.to_source(block);
    }
    blocks.extend(built);
}

/// Markdown で読んだブロックで、タグ (`@param` など) で始まる行の前の改行を文の区切りにする。
fn tag_line_breaks(block: &mut Block) {
    let mut added = false;
    for &at in &block.line_breaks {
        if TAG_LINE.is_match(block.text[at..].trim_start()) && !block.sentence_breaks.contains(&at)
        {
            block.sentence_breaks.push(at);
            added = true;
        }
    }
    if added {
        block.sentence_breaks.sort_unstable();
    }
}

/// 中間のテキストのうち、取り除く・置き換える・段落を切る範囲。
#[derive(Debug, Clone, PartialEq, Eq)]
struct Special {
    range: Range<usize>,
    action: Action,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    /// 解析用テキストから外す (インラインのタグ)。
    Drop,
    /// プレースホルダ 1 字に畳む (インラインコード・参照)。
    Placeholder(MarkKind),
    /// 外して、そこで段落を切る (段落の単位のタグ・コードブロック)。
    Break,
}

/// インラインコード (同じ行で、同じ数の `` ` `` で囲んだ範囲)。
fn code_spans(v: &str) -> Vec<Special> {
    let bytes = v.as_bytes();
    let run_at = |i: usize| bytes[i..].iter().take_while(|&&b| b == b'`').count();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'`' {
            i += 1;
            continue;
        }
        let run = run_at(i);
        let mut j = i + run;
        let mut end = None;
        while j < bytes.len() && bytes[j] != b'\n' {
            if bytes[j] == b'`' {
                let close = run_at(j);
                if close == run {
                    end = Some(j + close);
                    break;
                }
                j += close;
            } else {
                j += 1;
            }
        }
        match end {
            Some(end) if end > i + 2 * run => {
                out.push(Special {
                    range: i..end,
                    action: Action::Placeholder(MarkKind::Code),
                });
                i = end;
            }
            _ => i += run,
        }
    }
    out
}

/// XML・HTML で書く文書のコメントから、タグ・コード要素・インラインのタグを拾う。
fn markup(v: &str) -> Vec<Special> {
    let lower = v.to_ascii_lowercase();
    let mut found = Vec::new();
    for caps in INLINE_TAG.captures_iter(v) {
        let whole = caps.get(0).expect("whole match");
        let Some(end) = closing_brace(v, whole.start()) else {
            continue;
        };
        let kind = match &caps[1] {
            "code" | "literal" => MarkKind::Code,
            _ => MarkKind::Link,
        };
        found.push(Special {
            range: whole.start()..end,
            action: Action::Placeholder(kind),
        });
    }
    for caps in CODE_ELEMENT.captures_iter(v) {
        let open = caps.get(0).expect("whole match");
        let name = caps[1].to_ascii_lowercase();
        // 閉じのタグがなければ、開きのタグだけを外す
        let range = find_close(&lower, open.end(), &format!("</{name}"))
            .map_or(open.range(), |end| open.start()..end);
        let action = if name == "pre" || v[range.clone()].contains('\n') {
            Action::Break
        } else {
            Action::Placeholder(MarkKind::Code)
        };
        found.push(Special { range, action });
    }
    for caps in MARKUP_TAG.captures_iter(v) {
        let whole = caps.get(0).expect("whole match");
        let name = caps[1].to_ascii_lowercase();
        let action = if whole.as_str().ends_with("/>") && REFERENCE_TAGS.contains(&name.as_str()) {
            Action::Placeholder(MarkKind::Link)
        } else if BLOCK_TAGS.contains(&name.as_str()) {
            Action::Break
        } else {
            Action::Drop
        };
        found.push(Special {
            range: whole.range(),
            action,
        });
    }
    // 始まりの順 (同じなら長い順) に並べ、前のものと重なるもの (コード要素の中のタグなど) を捨てる
    found.sort_by(|a, b| {
        (a.range.start, std::cmp::Reverse(a.range.end))
            .cmp(&(b.range.start, std::cmp::Reverse(b.range.end)))
    });
    let mut out: Vec<Special> = Vec::new();
    for special in found {
        if out
            .last()
            .is_some_and(|last| special.range.start < last.range.end)
        {
            continue;
        }
        out.push(special);
    }
    out
}

/// `open` の `{` に対応する `}` の直後の位置。
fn closing_brace(v: &str, open: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (i, c) in v[open..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(open + i + 1);
                }
            }
            _ => {}
        }
    }
    None
}

/// `from` より後ろの閉じのタグ (`</name>`。`lower` は小文字にしたテキスト、`pattern` は `</name`)
/// の直後の位置。
fn find_close(lower: &str, from: usize, pattern: &str) -> Option<usize> {
    let mut at = from;
    while let Some(i) = lower[at..].find(pattern) {
        let after = at + i + pattern.len();
        let rest = &lower[after..];
        let spaces = rest.len() - rest.trim_start().len();
        if rest[spaces..].starts_with('>') {
            return Some(after + spaces + 1);
        }
        at = after;
    }
    None
}

/// 1 行のうち、解析用テキストに入れる部分 (中間のテキスト上の範囲)。
#[derive(Debug, Clone, PartialEq, Eq)]
enum Piece {
    Text(Range<usize>),
    Placeholder(Range<usize>, MarkKind),
}

impl Piece {
    fn range(&self) -> &Range<usize> {
        match self {
            Piece::Text(r) | Piece::Placeholder(r, _) => r,
        }
    }
}

/// 論理行 (中間のテキストの行を、段落を切る範囲でさらに分けたもの)。
#[derive(Debug, Default)]
struct Line {
    pieces: Vec<Piece>,
    /// この行の後ろで段落を切るか。
    breaks_after: bool,
}

/// 中間のテキストを、取り除く・置き換える範囲を当てた論理行に分ける。
fn lines(v: &str, specials: &[Special]) -> Vec<Line> {
    fn push_text(v: &str, out: &mut Vec<Line>, range: Range<usize>) {
        let mut start = range.start;
        for (i, part) in v[range].split('\n').enumerate() {
            if i > 0 {
                out.push(Line::default());
            }
            let end = start + part.len();
            if end > start {
                let line = out.last_mut().expect("at least one line");
                line.pieces.push(Piece::Text(start..end));
            }
            start = end + 1;
        }
    }
    let mut out = vec![Line::default()];
    let mut pos = 0;
    for special in specials {
        push_text(v, &mut out, pos..special.range.start);
        let line = out.last_mut().expect("at least one line");
        match special.action {
            Action::Drop => {}
            Action::Placeholder(kind) => {
                line.pieces
                    .push(Piece::Placeholder(special.range.clone(), kind));
            }
            Action::Break => {
                line.breaks_after = true;
                out.push(Line::default());
            }
        }
        pos = special.range.end;
    }
    push_text(v, &mut out, pos..v.len());
    out
}

/// 飾りに使う字 (ASCII の記号と罫線)。
fn is_decoration(c: char) -> bool {
    c.is_ascii_punctuation()
        || matches!(c, '\u{2500}'..='\u{257F}' | '—' | '―' | '＝' | '－' | '＊')
}

/// 行頭 (`from_end` なら行末) の、同じ飾りの字が 3 つ以上続き空白で区切られた並び (空白を含む)
/// のバイト数。
fn decoration_run(s: &str, from_end: bool) -> usize {
    let mut chars: Box<dyn Iterator<Item = char>> = if from_end {
        Box::new(s.chars().rev())
    } else {
        Box::new(s.chars())
    };
    let Some(first) = chars.next().filter(|&c| is_decoration(c)) else {
        return 0;
    };
    let mut len = first.len_utf8();
    let mut count = 1;
    let mut spaces = 0;
    for c in chars {
        if spaces == 0 && c == first {
            len += c.len_utf8();
            count += 1;
        } else if c.is_whitespace() {
            spaces += c.len_utf8();
        } else {
            break;
        }
    }
    if count >= 3 && spaces > 0 {
        len + spaces
    } else {
        0
    }
}

/// 前後の空白と飾りを外した行。
struct Trimmed {
    pieces: Vec<Piece>,
    /// 解析用テキストに入る文字列 (プレースホルダを含む)。
    visible: String,
    /// 外した行頭の空白のバイト数 (箇条書きの項目が続く行かを決める)。
    indent: usize,
}

impl Trimmed {
    fn refresh(&mut self, v: &str) {
        self.pieces.retain(|p| !p.range().is_empty());
        self.visible = self
            .pieces
            .iter()
            .map(|p| match p {
                Piece::Text(r) => &v[r.clone()],
                Piece::Placeholder(..) => "\u{FFFC}",
            })
            .collect();
    }
}

/// 行の前後の空白と飾りを外す。空いた行・飾りだけの行なら `None`。
fn trim_line(v: &str, mut pieces: Vec<Piece>) -> Option<Trimmed> {
    const INDENT: [char; 3] = [' ', '\t', '\u{3000}'];
    let mut indent = 0;
    while let Some(Piece::Text(r)) = pieces.first_mut() {
        let spaces = v[r.clone()].len() - v[r.clone()].trim_start_matches(INDENT).len();
        indent += spaces;
        r.start += spaces;
        if r.start < r.end {
            break;
        }
        pieces.remove(0);
    }
    while let Some(Piece::Text(r)) = pieces.last_mut() {
        r.end = r.start + v[r.clone()].trim_end().len();
        if r.start < r.end {
            break;
        }
        pieces.pop();
    }
    let mut line = Trimmed {
        pieces,
        visible: String::new(),
        indent,
    };
    line.refresh(v);
    if line
        .visible
        .chars()
        .all(|c| is_decoration(c) || c.is_whitespace())
    {
        return None;
    }
    // 行頭と行末の飾りの並び (`===== 設定 =====`)
    if let Some(Piece::Text(r)) = line.pieces.first_mut() {
        r.start += decoration_run(&v[r.clone()], false);
    }
    if let Some(Piece::Text(r)) = line.pieces.last_mut() {
        r.end -= decoration_run(&v[r.clone()], true);
    }
    line.refresh(v);
    Some(line)
}

/// 論理行を段落と箇条書きの項目に組む。
///
/// 箇条書きの記号で始まる行から項目を始め、項目の記号より深く字下げした行を項目の続きにする。
/// 記号は解析用テキストに入れない (Markdown の文書と同じ)。
fn paragraphs(v: &str, specials: &[Special]) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut current: Option<Draft> = None;
    for line in lines(v, specials) {
        let breaks_after = line.breaks_after;
        let trimmed = trim_line(v, line.pieces);
        let Some(mut line) = trimmed else {
            blocks.extend(current.take().map(Draft::finish));
            continue;
        };
        if let Some(marker) = LIST_ITEM.find(&line.visible) {
            blocks.extend(current.take().map(Draft::finish));
            if let Some(Piece::Text(r)) = line.pieces.first_mut() {
                let rest = &v[(r.start + marker.end()).min(r.end)..r.end];
                r.start = r.end - rest.trim_start().len();
            }
            line.refresh(v);
            if line.pieces.is_empty() {
                continue;
            }
            current = Some(Draft::new(BlockKind::ListItem, line.indent));
        } else if current
            .as_ref()
            .is_some_and(|d| d.kind == BlockKind::ListItem && line.indent <= d.indent)
        {
            blocks.extend(current.take().map(Draft::finish));
        }
        current
            .get_or_insert_with(|| Draft::new(BlockKind::Paragraph, line.indent))
            .push(v, &line.pieces, line.visible);
        if breaks_after {
            blocks.extend(current.take().map(Draft::finish));
        }
    }
    blocks.extend(current.take().map(Draft::finish));
    blocks
}

/// 組み立て途中のブロック (位置は中間のテキスト上)。
struct Draft {
    kind: BlockKind,
    /// 最初の行の字下げ (箇条書きの項目が続く行かを決める)。
    indent: usize,
    text: String,
    map: TextMap,
    span: Option<Span>,
    line_breaks: Vec<usize>,
    sentence_breaks: Vec<usize>,
    marks: Vec<InlineMark>,
    /// 直前の行 (文の区切りの判定に使う)。
    last_line: String,
}

impl Draft {
    fn new(kind: BlockKind, indent: usize) -> Self {
        Self {
            kind,
            indent,
            text: String::new(),
            map: TextMap::default(),
            span: None,
            line_breaks: Vec::new(),
            sentence_breaks: Vec::new(),
            marks: Vec::new(),
            last_line: String::new(),
        }
    }

    fn push(&mut self, v: &str, pieces: &[Piece], visible: String) {
        let (Some(first), Some(last)) = (pieces.first(), pieces.last()) else {
            return;
        };
        let (start, end) = (first.range().start, last.range().end);
        if let Some(prev) = self.span {
            self.line_breaks.push(self.text.len());
            if breaks_sentence(&self.last_line, &visible) {
                self.sentence_breaks.push(self.text.len());
            }
            // 英数字どうしがつながらないよう、改行の代わりに空白を入れる
            if let (Some(p), Some(n)) = (self.text.chars().next_back(), visible.chars().next())
                && !(text::is_cjk_like(p) && text::is_cjk_like(n))
                && !p.is_whitespace()
                && !n.is_whitespace()
            {
                let at = self.text.len();
                self.text.push(' ');
                self.map.push_opaque(at, 1, Span::new(prev.end, start));
            }
        }
        for piece in pieces {
            let at = self.text.len();
            match piece {
                Piece::Text(r) => {
                    self.text.push_str(&v[r.clone()]);
                    self.map.push_exact(at, r.start, r.len());
                }
                Piece::Placeholder(r, kind) => {
                    let span = Span::new(r.start, r.end);
                    self.text.push(PLACEHOLDER);
                    self.map.push_opaque(at, PLACEHOLDER.len_utf8(), span);
                    self.marks.push(InlineMark {
                        kind: *kind,
                        range: at..self.text.len(),
                        span,
                    });
                }
            }
        }
        self.span = Some(Span::new(self.span.map_or(start, |s| s.start), end));
        self.last_line = visible;
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
            sentence_breaks: self.sentence_breaks,
            marks: self.marks,
            sentences: 0..0,
        }
    }
}

/// 文の途中でしか行を終えない字 (読点・助詞)。コメントの行がこれで終わるなら、次の行へ続く文。
const CONTINUES_TO_NEXT_LINE: &str = "、，,をにがはのへやてでとば";

/// 開き括弧。コメントの行がこれで終わるなら、次の行へ続く文。
const OPENING_BRACKETS: &str = "（(「『[［【〈《{｛";

/// 前の行の続きとして読む行頭の字 (閉じ括弧・読点・句点と、前の文への注記を始める丸括弧)。
const CONTINUES_FROM_PREVIOUS_LINE: &str = "、，,。．.）)」』]］】〉》}｝（(";

/// 隣り合うコメント行の間の改行が、文の区切りか。
///
/// コードのコメントは、句点を打たずに 1 行に 1 つのことを書くことが多い (「設定を読む」の次の行に
/// 「見つからなければ既定値を使う」)。つなぐと、別々の文が 1 つの長い文として数えられるので、改行は
/// 既定で文の区切りにする。つなぐのは、前の行が文の途中でしか終わらない字 (読点・助詞・開き括弧) で
/// 終わるか、次の行が前の行の続きとして読む字 (閉じ括弧・読点・句点・注記の丸括弧) で始まるか、英文を折り
/// 返したとき (前の行の終わりと次の行の始まりがどちらも英数字) だけ。ただし、Markdown の文書と同じ
/// 判定 (箇条書きの記号・ラベル・コロン・丁寧体の文末など) に当たるか、次の行がタグ (`@param`・
/// `\brief`・`:param x:`) で始まるなら、前の行の終わりによらず区切る。
fn breaks_sentence(prev: &str, next: &str) -> bool {
    if crate::markdown::lines_break_sentence(prev, next, false) || TAG_LINE.is_match(next) {
        return true;
    }
    let (Some(last), Some(first)) = (
        prev.trim_end().chars().next_back(),
        next.trim_start().chars().next(),
    ) else {
        return true;
    };
    let continues = CONTINUES_TO_NEXT_LINE.contains(last)
        || OPENING_BRACKETS.contains(last)
        || CONTINUES_FROM_PREVIOUS_LINE.contains(first)
        || (last.is_ascii_alphanumeric() && first.is_ascii_alphanumeric());
    !continues
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(v: &str, specials: &[Special]) -> Vec<(BlockKind, String)> {
        paragraphs(v, specials)
            .into_iter()
            .map(|b| (b.kind, b.text))
            .collect()
    }

    #[test]
    fn paragraphs_split_on_blank_and_decoration_lines() {
        let v = "一段落目の\n続き。\n\n二段落目。\n=====\n三段落目。";
        assert_eq!(
            texts(v, &[]),
            [
                (BlockKind::Paragraph, "一段落目の続き。".to_string()),
                (BlockKind::Paragraph, "二段落目。".to_string()),
                (BlockKind::Paragraph, "三段落目。".to_string()),
            ]
        );
    }

    #[test]
    fn markdown_style_bullets_become_list_items() {
        // 項目の記号より深く字下げした行は項目の続き、そうでない行は新しい段落
        let v = "手順:\n- 設定を読む\n  続きの行\n1. 保存する\n最後の文。";
        assert_eq!(
            texts(v, &[]),
            [
                (BlockKind::Paragraph, "手順:".to_string()),
                (BlockKind::ListItem, "設定を読む続きの行".to_string()),
                (BlockKind::ListItem, "保存する".to_string()),
                (BlockKind::Paragraph, "最後の文。".to_string()),
            ]
        );
        let b = &paragraphs(v, &[])[1];
        assert_eq!(
            &v[b.to_source(0..b.text.len()).range()],
            "設定を読む\n  続きの行"
        );
        // `-1` や `3.5` は記号ではない
        assert_eq!(texts("-1 を返す\n3.5 倍", &[])[0].0, BlockKind::Paragraph);
    }

    #[test]
    fn decoration_runs_around_titles_are_removed() {
        assert_eq!(
            texts("===== 設定の読み込み =====", &[]),
            [(BlockKind::Paragraph, "設定の読み込み".to_string())]
        );
        // 2 字の並びや、空白で区切らない並びは本文のまま
        assert_eq!(
            texts("-- 設定 --", &[]),
            [(BlockKind::Paragraph, "-- 設定 --".to_string())]
        );
    }

    #[test]
    fn line_breaks_become_sentence_breaks_unless_the_line_continues() {
        let heads = |v: &str| -> Vec<String> {
            let b = &paragraphs(v, &[])[0];
            b.sentence_breaks
                .iter()
                .map(|&at| b.text[at..].trim_start().chars().take(3).collect())
                .collect()
        };
        // 文末記号・丁寧体の文末・コロン・タグの行
        assert_eq!(heads("読む。\n書く\n"), ["書く"]);
        assert_eq!(heads("設定を読みます\n保存します"), ["保存し"]);
        assert_eq!(heads("引数:\n設定のパス"), ["設定の"]);
        assert_eq!(heads("設定を読む\n@param path パス"), ["@pa"]);
        // 句点のない 1 行 1 文のコメント (体言止め・常体の文末) も区切る
        assert_eq!(
            heads(
                "設定のディレクトリを使用\nパスが既定値と一致するかを確かめる\n見つからなければ空で返す"
            ),
            ["パスが", "見つか"]
        );
        // 読点・助詞・開き括弧で折り返した本文と、閉じ括弧・読点で始まる行はつなぐ
        assert!(heads("設定を読み、\n既定値と重ねた結果を\n返す").is_empty());
        assert!(heads("ユーザーの設定と\nプロジェクトの設定を重ねる").is_empty());
        assert!(heads("既定値 (\n設定ファイルがないとき) を使う").is_empty());
        assert!(heads("既定値を使う\n（設定ファイルがないとき）").is_empty());
        // 英文の折り返しはつなぐ
        assert!(heads("reads the config and\nmerges the defaults").is_empty());
    }

    #[test]
    fn inline_code_becomes_a_placeholder() {
        let v = "`Config::load` を呼ぶ。``a`b`` も畳む。`閉じない";
        let specials = code_spans(v);
        let b = &paragraphs(v, &specials)[0];
        assert_eq!(b.text, "\u{FFFC} を呼ぶ。\u{FFFC} も畳む。`閉じない");
        assert_eq!(b.marks.len(), 2);
        assert_eq!(&v[b.to_source(0..3).range()], "`Config::load`");
    }

    #[test]
    fn markup_tags_break_paragraphs_and_code_is_folded() {
        let v = "<summary>\n設定を<b>必ず</b>読む。\n</summary>\n<param name=\"path\">設定の {@code Path} を渡す。</param>\n<pre>\nlet x = 1;\n</pre>\n<see cref=\"Config\"/> を参照する。";
        let specials = markup(v);
        assert_eq!(
            texts(v, &specials),
            [
                (BlockKind::Paragraph, "設定を必ず読む。".to_string()),
                (BlockKind::Paragraph, "設定の \u{FFFC} を渡す。".to_string()),
                (BlockKind::Paragraph, "\u{FFFC} を参照する。".to_string()),
            ]
        );
        // 入れ子の波括弧と、閉じのタグの大文字小文字
        let v = "{@code Map<K, {V}>} と <CODE>x</CODE> と <c>y</c>";
        let specials = markup(v);
        assert_eq!(specials.len(), 3);
        assert!(
            specials
                .iter()
                .all(|s| s.action == Action::Placeholder(MarkKind::Code))
        );
    }
}
