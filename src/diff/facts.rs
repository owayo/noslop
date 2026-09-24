//! 事実の変化: 数字・日付・英字の語・カタカナ語・括弧の語・URL の消失と追加。
//!
//! 改稿で事実を落としていないか、原文にない事実 (とくに数字) を足していないかを
//! 確かめるための手掛かりを取り出す。表記の揺れ (全角と半角、漢数字と算用数字、
//! 「ヶ月」と「か月」) は正規化してから比べる。

use std::collections::BTreeMap;
use std::ops::Range;
use std::sync::LazyLock;

use regex::Regex;
use serde::Serialize;

use crate::diagnostic::Span;
use crate::document::{Block, Document, MarkKind};

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
const UNIT_PATTERN: &str = "万円|億円|年間|年代|か月|ヶ月|カ月|ヵ月|箇月|週間|日間|時間|分間|秒間|ページ|キロ|%|％|円|年|月|週|日|時|分|秒|歳|人|名|件|回|倍|割|個|本|社|台|点|字|文|行|章|節|頁|万|億|兆|kg|km|cm|mm|MB|GB|TB|KB|ms";

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
pub fn extract(doc: &Document) -> Vec<Fact> {
    let mut facts = Vec::new();
    for block in &doc.blocks {
        extract_block(doc, block, &mut facts);
    }
    facts.sort_by_key(|f| (f.span.start, f.kind));
    facts
}

fn extract_block(doc: &Document, block: &Block, out: &mut Vec<Fact>) {
    let text = block.text.as_str();
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
        if overlaps(&taken, &range) {
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
    let (factor, rest) = [("兆", 1e12), ("億", 1e8), ("万", 1e4)]
        .iter()
        .find_map(|&(prefix, f)| unit.strip_prefix(prefix).map(|r| (f, r)))
        .unwrap_or((1.0, unit));
    let value = if factor == 1.0 {
        let trimmed = digits.trim_start_matches('0');
        if trimmed.is_empty() || trimmed.starts_with('.') {
            format!("0{trimmed}")
        } else {
            trimmed.to_string()
        }
    } else {
        match digits.parse::<f64>() {
            Ok(v) => {
                let scaled = v * factor;
                if scaled.fract() == 0.0 && scaled < 9.0e15 {
                    format!("{}", scaled as u64)
                } else {
                    format!("{scaled}")
                }
            }
            Err(_) => digits,
        }
    };
    format!("{value}{}", normalize_unit(rest))
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
///   英字の語・カタカナ語・URL) は改稿後に 1 度も出なくなったものだけ。何度も使う語を
///   1 回減らすのは普通の推敲なので出さない
/// - 増えたもの: 改稿前に 1 度も出ない事実。数値・日付は原文にない数字として強調する。
///   既にある事実を繰り返しただけのものは出さない
pub fn compare(before: &[Fact], after: &[Fact]) -> FactChanges {
    #[derive(Default)]
    struct Tally<'a> {
        before: Vec<&'a Fact>,
        after: Vec<&'a Fact>,
    }
    let mut tallies: BTreeMap<(FactKind, &str), Tally<'_>> = BTreeMap::new();
    for f in before {
        tallies.entry((f.kind, &f.key)).or_default().before.push(f);
    }
    for f in after {
        tallies.entry((f.kind, &f.key)).or_default().after.push(f);
    }

    let mut changes = FactChanges::default();
    for ((kind, key), t) in tallies {
        let (nb, na) = (t.before.len(), t.after.len());
        let lost = if kind.is_verifiable() {
            na < nb
        } else {
            na == 0 && nb > 0
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
        } else if nb == 0 && na > 0 {
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
            .into_iter()
            .map(|f| (f.kind, f.key))
            .collect()
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
        assert_eq!(removed, vec!["40件", "slack"]);
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
}
