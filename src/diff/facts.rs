//! 事実の変化: 数字・日付・英字の語・カタカナ語・固有名詞・括弧の語・URL・人や担当の消失と追加。
//!
//! 改稿で事実を落としていないか、原文にない事実 (とくに数字) や人・担当を足していないかを
//! 確かめるための手掛かりを取り出す。表記の揺れ (全角と半角、漢数字と算用数字、
//! 「ヶ月」と「か月」) は正規化してから比べる。

mod actors;

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
use std::sync::LazyLock;

use regex::Regex;
use serde::Serialize;

use crate::diagnostic::Span;
use crate::document::{Block, Document, MarkKind};
use crate::morph::{DocMorphology, Morphology};

/// 事実の種類。並び順は出力の順 (数字・日付を先に出す)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FactKind {
    Number,
    Date,
    Quoted,
    Latin,
    Katakana,
    Url,
    Proper,
    /// 主語や連絡・依頼の相手の位置にある、人・組織・担当の語 (担当者・管理者・私たちなど)。
    Actor,
}

impl FactKind {
    pub fn label_ja(self) -> &'static str {
        match self {
            FactKind::Number => "数値",
            FactKind::Date => "日付",
            FactKind::Quoted => "括弧の語",
            FactKind::Latin => "英字の語",
            FactKind::Katakana => "カタカナ語",
            FactKind::Url => "URL",
            FactKind::Proper => "固有名詞",
            FactKind::Actor => "人・組織・担当",
        }
    }

    /// 原文にないまま増えたときに、捏造の疑いとして強調する種類か。
    pub fn is_verifiable(self) -> bool {
        matches!(self, FactKind::Number | FactKind::Date)
    }
}

/// 文書から取り出した事実 1 件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fact {
    pub kind: FactKind,
    /// 原文での表記。
    pub text: String,
    /// 比べるための正規化したキー。
    pub key: String,
    /// 原文上の位置。
    pub span: Span,
}

/// 前後で数が変わった事実 1 種類。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactChange {
    pub kind: FactKind,
    pub key: String,
    /// 代表の表記 (消えたものは改稿前、増えたものは改稿後の最初の表記)。
    pub text: String,
    pub before_count: usize,
    pub after_count: usize,
    /// 出現位置 (消えたものは改稿前の文書、増えたものは改稿後の文書)。
    pub spans: Vec<Span>,
    /// 改稿前に 1 度も出ない数値・日付が増えた (原文にない数字の追加)。
    pub suspicious: bool,
}

/// 文書から取り出した事実と、照合だけに使う人・組織・担当の語。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExtractedFacts {
    /// 位置の順に並んだ事実。人・組織・担当の語は、主語や連絡・依頼の相手の位置にあるものだけ。
    pub facts: Vec<Fact>,
    /// 文書のどこか (原文と各ブロックの解析用テキスト) に出る人・組織・担当の語の照合キー。
    /// 位置を問わず、複合語や引用の中の出現も含む。相手の文書にこの語がなければ増減として出す。
    pub actors: BTreeSet<&'static str>,
}

/// 事実の変化。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FactChanges {
    /// 改稿で減った・消えた事実 (事実の欠落の疑い)。
    pub removed: Vec<FactChange>,
    /// 改稿で増えた事実。原文にない数値・日付を先に並べる。
    pub added: Vec<FactChange>,
}

static DATE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"(?P<y>[0-9０-９]{4})\s*[年/／\-－]\s*(?P<m>[0-9０-９]{1,2})\s*[月/／\-－]\s*(?P<d>[0-9０-９]{1,2})\s*日?",
        r"|(?P<y2>[0-9０-９]{4})\s*年\s*(?P<m2>[0-9０-９]{1,2})\s*月",
        r"|(?P<m3>[0-9０-９]{1,2})\s*月\s*(?P<d3>[0-9０-９]{1,2})\s*日",
    ))
    .expect("date regex")
});

/// 数字に続けて取り込む単位・助数詞 (長いものを先に並べる)。
const UNIT_PATTERN: &str = "万円|億円|兆円|年間|年代|か月|ヶ月|カ月|ヵ月|箇月|週間|日間|時間|分間|秒間|ページ|キロ|%|％|円|年|月|週|日|時|分|秒|歳|人|名|件|回|倍|割|個|本|社|台|点|字|文|行|章|節|頁|万|億|兆|kg|km|cm|mm|MB|GB|TB|KB|ms";

static NUMBER_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"[0-9０-９]+(?:[.,．，][0-9０-９]+)*(?:{UNIT_PATTERN})?"
    ))
    .expect("number regex")
});

/// 漢数字は助数詞が続くときだけ数値とみなす (「一部」「一方」「統一」を拾わないため)。
static KANJI_NUMBER_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"[〇一二三四五六七八九十百千万億]+(?:万円|億円|年間|か月|ヶ月|カ月|ヵ月|箇月|週間|日間|時間|分間|ページ|円|年|月|週|日|時|分|秒|歳|人|名|件|回|倍|割|個|本|社|台|点|字|文|行|章|節)",
    )
    .expect("kanji number regex")
});

static LATIN_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"[A-Za-z][A-Za-z0-9]*(?:[._+#\-][A-Za-z0-9]+)*[+#]*").expect("latin regex")
});

static KATAKANA_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[ァ-ヺー・]{4,}").expect("katakana regex"));

static QUOTED_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"「(?P<a>[^「」\n]{1,60})」|『(?P<b>[^『』\n]{1,60})』").expect("quoted regex")
});

static URL_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"https?://[^\s<>()（）「」『』\[\]]+").expect("url regex"));

/// 文書の全ブロック (見出し・リスト・表を含む) から事実を取り出す。位置の順に並ぶ。
pub fn extract(doc: &Document) -> ExtractedFacts {
    extract_with_morphology(doc, None)
}

/// 文字種による抽出に、辞書が固有名詞とした語を加える。辞書がなければ文字種だけで読む。
pub fn extract_with_morphology(doc: &Document, morphology: Option<&Morphology>) -> ExtractedFacts {
    let mut facts = Vec::new();
    let morph = morphology.map(|m| m.for_document(doc));
    for block in &doc.blocks {
        extract_block(doc, block, morph.as_ref(), &mut facts);
        actors::extract_block(doc, block, &mut facts);
    }
    facts.sort_by_key(|f| (f.span.start, f.kind));
    ExtractedFacts {
        facts,
        actors: actors::presence(doc),
    }
}

/// 種類が文脈で入れ替わらないよう、英字・長いカタカナの語は文字種による抽出に任せる。
fn is_surface_word(word: &str) -> bool {
    let whole = |re: &Regex| re.find(word).is_some_and(|m| m.range() == (0..word.len()));
    (word.len() >= 2 && whole(&LATIN_RE))
        || (whole(&KATAKANA_RE)
            && word.trim_matches('・').chars().count() >= 4
            && !word.chars().all(|c| c == 'ー'))
}

/// ブロックの解析用テキスト上の固有名詞の範囲。原文への対応は採用するときに取る。
fn proper_ranges(doc: &Document, block: &Block, morph: &DocMorphology<'_>) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    for index in block.sentences.clone() {
        let sentence = &doc.sentences[index];
        let Some(tokens) = morph.sentence(index) else {
            continue;
        };
        for token in tokens
            .iter()
            .filter(|t| t.pos == hasami::CoarsePos::ProperNoun)
        {
            let range =
                sentence.range.start + token.range.start..sentence.range.start + token.range.end;
            if !is_surface_word(&block.text[range.clone()]) {
                ranges.push(range);
            }
        }
    }
    ranges
}

fn extract_block(
    doc: &Document,
    block: &Block,
    morph: Option<&DocMorphology<'_>>,
    out: &mut Vec<Fact>,
) {
    let text = block.text.as_str();
    let proper = morph.map_or_else(Vec::new, |m| proper_ranges(doc, block, m));
    let mut taken: Vec<Range<usize>> = Vec::new();
    let push = |out: &mut Vec<Fact>, kind: FactKind, range: Range<usize>, key: String| {
        out.push(Fact {
            kind,
            text: text[range.clone()].to_string(),
            key,
            span: block.to_source(range),
        });
    };

    // URL (本文に書かれたもの)。英字の語として重ねて数えないよう範囲を控える
    for m in URL_RE.find_iter(text) {
        let url = trim_url(m.as_str());
        let range = m.start()..m.start() + url.len();
        push(out, FactKind::Url, range.clone(), url.to_string());
        taken.push(range);
    }
    // リンクの URL (解析用テキストには残らないので原文から読む)
    for mark in block.marks.iter().filter(|m| m.kind == MarkKind::Link) {
        let src = doc.slice(mark.span);
        for m in URL_RE.find_iter(src) {
            let url = trim_url(m.as_str());
            let start = mark.span.start + m.start();
            out.push(Fact {
                kind: FactKind::Url,
                text: url.to_string(),
                key: url.to_string(),
                span: Span::new(start, start + url.len()),
            });
        }
    }

    // 日付は数値より先に取り、日付の中の数字を数値として重ねて数えない
    let mut dates: Vec<Range<usize>> = Vec::new();
    for caps in DATE_RE.captures_iter(text) {
        let whole = caps.get(0).expect("group 0");
        if overlaps(&taken, &whole.range()) {
            continue;
        }
        let g = |name: &str| caps.name(name).map(|m| ascii_digits(m.as_str()));
        let key = if let (Some(y), Some(m), Some(d)) = (g("y"), g("m"), g("d")) {
            format!("{y}-{:0>2}-{:0>2}", m, d)
        } else if let (Some(y), Some(m)) = (g("y2"), g("m2")) {
            format!("{y}-{:0>2}", m)
        } else if let (Some(m), Some(d)) = (g("m3"), g("d3")) {
            format!("--{:0>2}-{:0>2}", m, d)
        } else {
            continue;
        };
        push(out, FactKind::Date, whole.range(), key);
        dates.push(whole.range());
    }
    taken.extend(dates);

    // 算用数字 (英字に接しているものは製品名・版番号の一部として英字の語に回す)
    for m in NUMBER_RE.find_iter(text) {
        let range = m.range();
        if overlaps(&taken, &range) || touches_ascii_letter(text, &range) {
            continue;
        }
        let (number, unit) = split_unit(m.as_str());
        let key = number_key(number, unit);
        push(out, FactKind::Number, range.clone(), key);
        taken.push(range);
    }
    // 漢数字 + 助数詞
    for m in KANJI_NUMBER_RE.find_iter(text) {
        let range = m.range();
        // 「四日市」「二本松」のような固有名詞の一部を漢数字の数値にしない。
        if overlaps(&taken, &range)
            || proper
                .iter()
                .any(|p| p.start <= range.start && range.end <= p.end)
        {
            continue;
        }
        let (number, unit) = split_kanji_unit(m.as_str());
        let Some(value) = kanji_to_number(number) else {
            continue;
        };
        let key = format!("{value}{}", normalize_unit(unit));
        push(out, FactKind::Number, range.clone(), key);
        taken.push(range);
    }

    for m in LATIN_RE.find_iter(text) {
        let range = m.range();
        if m.as_str().len() < 2 || overlaps(&taken, &range) {
            continue;
        }
        push(out, FactKind::Latin, range, m.as_str().to_ascii_lowercase());
    }
    for m in KATAKANA_RE.find_iter(text) {
        let word = m.as_str().trim_matches('・');
        if word.chars().count() < 4 || word.chars().all(|c| c == 'ー') {
            continue;
        }
        let start = m.start() + (m.as_str().len() - m.as_str().trim_start_matches('・').len());
        push(
            out,
            FactKind::Katakana,
            start..start + word.len(),
            word.to_string(),
        );
    }
    for caps in QUOTED_RE.captures_iter(text) {
        let inner = caps.name("a").or_else(|| caps.name("b")).expect("inner");
        let key: String = inner
            .as_str()
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        if key.is_empty() {
            continue;
        }
        push(
            out,
            FactKind::Quoted,
            caps.get(0).expect("group 0").range(),
            key,
        );
    }

    // リンク先を含む原文の包絡範囲で除外せず、表示される本文上で重なりを調べる。
    taken.sort_unstable_by_key(|r| r.start);
    let mut protected: Vec<Range<usize>> = Vec::new();
    for range in taken {
        if let Some(last) = protected.last_mut()
            && range.start <= last.end
        {
            last.end = last.end.max(range.end);
        } else {
            protected.push(range);
        }
    }
    for range in proper {
        let at = protected.partition_point(|r| r.end <= range.start);
        if !protected.get(at).is_some_and(|r| r.start < range.end) {
            let key = text[range.clone()].to_ascii_lowercase();
            push(out, FactKind::Proper, range, key);
        }
    }
}

fn overlaps(taken: &[Range<usize>], r: &Range<usize>) -> bool {
    taken.iter().any(|t| t.start < r.end && r.start < t.end)
}

fn touches_ascii_letter(text: &str, r: &Range<usize>) -> bool {
    let before = text[..r.start].chars().next_back();
    let after = text[r.end..].chars().next();
    before.is_some_and(|c| c.is_ascii_alphabetic())
        || after.is_some_and(|c| c.is_ascii_alphabetic())
}

/// URL の末尾に付いた句読点を外す。
fn trim_url(url: &str) -> &str {
    url.trim_end_matches([
        '.', ',', '。', '、', '！', '？', '!', '?', ':', ';', '’', '”',
    ])
}

/// 全角数字を半角にする。
fn ascii_digits(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '０'..='９' => char::from(b'0' + (c as u32 - '０' as u32) as u8),
            _ => c,
        })
        .collect()
}

/// 数字部分と単位に分ける。
fn split_unit(s: &str) -> (&str, &str) {
    let end = s
        .char_indices()
        .find(|&(_, c)| !(c.is_ascii_digit() || matches!(c, '０'..='９' | '.' | ',' | '．' | '，')))
        .map_or(s.len(), |(i, _)| i);
    (&s[..end], &s[end..])
}

/// 漢数字の部分と助数詞に分ける (「三万円」は「三万」と「円」。万・億は数に含める)。
fn split_kanji_unit(s: &str) -> (&str, &str) {
    let end = s
        .char_indices()
        .find(|&(_, c)| !"〇一二三四五六七八九十百千万億".contains(c))
        .map_or(s.len(), |(i, _)| i);
    (&s[..end], &s[end..])
}

/// 算用数字の値を正規化したキーにする。
///
/// 桁区切りを除き、全角を半角にそろえ、単位の頭の「万」「億」「兆」は数に掛ける
/// (「3万円」「三万円」「30000円」を同じキーにするため)。
fn number_key(number: &str, unit: &str) -> String {
    let digits = ascii_digits(number)
        .replace(['，', ','], "")
        .replace('．', ".");
    let (places, rest) = [("兆", 12), ("億", 8), ("万", 4)]
        .iter()
        .find_map(|&(prefix, places)| unit.strip_prefix(prefix).map(|r| (places, r)))
        .unwrap_or((0, unit));
    // 節番号や単位のない番号では、1.10 と 1.1 は別の識別子になり得る。
    let value = if places == 0 && matches!(rest, "" | "章" | "節" | "行" | "頁" | "ページ") {
        let trimmed = digits.trim_start_matches('0');
        if trimmed.is_empty() || trimmed.starts_with('.') {
            format!("0{trimmed}")
        } else {
            trimmed.to_string()
        }
    } else {
        decimal_key(&digits, places).unwrap_or(digits)
    };
    format!("{value}{}", normalize_unit(rest))
}

/// 小数点を文字列上で移す。浮動小数点の丸めで異なる金額を同一視しない。
fn decimal_key(digits: &str, places: usize) -> Option<String> {
    let (integer, fraction) = digits.split_once('.').unwrap_or((digits, ""));
    if integer.is_empty()
        || !integer
            .bytes()
            .chain(fraction.bytes())
            .all(|b| b.is_ascii_digit())
    {
        return None;
    }
    let moved = places.min(fraction.len());
    let mut expanded = String::with_capacity(digits.len() + places);
    expanded.push_str(integer);
    expanded.push_str(&fraction[..moved]);
    expanded.extend(std::iter::repeat_n('0', places - moved));
    let integer = expanded.trim_start_matches('0');
    let fraction = fraction[moved..].trim_end_matches('0');
    let mut value = if integer.is_empty() {
        "0".into()
    } else {
        integer.to_string()
    };
    if !fraction.is_empty() {
        value.push('.');
        value.push_str(fraction);
    }
    Some(value)
}

/// 単位の表記揺れをそろえる。
fn normalize_unit(unit: &str) -> &str {
    match unit {
        "ヶ月" | "カ月" | "ヵ月" | "箇月" => "か月",
        "％" => "%",
        "頁" => "ページ",
        other => other,
    }
}

/// 漢数字を整数に直す (「二百四十」→240、「二〇二六」→2026)。直せなければ `None`。
fn kanji_to_number(s: &str) -> Option<u64> {
    if s.is_empty() {
        return None;
    }
    let digit = |c: char| {
        "〇一二三四五六七八九"
            .chars()
            .position(|d| d == c)
            .map(|v| v as u64)
    };
    // 位取りの字を含まなければ、数字を並べたもの (二〇二六 = 2026)
    if s.chars().all(|c| digit(c).is_some()) {
        return s
            .chars()
            .try_fold(0u64, |acc, c| acc.checked_mul(10)?.checked_add(digit(c)?));
    }
    let (mut total, mut section, mut current) = (0u64, 0u64, 0u64);
    for c in s.chars() {
        if let Some(d) = digit(c) {
            current = current.checked_mul(10)?.checked_add(d)?;
            continue;
        }
        let unit = match c {
            '十' => 10,
            '百' => 100,
            '千' => 1000,
            '万' => 10_000,
            '億' => 100_000_000,
            _ => return None,
        };
        if unit >= 10_000 {
            section = section.checked_add(current)?;
            total = total.checked_add(section.max(1).checked_mul(unit)?)?;
            section = 0;
        } else {
            section = section.checked_add(current.max(1).checked_mul(unit)?)?;
        }
        current = 0;
    }
    total.checked_add(section)?.checked_add(current)
}

/// 前後の事実を種類とキーで数え、減ったものと増えたものを返す。
///
/// - 減ったもの: 数値・日付は 1 回でも減ったもの (その数字に触れた文が消えた)。語 (括弧の語・
///   英字の語・カタカナ語・固有名詞・URL) は改稿後に 1 度も出なくなったものだけ。何度も使う語を
///   1 回減らすのは普通の推敲なので出さない。人・組織・担当の語は、改稿前に主語などの位置に
///   あった語が、改稿後の文書のどこにも (位置を問わず) 出なくなったものだけ
/// - 増えたもの: 改稿前に 1 度も出ない事実。数値・日付は原文にない数字として強調する。
///   既にある事実を繰り返しただけのものは出さない。人・組織・担当の語は、主語などの位置に出た語が、
///   改稿前の文書のどこにも (位置を問わず) なかったものだけ
pub fn compare(before: &ExtractedFacts, after: &ExtractedFacts) -> FactChanges {
    #[derive(Default)]
    struct Tally<'a> {
        before: Vec<&'a Fact>,
        after: Vec<&'a Fact>,
    }
    let mut tallies: BTreeMap<(FactKind, &str), Tally<'_>> = BTreeMap::new();
    for f in &before.facts {
        tallies.entry((f.kind, &f.key)).or_default().before.push(f);
    }
    for f in &after.facts {
        tallies.entry((f.kind, &f.key)).or_default().after.push(f);
    }

    let mut changes = FactChanges::default();
    for ((kind, key), t) in tallies {
        let (nb, na) = (t.before.len(), t.after.len());
        let (lost, new) = if kind == FactKind::Actor {
            (
                nb > 0 && !after.actors.contains(key),
                na > 0 && !before.actors.contains(key),
            )
        } else if kind.is_verifiable() {
            (na < nb, nb == 0 && na > 0)
        } else {
            (na == 0 && nb > 0, nb == 0 && na > 0)
        };
        if lost {
            changes.removed.push(FactChange {
                kind,
                key: key.to_string(),
                text: t.before[0].text.clone(),
                before_count: nb,
                after_count: na,
                spans: t.before.iter().map(|f| f.span).collect(),
                suspicious: false,
            });
        } else if new {
            changes.added.push(FactChange {
                kind,
                key: key.to_string(),
                text: t.after[0].text.clone(),
                before_count: nb,
                after_count: na,
                spans: t.after.iter().map(|f| f.span).collect(),
                suspicious: kind.is_verifiable(),
            });
        }
    }
    let first = |c: &FactChange| c.spans.first().map_or(0, |s| s.start);
    changes.removed.sort_by_key(|c| (c.kind, first(c)));
    changes
        .added
        .sort_by_key(|c| (!c.suspicious, c.kind, first(c)));
    changes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(doc: &str) -> Vec<(FactKind, String)> {
        extract(&Document::markdown(doc))
            .facts
            .into_iter()
            .map(|f| (f.kind, f.key))
            .collect()
    }

    #[test]
    fn proper_nouns_in_headings_lists_tables_and_quotes_keep_source_positions() {
        let morph = crate::morph::testing::morphology(&[
            ("東京", "名詞,固有名詞,地域,一般"),
            ("京都", "名詞,固有名詞,地域,一般"),
            ("山田", "名詞,固有名詞,人名,姓"),
            ("トヨタ", "名詞,固有名詞,組織,*"),
            ("ソニー", "名詞,固有名詞,組織,*"),
            ("メール", "名詞,一般,*,*"),
        ]);
        let doc = Document::markdown(
            "# **東京**\n\n- 山田が[トヨタ](https://example.com)にメールした。\n\n| 拠点 |\n| --- |\n| 京都 |\n\n> ソニー。\n\n`東京`\n",
        );
        let facts = extract_with_morphology(&doc, Some(&morph));
        let proper: Vec<_> = facts
            .facts
            .iter()
            .filter(|f| f.kind == FactKind::Proper)
            .collect();
        assert_eq!(
            proper.iter().map(|f| f.text.as_str()).collect::<Vec<_>>(),
            ["東京", "山田", "トヨタ", "京都", "ソニー"]
        );
        for fact in proper {
            assert_eq!(doc.slice(fact.span), fact.text);
        }
        assert_eq!(extract_with_morphology(&doc, None), extract(&doc));
    }

    #[test]
    fn proper_nouns_do_not_duplicate_existing_words_or_url_and_number_parts() {
        let morph = crate::morph::testing::morphology(&[
            ("GitHub", "名詞,固有名詞,組織,*"),
            ("リポジトリ", "名詞,固有名詞,一般,*"),
            ("example", "名詞,固有名詞,組織,*"),
            ("5月", "名詞,固有名詞,人名,名"),
        ]);
        let doc =
            Document::plain_text("GitHub リポジトリ https://example.com 2026年5月1日 5月。\n");
        let facts = extract_with_morphology(&doc, Some(&morph));
        assert_eq!(facts, extract(&doc));
    }

    #[test]
    fn a_name_across_a_link_is_not_part_of_the_link_destination() {
        let morph = crate::morph::testing::morphology(&[
            ("東京", "名詞,固有名詞,地域,一般"),
            ("本社", "名詞,一般,*,*"),
        ]);
        let before = Document::markdown("[東](https://example.com)京。\n");
        let after = Document::markdown("[本社](https://example.com)。\n");
        let changes = compare(
            &extract_with_morphology(&before, Some(&morph)),
            &extract_with_morphology(&after, Some(&morph)),
        );
        assert_eq!(changes.removed.len(), 1);
        let removed = &changes.removed[0];
        assert_eq!(removed.kind, FactKind::Proper);
        assert_eq!(removed.text, "東京");
        assert_eq!(removed.spans[0].start, before.source.find('東').unwrap());
        assert_eq!(
            removed.spans[0].end,
            before.source.find('京').unwrap() + '京'.len_utf8()
        );
        assert!(changes.added.is_empty());
    }

    #[test]
    fn surface_words_keep_the_same_kind_when_their_context_changes() {
        let morph = crate::morph::testing::morphology(&[
            ("Python", "名詞,固有名詞,組織,*"),
            ("アサガオ", "名詞,固有名詞,組織,*"),
        ]);
        for text in ["Python3。", "Python。", "アサガオサーバー。", "アサガオ。"] {
            let doc = Document::plain_text(text);
            assert_eq!(extract_with_morphology(&doc, Some(&morph)), extract(&doc));
        }
    }

    #[test]
    fn kanji_number_prefixes_inside_names_are_not_numbers() {
        let morph = crate::morph::testing::morphology(&[
            ("四日市", "名詞,固有名詞,地域,一般"),
            ("二本松", "名詞,固有名詞,地域,一般"),
            ("本社", "名詞,一般,*,*"),
        ]);
        let changes = compare(
            &extract_with_morphology(&Document::plain_text("四日市と二本松へ。"), Some(&morph)),
            &extract_with_morphology(&Document::plain_text("本社へ。"), Some(&morph)),
        );
        assert_eq!(
            changes
                .removed
                .iter()
                .map(|f| (f.kind, f.text.as_str()))
                .collect::<Vec<_>>(),
            [(FactKind::Proper, "四日市"), (FactKind::Proper, "二本松")]
        );
        assert!(changes.added.is_empty());
    }

    #[test]
    fn proper_nouns_are_reported_only_when_lost_or_new_and_are_not_suspicious() {
        let morph = crate::morph::testing::morphology(&[
            ("東京", "名詞,固有名詞,地域,一般"),
            ("京都", "名詞,固有名詞,地域,一般"),
        ]);
        let before = extract_with_morphology(&Document::plain_text("東京。東京。\n"), Some(&morph));
        let after = extract_with_morphology(&Document::plain_text("東京。京都。\n"), Some(&morph));
        let changes = compare(&before, &after);
        assert!(changes.removed.is_empty());
        assert_eq!(changes.added.len(), 1);
        assert_eq!(changes.added[0].key, "京都");
        assert!(!changes.added[0].suspicious);
        let changes = compare(&after, &ExtractedFacts::default());
        assert_eq!(changes.removed.len(), 2);
    }

    #[test]
    fn normalizes_digits_units_and_kanji_numbers() {
        let k = keys("問い合わせは４０件、四十件、40件で、期間は三ヶ月と3か月、率は１２．５％。\n");
        let numbers: Vec<&str> = k
            .iter()
            .filter(|(kind, _)| *kind == FactKind::Number)
            .map(|(_, key)| key.as_str())
            .collect();
        assert_eq!(
            numbers,
            vec!["40件", "40件", "40件", "3か月", "3か月", "12.5%"]
        );
    }

    #[test]
    fn decimal_amounts_keep_all_digits_when_scaling_units() {
        for (source, expected) in [
            ("3.000万円", "30000円"),
            ("30000.000円", "30000円"),
            ("0.00001万円", "0.1円"),
            ("0.000000001億円", "0.1円"),
            ("0.0000000000001兆円", "0.1円"),
            ("9007199254740993万円", "90071992547409930000円"),
            ("99999999999999999兆円", "99999999999999999000000000000円"),
            ("０００３．０００万円", "30000円"),
            ("000.000万円", "0円"),
        ] {
            assert_eq!(
                keys(source),
                vec![(FactKind::Number, expected.into())],
                "{source}"
            );
        }
    }

    #[test]
    fn amount_changes_beyond_float_precision_are_not_lost() {
        let before = Document::plain_text("金額は9007199254740992万円です。");
        let after = Document::plain_text("金額は9007199254740993万円です。");
        let changes = compare(&extract(&before), &extract(&after));
        assert_eq!(changes.removed.len(), 1);
        assert_eq!(changes.added.len(), 1);
        assert_eq!(
            before.slice(changes.removed[0].spans[0]),
            "9007199254740992万円"
        );
        assert_eq!(
            after.slice(changes.added[0].spans[0]),
            "9007199254740993万円"
        );
        assert!(changes.added[0].suspicious);
    }

    #[test]
    fn section_and_unitless_identifiers_preserve_fractional_trailing_zeros() {
        for unit in ["", "節", "章", "行", "頁", "ページ"] {
            let before = Document::plain_text(format!("01.10{unit}"));
            let after = Document::plain_text(format!("1.1{unit}"));
            let changes = compare(&extract(&before), &extract(&after));
            assert_eq!(changes.removed.len(), 1, "{unit}");
            assert_eq!(changes.added.len(), 1, "{unit}");
            assert_eq!(
                changes.removed[0].key,
                format!("1.10{}", normalize_unit(unit))
            );
        }
    }

    #[test]
    fn kanji_numbers_need_a_counter_and_parse_positional_forms() {
        assert_eq!(kanji_to_number("二百四十"), Some(240));
        assert_eq!(kanji_to_number("十一"), Some(11));
        assert_eq!(kanji_to_number("二〇二六"), Some(2026));
        assert_eq!(kanji_to_number("三万五千"), Some(35_000));
        let k = keys("一部の人は一方で統一した。二百四十日かかった。\n");
        assert_eq!(k, vec![(FactKind::Number, "240日".to_string())]);
    }

    #[test]
    fn dates_are_taken_before_numbers() {
        let k = keys("2026年9月24日と2026-09-25と9月3日に公開した。\n");
        let dates: Vec<&str> = k
            .iter()
            .filter(|(kind, _)| *kind == FactKind::Date)
            .map(|(_, key)| key.as_str())
            .collect();
        assert_eq!(dates, vec!["2026-09-24", "2026-09-25", "--09-03"]);
        assert!(!k.iter().any(|(kind, _)| *kind == FactKind::Number));
    }

    #[test]
    fn latin_katakana_quoted_and_urls() {
        let src = "Slack と GitHub で「定例会議」を「月次」にした。リポジトリの Python3 と [手順](https://example.com/guide) を見る。\n";
        let k = keys(src);
        assert!(k.contains(&(FactKind::Latin, "slack".to_string())));
        assert!(k.contains(&(FactKind::Latin, "python3".to_string())));
        assert!(k.contains(&(FactKind::Katakana, "リポジトリ".to_string())));
        assert!(k.contains(&(FactKind::Quoted, "定例会議".to_string())));
        assert!(k.contains(&(FactKind::Url, "https://example.com/guide".to_string())));
        // 英字に接した数字は数値として数えない
        assert!(
            !k.iter()
                .any(|(kind, key)| *kind == FactKind::Number && key == "3")
        );
    }

    #[test]
    fn compare_reports_removed_and_suspicious_additions() {
        let before = extract(&Document::markdown(
            "問い合わせは40件から11件に減った。担当は Slack で連絡した。\n",
        ));
        let after = extract(&Document::markdown(
            "問い合わせは１１件に減り、満足度は35%上がった。\n",
        ));
        let changes = compare(&before, &after);
        let removed: Vec<&str> = changes.removed.iter().map(|c| c.key.as_str()).collect();
        // 主語の「担当」は、文ごと消えて改稿後のどこにも出ない
        assert_eq!(removed, vec!["40件", "slack", "担当"]);
        assert_eq!(changes.added.len(), 1);
        assert_eq!(changes.added[0].key, "35%");
        assert!(changes.added[0].suspicious);
    }

    #[test]
    fn words_count_only_when_they_disappear_or_first_appear() {
        let before = extract(&Document::markdown(
            "ドキュメントを書き、ドキュメントを読む。参加は40件、見学も40件だった。\n",
        ));
        let after = extract(&Document::markdown(
            "ドキュメントを書く。参加は40件だった。レビューを三回した。\n",
        ));
        let changes = compare(&before, &after);
        // 語の 2 → 1 回は出さず、数値の 2 → 1 回は出す
        let removed: Vec<(&str, usize, usize)> = changes
            .removed
            .iter()
            .map(|c| (c.key.as_str(), c.before_count, c.after_count))
            .collect();
        assert_eq!(removed, vec![("40件", 2, 1)]);
        let added: Vec<(&str, bool)> = changes
            .added
            .iter()
            .map(|c| (c.key.as_str(), c.suspicious))
            .collect();
        assert_eq!(added, vec![("3回", true), ("レビュー", false)]);

        // 既にある数値を繰り返しただけなら増えたものに出さない
        let again = extract(&Document::markdown(
            "参加は40件、見学も40件だった。確かに40件だ。\n",
        ));
        let before = extract(&Document::markdown("参加は40件、見学も40件だった。\n"));
        assert!(compare(&before, &again).added.is_empty());
    }

    /// 人・組織・担当の語の増減を、(消えた語の表記, 増えた語の表記) で返す。
    fn actor_changes(before: &str, after: &str) -> (Vec<String>, Vec<String>) {
        let changes = compare(
            &extract(&Document::markdown(before)),
            &extract(&Document::markdown(after)),
        );
        let texts = |list: &[FactChange]| -> Vec<String> {
            list.iter()
                .filter(|c| c.kind == FactKind::Actor)
                .map(|c| c.text.clone())
                .collect()
        };
        (texts(&changes.removed), texts(&changes.added))
    }

    fn words(list: &[&str]) -> Vec<String> {
        list.iter().map(|w| w.to_string()).collect()
    }

    #[test]
    fn actors_added_as_subjects_or_addressees_are_reported_but_not_suspicious() {
        let before = Document::markdown("設定を変更したら、再起動してください。\n");
        let after = Document::markdown(
            "設定を変更したら、担当者が再起動します。管理者に完了を報告してください。\n",
        );
        let changes = compare(&extract(&before), &extract(&after));
        assert!(changes.removed.is_empty());
        let actors: Vec<_> = changes
            .added
            .iter()
            .filter(|c| c.kind == FactKind::Actor)
            .collect();
        assert_eq!(
            actors
                .iter()
                .map(|c| (c.key.as_str(), c.text.as_str()))
                .collect::<Vec<_>>(),
            [("担当", "担当者"), ("管理者", "管理者")]
        );
        for c in actors {
            assert!(!c.suspicious);
            assert_eq!((c.before_count, c.after_count), (0, 1));
            assert_eq!(after.slice(c.spans[0]), c.text);
        }
    }

    #[test]
    fn actors_already_in_the_original_in_any_form_or_position_are_not_reported() {
        for (before, after) in [
            ("担当を決めてください。", "担当者が再起動します。"),
            ("私たちは確認します。", "私達が確認します。"),
            ("私たちは確認します。", "わたしたちが確認します。"),
            ("我々が進めます。", "われわれが進めます。"),
            // 同じものを指す言い換え (文体に合わせた入れ替え) は足したとみなさない
            ("今を生きる我々が進めます。", "私たちが進めます。"),
            ("当社が対応します。", "弊社が対応します。"),
            ("上司に報告します。", "上長に報告します。"),
            ("皆さんに伝えます。", "みなさんに伝えます。"),
            ("アドミンについて説明します。", "管理者が対応します。"),
            ("管理者について説明します。", "アドミンが対応します。"),
            // 複合語・引用・コード・引用ブロック・装飾の中にあれば、原文にあるとみなす
            ("システム管理者の手順です。", "管理者が再起動します。"),
            (
                "「担当者が再起動します。」とある。",
                "担当者が再起動します。",
            ),
            ("`担当者` を参照する。", "担当者が再起動します。"),
            ("> 担当者が再起動します。", "担当者が再起動します。"),
            ("**管理**者の手順です。", "管理者が再起動します。"),
        ] {
            assert_eq!(
                actor_changes(before, after),
                (vec![], vec![]),
                "{before} → {after}"
            );
        }
    }

    #[test]
    fn actors_inside_compounds_modifiers_quotes_and_hidden_text_are_not_taken() {
        for after in [
            "システム管理者が再起動します。",
            "営業の担当者が再起動します。",
            "ご担当者が再起動します。",
            "「担当者が再起動します。」と書いてあります。",
            "「説明、担当者が再起動します",
            "> 担当者が再起動します。",
            "~~担当者が再起動します。~~",
            "| 担当者 | 再起動します |\n| --- | --- |\n| 済み | 済み |",
            "私たち・管理者が再起動します。",
            "担当者もしくは管理者が再起動します。",
            "管理者もどきが再起動します。",
            "皆さんがたが再起動します。",
            "担当者様が再起動します。",
            "- 管理者に\n- 報告してください。",
            // 文の途中で折り返した行の頭は、文の頭にしない
            "営業の\n担当者が再起動します。",
            "システム\n管理者が再起動します。",
            // 助詞や述語に見える語の頭
            "担当がらみの不具合を調べます。",
            "管理者に通知しきい値の変更権限はありません。",
            "管理者に確認しかできません。",
            // 述語の残りが取り消し線に重なる
            "管理者に報告し~~ません~~。",
        ] {
            assert_eq!(
                actor_changes("再起動します。", after),
                (vec![], vec![]),
                "{after}"
            );
        }
    }

    #[test]
    fn topics_take_the_listed_particles_including_negations() {
        for (after, expected) in [
            ("担当者は作業を進めます。", &["担当者"][..]),
            ("担当者が作業を進めます。", &["担当者"]),
            ("担当者も作業を進めます。", &["担当者"]),
            ("当社では作業を進めます。", &["当社"]),
            ("責任者 が作業を進めます。", &[]),
            (
                "担当者ではなく、管理者が作業を進めます。",
                &["担当者", "管理者"],
            ),
            ("作業は、我々が進めます。", &["我々"]),
            ("作業を進める。 上司が確認する。", &["上司"]),
        ] {
            assert_eq!(
                actor_changes("作業を進めます。", after),
                (vec![], words(expected)),
                "{after}"
            );
        }
    }

    #[test]
    fn addressees_need_a_listed_predicate_right_after_or_after_a_short_object() {
        for (after, expected) in [
            ("管理者に報告してください。", true),
            ("管理者にご連絡ください。", true),
            ("管理者へも共有します。", true),
            ("管理者にはお知らせしません。", true),
            ("管理者にお伝えください。", true),
            ("管理者に知らせた。", true),
            ("管理者に頼んでください。", true),
            ("管理者に相談を依頼する。", true),
            ("管理者に完了を報告してください。", true),
            ("管理者にURLを共有します。", true),
            ("管理者に一二三四五六七八九十一二を報告します。", true),
            ("管理者に一二三四五六七八九十一二三を報告します。", false),
            ("管理者に完了について報告します。", false),
            ("管理者にメールで連絡します。", false),
            ("管理者に「完了」を報告します。", false),
            ("管理者に連絡先を尋ねてください。", false),
            ("管理者に確認画面を見せてください。", false),
            ("管理者に報告書を出してください。", false),
            ("管理者に頼る。", false),
            ("管理者に頼みごとをする。", false),
            ("管理者に頼もしさを感じる。", false),
            ("管理者に伝わる。", false),
            ("管理者に確認が必要です。", false),
            ("管理者において確認する。", false),
            ("管理者への報告をお願いします。", false),
        ] {
            let (_, added) = actor_changes("作業を進めます。", after);
            assert_eq!(!added.is_empty(), expected, "{after}");
        }
    }

    #[test]
    fn changed_addressees_and_explicit_subjects_are_reported_as_changes() {
        assert_eq!(
            actor_changes("担当者に報告します。", "管理者に報告します。"),
            (words(&["担当者"]), words(&["管理者"]))
        );
        // 語が新しいことだけを知らせる。原文の根拠を超えるかは読み手が確かめる
        assert_eq!(
            actor_changes(
                "作業は私だけで進めます。再起動します。",
                "作業は私だけで進めます。私たちが再起動します。"
            ),
            (vec![], words(&["私たち"]))
        );
    }

    #[test]
    fn actors_are_removed_only_when_the_word_disappears_entirely() {
        assert_eq!(
            actor_changes("担当者が再起動します。", "再起動してください。"),
            (words(&["担当者"]), vec![])
        );
        for after in [
            "「担当者」という表記を使います。",
            "システム担当が再起動します。",
            "作業の担当を決めます。",
        ] {
            assert_eq!(
                actor_changes("担当者が再起動します。", after),
                (vec![], vec![]),
                "{after}"
            );
        }
    }

    #[test]
    fn actors_in_headings_lists_tables_and_wrapped_lines_keep_source_positions() {
        let source = "# 担当者が行う作業\n\n- 管理者が実行します。\n\n| 手順 |\n| --- |\n| 責任者が承認します |\n\n設定を変えたら、上長に\n報告してください。\n\n　事務局が受け付けます。\n\n・弊社が手配します。\n\n**私たち**が確認します。\n\n[みなさん](https://example.com)も確認します。\n";
        let doc = Document::markdown(source);
        let extracted = extract(&doc);
        let actors: Vec<&Fact> = extracted
            .facts
            .iter()
            .filter(|f| f.kind == FactKind::Actor)
            .collect();
        assert_eq!(
            actors.iter().map(|f| f.text.as_str()).collect::<Vec<_>>(),
            [
                "担当者",
                "管理者",
                "責任者",
                "上長",
                "事務局",
                "弊社",
                "私たち",
                "みなさん"
            ]
        );
        for fact in &actors {
            assert_eq!(doc.slice(fact.span), fact.text);
            assert!(extracted.actors.contains(fact.key.as_str()), "{}", fact.key);
        }
    }

    #[test]
    fn repeated_new_actors_are_one_change_with_their_count() {
        let changes = compare(
            &extract(&Document::markdown("作業を進めます。\n")),
            &extract(&Document::markdown(
                "担当者が始めます。担当が続けます。担当者に報告します。\n",
            )),
        );
        let actors: Vec<_> = changes
            .added
            .iter()
            .filter(|c| c.kind == FactKind::Actor)
            .collect();
        assert_eq!(actors.len(), 1);
        assert_eq!(
            (actors[0].key.as_str(), actors[0].text.as_str()),
            ("担当", "担当者")
        );
        assert_eq!((actors[0].before_count, actors[0].after_count), (0, 3));
        assert_eq!(actors[0].spans.len(), 3);
    }
}
