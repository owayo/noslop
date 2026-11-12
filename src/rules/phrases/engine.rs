//! 語句辞書による照合の共通部品。
//!
//! 辞書の 1 項目 ([`Entry`]) はリテラルか正規表現で、項目ごとに重大度を持ち、手掛かりの強さを
//! ルールの値から変えることもできる。
//! ルールは項目の並び ([`PhraseSpec`]) を持ち、文ごとに一致を探して診断にする。

use std::collections::BTreeMap;
use std::ops::Range;

use aho_corasick::{AhoCorasick, AhoCorasickBuilder, Input, MatchKind};
use regex::Regex;

use crate::diagnostic::{Diagnostic, Severity, Signal, Span};
use crate::rules::{Rule, RuleContext, RuleMeta, RuleUnit, quote};
use crate::text::quotes::QuoteRanges;

/// 辞書の項目の照合方法。
#[derive(Debug, Clone, Copy)]
pub(super) enum Pattern {
    Literal(&'static str),
    Regex(&'static str),
}

impl Pattern {
    /// 項目を見分ける名前 (診断の metrics の `item`)。リテラルはその文字列、正規表現は
    /// `/パターン/`。`noslop calibrate` は、一致した表記ではなくこの名前で項目ごとに集計する。
    pub fn item(&self) -> String {
        match self {
            Pattern::Literal(s) => (*s).to_string(),
            Pattern::Regex(s) => format!("/{s}/"),
        }
    }
}

/// 辞書の 1 項目。
#[derive(Debug, Clone, Copy)]
pub(super) struct Entry {
    pub pattern: Pattern,
    pub severity: Severity,
    /// 手掛かりの強さ。`None` ならルールの値 ([`RuleMeta::signal`]) を使う。
    ///
    /// ルールの強さは、ルールの校正の基準の重大度以上の項目で測ったもの。基準より軽い項目
    /// (警告のルールにある情報の項目) はその測定に入らないので、必ず自分の強さを書く。
    pub signal: Option<Signal>,
}

impl Entry {
    /// リテラル。
    pub const fn lit(s: &'static str, severity: Severity) -> Self {
        Self {
            pattern: Pattern::Literal(s),
            severity,
            signal: None,
        }
    }

    /// 正規表現。
    pub const fn re(s: &'static str, severity: Severity) -> Self {
        Self {
            pattern: Pattern::Regex(s),
            severity,
            signal: None,
        }
    }

    /// 手掛かりの強さをルールの値から変える (項目ごとに測った強さがルールと違うときなど)。
    pub const fn with_signal(self, signal: Signal) -> Self {
        Self {
            signal: Some(signal),
            ..self
        }
    }

    /// この項目の指摘の手掛かりの強さ。
    pub fn signal_in(&self, meta: &RuleMeta) -> Option<Signal> {
        self.signal.or(meta.signal)
    }
}

/// 正規表現の項目で、指す範囲を表す名前付きグループの名前。
///
/// このグループを持つ正規表現は、グループの範囲だけを一致として扱う。前後の文字を
/// 条件にだけ使い、指す範囲に含めないため (Rust の regex は前後の読みを使えない)。
/// 例: `(?:^|[^が])(?P<m>大事なのは、)` は「〜が大事なのは、」を拾わず、「大事なのは、」だけを指す。
const MATCH_GROUP: &str = "m";

/// 語句辞書で動くルールの定義。
pub(super) struct PhraseSpec {
    pub meta: RuleMeta,
    pub entries: &'static [Entry],
    /// 診断メッセージの雛形。`{m}` を一致した文字列に置き換える。
    pub message: &'static str,
    pub hint: &'static str,
}

/// 文の中の一致 1 件 (文のテキスト上の範囲と、項目の添字)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Hit {
    pub range: Range<usize>,
    pub entry: usize,
}

/// 正規表現の項目 1 つ。
struct RegexEntry {
    /// 項目の添字。
    entry: usize,
    re: Regex,
    /// 名前付きグループ [`MATCH_GROUP`] を持つか。
    grouped: bool,
}

/// 項目の並びから作った照合器。
pub(super) struct Matcher {
    literals: Option<AhoCorasick>,
    /// Aho-Corasick のパターン番号 → 項目の添字。
    literal_entries: Vec<usize>,
    regexes: Vec<RegexEntry>,
    /// 重なる追加項目より先に採る、先頭の項目数 (0 なら開始位置と長さで選ぶ)。
    preferred_entries: usize,
}

impl Matcher {
    pub fn new(entries: &[Entry]) -> Self {
        let mut literal_patterns = Vec::new();
        let mut literal_entries = Vec::new();
        let mut regexes = Vec::new();
        for (idx, entry) in entries.iter().enumerate() {
            match entry.pattern {
                Pattern::Literal(s) => {
                    literal_patterns.push(s);
                    literal_entries.push(idx);
                }
                Pattern::Regex(s) => {
                    let re = Regex::new(s)
                        .unwrap_or_else(|e| panic!("組み込みの正規表現が不正です ({s}): {e}"));
                    let grouped = re.capture_names().any(|n| n == Some(MATCH_GROUP));
                    regexes.push(RegexEntry {
                        entry: idx,
                        re,
                        grouped,
                    });
                }
            }
        }
        let literals = (!literal_patterns.is_empty()).then(|| {
            AhoCorasickBuilder::new()
                .match_kind(MatchKind::LeftmostLongest)
                .build(&literal_patterns)
                .expect("組み込みの語句辞書から照合器を作れません")
        });
        Self {
            literals,
            literal_entries,
            regexes,
            preferred_entries: 0,
        }
    }

    /// 文のテキストから一致を探す。重なる一致は、先に始まり長いほうを 1 件だけ残す。
    pub fn find(&self, text: &str) -> Vec<Hit> {
        self.find_except(text, |_| None)
    }

    /// 言及 (引用した語句など) として除く一致を除いて、一致を探す。
    ///
    /// `skip` は、除く一致なら、その言及の終わり (次の一致を探し始める位置) を返す (除かないなら
    /// `None`)。除いた一致の開始位置から言及の終わりまでを言及の範囲とし、そこから始まる一致は
    /// どの項目のものも除く (同じ言及の中の短い語句を拾い直さないように)。言及の範囲の後ろの一致は
    /// 隠さない。言及の終わり (少なくとも除いた一致の開始位置の次の字) から探し直し、残した一致の
    /// 間だけで重なりを解消する (引用の中の「単なる」から始まる一致が、引用の外の
    /// 「単なる道具ではなく」を飲み込まないように)。
    pub fn find_except(
        &self,
        text: &str,
        skip: impl Fn(&Range<usize>) -> Option<usize>,
    ) -> Vec<Hit> {
        // 除いた一致 `range` の後ろで探し直す位置。`at` を、一致の開始位置の次の字から
        // テキストの末尾までに収める (開始位置が末尾なら探し終える)
        let resume = |range: &Range<usize>, at: usize| {
            after_char(text, range.start).map(|next| at.clamp(next, text.len()))
        };
        // 除いた一致の言及の範囲 (開始位置から探し直す位置まで)
        let mut mentions: Vec<Range<usize>> = Vec::new();
        let mut hits = Vec::new();
        // 照合方法によらず、空の一致・言及の除外・再開位置を同じ手順で扱う。
        let mut record = |range: Range<usize>, entry: usize| {
            if range.is_empty() {
                return resume(&range, range.end);
            }
            match skip(&range) {
                None => {
                    // 追加の正規表現が長い範囲を拾っても、その途中から始まる候補を失わない。
                    // 既存の一致との優先順位を決めるのは、すべて集めてからにする。
                    let next = if self.preferred_entries > 0 && entry >= self.preferred_entries {
                        after_char(text, range.start)?
                    } else {
                        range.end
                    };
                    hits.push(Hit { range, entry });
                    Some(next)
                }
                Some(at) => {
                    let next = resume(&range, at)?;
                    mentions.push(range.start..next);
                    Some(next)
                }
            }
        };
        if let Some(ac) = &self.literals {
            let mut position = 0;
            while let Some(m) = ac.find(Input::new(text).range(position..)) {
                let Some(next) = record(
                    m.start()..m.end(),
                    self.literal_entries[m.pattern().as_usize()],
                ) else {
                    break;
                };
                position = next;
            }
        }
        for r in &self.regexes {
            if r.grouped {
                // グループを取り出すのは遅いので、グループを持つ正規表現だけにする
                let mut position = 0;
                while let Some(caps) = r.re.captures_at(text, position) {
                    let mut next = caps.get(0).expect("全体の一致がありません").end();
                    if let Some(m) = caps.name(MATCH_GROUP)
                        && m.start() < m.end()
                    {
                        // 後ろの条件は次の一致でも使えるよう、指した範囲の末尾から探す。
                        next = record(m.start()..m.end(), r.entry).unwrap_or(m.end());
                    }
                    if next > position {
                        position = next;
                    } else if let Some(ch) = text[position..].chars().next() {
                        // 空・不参加のグループで全体も空なら、文字の境界まで進める。
                        position += ch.len_utf8();
                    } else {
                        break;
                    }
                }
            } else {
                let mut position = 0;
                while let Some(m) = r.re.find_at(text, position) {
                    let Some(next) = record(m.start()..m.end(), r.entry) else {
                        break;
                    };
                    position = next;
                }
            }
        }
        select_non_overlapping(hits, mentions, self.preferred_entries)
    }
}

/// `at` の次の文字の境界。`at` が末尾なら `None`。
fn after_char(text: &str, at: usize) -> Option<usize> {
    text[at..].chars().next().map(|c| at + c.len_utf8())
}

/// 重なる一致から、先に始まり長いものを優先して重ならない組を選ぶ。
fn select_non_overlapping(
    mut hits: Vec<Hit>,
    mut mentions: Vec<Range<usize>>,
    preferred_entries: usize,
) -> Vec<Hit> {
    hits.sort_by(|a, b| {
        a.range
            .start
            .cmp(&b.range.start)
            .then(b.range.end.cmp(&a.range.end))
            .then(a.entry.cmp(&b.entry))
    });
    // 別の項目で見つけた一致も、言及の範囲から始まるものは除く。
    // 両方を開始位置順にたどり、重なる言及の終点は最大値を保つ。
    // 一致ごとに全言及を探し直さず、各言及を一度だけ読む。
    if !mentions.is_empty() {
        mentions.sort_unstable_by_key(|m| m.start);
        let mut next = 0;
        let mut end = 0;
        hits.retain(|hit| {
            while let Some(mention) = mentions.get(next)
                && mention.start <= hit.range.start
            {
                end = end.max(mention.end);
                next += 1;
            }
            hit.range.start >= end
        });
    }
    if preferred_entries > 0 {
        // 安定ソートで、先頭の項目を従来どおりの開始位置・長さの順に先に採る。
        // 後から読む追加項目は開始位置が前にも戻るので、直前の一致との比較では足りない。
        hits.sort_by_key(|hit| hit.entry >= preferred_entries);
        let mut selected: BTreeMap<usize, Hit> = BTreeMap::new();
        for hit in hits {
            // 採った区間は重ならず、終点も単調に増える。候補の終点より前で始まる最後の
            // 区間だけを見れば重なりが分かり、全区間を毎回探さずに済む。
            if selected
                .range(..hit.range.end)
                .next_back()
                .is_some_and(|(_, prior)| prior.range.end > hit.range.start)
            {
                continue;
            }
            selected.insert(hit.range.start, hit);
        }
        return selected.into_values().collect();
    }
    let mut out: Vec<Hit> = Vec::with_capacity(hits.len());
    for hit in hits {
        if out
            .last()
            .is_some_and(|last| hit.range.start < last.range.end)
        {
            continue;
        }
        out.push(hit);
    }
    out
}

/// 語句系ルールの診断を組み立てる。
///
/// metrics には、一致した表記 (`matched`) と、項目を見分ける名前 (`item`) を載せる。
/// ここでは `item` を一致した表記にしておく。辞書の項目で拾うルールは、項目の名前
/// ([`Pattern::item`]) で上書きする (正規表現の項目が表記ごとに分かれないように)。
#[allow(clippy::too_many_arguments)]
pub(super) fn diagnostic(
    meta: &'static RuleMeta,
    span: Span,
    context: Span,
    matched: &str,
    message: String,
    hint: &str,
    severity: Severity,
    signal: Option<Signal>,
) -> Diagnostic {
    let quoted = quote(matched);
    let mut d = meta
        .diagnostic(span, message)
        .with_hint(hint)
        .with_context(context)
        .with_metric("item", quoted.clone())
        .with_metric("matched", quoted);
    d.severity = severity;
    d.signal = signal;
    d
}

/// 語句辞書で動くルール。
///
/// 同じブロック内で対応する引用符 (「」・『』・“”・二重引用符) の内側に完全に収まる一致と、
/// 開き引用符の直後の一致は、語句への言及として除く ([`QuoteRanges`])。辞書の語句は書き手の
/// 言い回しの癖を指すもので、引用した語句や発言は書き手の言い回しではないため。除いた一致は、
/// 重なる後ろの一致を隠さない ([`Matcher::find_except`])。
pub(super) struct PhraseRule {
    spec: &'static PhraseSpec,
    matcher: Matcher,
    exclude_quoted_mentions: bool,
    /// 文書の結び (地の文の最後の段落) だけを見るときの段落の数。
    closing_paragraphs: Option<usize>,
}

impl PhraseRule {
    pub fn new(spec: &'static PhraseSpec) -> Self {
        Self {
            spec,
            matcher: Matcher::new(spec.entries),
            exclude_quoted_mentions: true,
            closing_paragraphs: None,
        }
    }

    /// 重なる追加項目が、先頭の `count` 項目の範囲や強さを置き換えないようにする。
    /// 先頭の項目どうしは従来どおり最長を選ぶ。追加項目は正規表現で定義する。
    pub fn prefer_first(mut self, count: usize) -> Self {
        self.matcher.preferred_entries = count;
        self
    }

    /// 引用の内側の一致も数える (辞書の照合だけを確かめるテスト向け)。
    #[cfg(test)]
    pub fn including_quoted_mentions(mut self) -> Self {
        self.exclude_quoted_mentions = false;
        self
    }

    /// 文書の結び ([`Document::closing_paragraphs`] の最後の `count` 段落) にある語句だけを拾う。
    ///
    /// 判定に文書の構造が要るので、断片の集まり (コードのコメント・表計算のセル) には当てない。
    ///
    /// [`Document::closing_paragraphs`]: crate::document::Document::closing_paragraphs
    pub fn only_in_closing(mut self, count: usize) -> Self {
        self.closing_paragraphs = Some(count);
        self
    }
}

impl Rule for PhraseRule {
    fn unit(&self) -> RuleUnit {
        if self.closing_paragraphs.is_some() {
            RuleUnit::Document
        } else {
            RuleUnit::Sentence
        }
    }

    fn meta(&self) -> &'static RuleMeta {
        &self.spec.meta
    }

    fn check(&self, ctx: &RuleContext<'_>, out: &mut Vec<Diagnostic>) {
        let closing = self
            .closing_paragraphs
            .map(|count| ctx.doc.closing_paragraphs(count));
        for (idx, block) in ctx.scoped_blocks() {
            if closing.as_ref().is_some_and(|c| !c.contains(&idx)) {
                continue;
            }
            let quotes = self
                .exclude_quoted_mentions
                .then(|| QuoteRanges::new(&block.text));
            for sentence in ctx.doc.block_sentences(idx) {
                let text = &block.text[sentence.range.clone()];
                let offset = sentence.range.start;
                // 引用の中の一致は除き、引用の閉じ (閉じない引用は一致の末尾) から探し直す
                let hits = self.matcher.find_except(text, |r| {
                    let quotes = quotes.as_ref()?;
                    let abs = offset + r.start..offset + r.end;
                    quotes
                        .excludes(&abs)
                        .then(|| quotes.resume_after(&abs).saturating_sub(offset))
                });
                for hit in hits {
                    let entry = &self.spec.entries[hit.entry];
                    let matched = &text[hit.range.clone()];
                    let abs = (offset + hit.range.start)..(offset + hit.range.end);
                    let message = self.spec.message.replace("{m}", &quote(matched));
                    out.push(
                        diagnostic(
                            &self.spec.meta,
                            block.to_source(abs),
                            sentence.span,
                            matched,
                            message,
                            self.spec.hint,
                            entry.severity,
                            entry.signal_in(&self.spec.meta),
                        )
                        .with_metric("item", entry.pattern.item()),
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ENTRIES: &[Entry] = &[
        Entry::lit("言える", Severity::Warning),
        Entry::lit("と言えるでしょう", Severity::Warning),
        Entry::lit("でしょう", Severity::Info).with_signal(Signal::Weak),
        Entry::re(r"ことが(でき|出来)(る|ます)", Severity::Info),
    ];

    #[test]
    fn prefers_leftmost_longest_and_drops_overlaps() {
        let m = Matcher::new(ENTRIES);
        let hits = m.find("これはと言えるでしょう。");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].entry, 1);
    }

    #[test]
    fn every_entry_is_matched() {
        let m = Matcher::new(ENTRIES);
        assert_eq!(m.find("晴れるでしょう").len(), 1);
    }

    #[test]
    fn skipped_hits_do_not_hide_later_overlapping_hits() {
        let texts = |text: &str, hits: &[Hit]| -> Vec<String> {
            hits.iter()
                .map(|h| text[h.range.clone()].to_string())
                .collect()
        };
        // リテラル: 先に始まる長い一致を除き、返した位置 (その中の次の字) から探し直す
        let m = Matcher::new(ENTRIES);
        let text = "これはと言えるでしょう。";
        let start = text.find("と言える").unwrap();
        let skip = |r: &Range<usize>| (r.start == start).then_some(r.start);
        assert_eq!(
            texts(text, &m.find_except(text, skip)),
            ["言える", "でしょう"]
        );
        // 返した位置が一致の末尾なら、一致の中は探し直さない
        let skip = |r: &Range<usize>| (r.start == start).then_some(r.end);
        assert!(m.find_except(text, skip).is_empty());
        // 正規表現 (グループあり・なし): 引用の中の「単なる」から始まる一致を除いても、
        // その一致に含まれていた後ろの「単なる」を拾う
        const FRAMED: &[Entry] = &[Entry::re(r"単なる[^。]{1,25}?ではなく", Severity::Info)];
        const GROUPED_FRAMED: &[Entry] = &[Entry::re(
            r"(?:^|[^が])(?P<m>単なる[^。]{1,25}?ではなく)",
            Severity::Info,
        )];
        let text = "「単なる作業」と、これは単なる道具ではなく、体験だ。";
        let quoted = text.find("単なる").unwrap();
        let closing = text.find('」').unwrap();
        for entries in [FRAMED, GROUPED_FRAMED] {
            let m = Matcher::new(entries);
            let all = m.find(text);
            assert_eq!(all.len(), 1);
            assert_eq!(all[0].range.start, quoted);
            let skip = |r: &Range<usize>| (r.start == quoted).then_some(closing);
            assert_eq!(
                texts(text, &m.find_except(text, skip)),
                ["単なる道具ではなく"]
            );
        }
    }

    #[test]
    fn a_skipped_mention_hides_shorter_hits_of_other_entries_inside_it() {
        // リテラルの「することができる」を言及として除くと、その中から始まる正規表現の
        // 「ことができる」も同じ言及として除く
        const ENTRIES: &[Entry] = &[
            Entry::lit("することができる", Severity::Info),
            Entry::re(r"ことが(でき|出来)(る|ます)", Severity::Info),
        ];
        let m = Matcher::new(ENTRIES);
        let text = "することができる。";
        assert_eq!(m.find(text).len(), 1);
        let skip = |r: &Range<usize>| (r.start == 0).then_some(r.end);
        assert!(m.find_except(text, skip).is_empty());
    }

    #[test]
    fn mentions_from_different_matchers_can_overlap_and_arrive_out_of_order() {
        const ENTRIES: &[Entry] = &[
            Entry::lit("AA", Severity::Info),
            Entry::lit("EE", Severity::Info),
            Entry::lit("FF", Severity::Info),
            Entry::re("BB", Severity::Info),
            Entry::re("(?P<m>CC)", Severity::Info),
            Entry::re("DD", Severity::Info),
        ];
        let text = "AA BB CC DD EE FF";
        let hits = Matcher::new(ENTRIES).find_except(text, |range| match range.start {
            0 => Some(9),
            3 => Some(15),
            12 => Some(14),
            _ => None,
        });
        // 先に集めた 12..14 の範囲も、後で集める 3..15 に含まれる。
        // 終点の 15 から始まる一致は残す。
        assert_eq!(hits.len(), 1);
        assert_eq!(&text[hits[0].range.clone()], "FF");
        assert_eq!(hits[0].entry, 2);
    }

    #[test]
    fn empty_matches_do_not_call_skip_and_resumption_is_bounded() {
        for entry in [
            Entry::lit("", Severity::Info),
            Entry::re("", Severity::Info),
        ] {
            assert!(
                Matcher::new(&[entry])
                    .find_except("日本語", |_| panic!("空の一致は除外の判定に渡さない"))
                    .is_empty()
            );
        }
        for entry in [
            Entry::lit("日本", Severity::Info),
            Entry::re("日本", Severity::Info),
            Entry::re("(?P<m>日本)", Severity::Info),
        ] {
            assert!(
                Matcher::new(&[entry])
                    .find_except("日本語日本", |_| Some(usize::MAX))
                    .is_empty()
            );
        }
    }

    #[test]
    fn regex_and_literal_hits_are_merged_in_order() {
        let m = Matcher::new(ENTRIES);
        let hits = m.find("読むことができる。言える。");
        let entries: Vec<_> = hits.iter().map(|h| h.entry).collect();
        assert_eq!(entries, vec![3, 0]);
    }

    #[test]
    fn quote_replaces_placeholders_and_collapses_spaces() {
        assert_eq!(quote("\u{FFFC}を  使う"), "…を 使う");
    }

    static SPEC: PhraseSpec = PhraseSpec {
        meta: RuleMeta {
            id: "T90",
            name: "TEST_PHRASES",
            title: "テスト",
            lane: crate::diagnostic::Lane::Slop,
            signal: Some(Signal::Strong),
            default_severity: Severity::Warning,
            summary: "テスト用",
            explanation: "テスト用",
        },
        entries: ENTRIES,
        message: "「{m}」",
        hint: "テスト用",
    };

    fn text_metric<'a>(d: &'a Diagnostic, key: &str) -> Option<&'a str> {
        match d.metrics.get(key) {
            Some(crate::diagnostic::Metric::Text(s)) => Some(s.as_str()),
            _ => None,
        }
    }

    #[test]
    fn diagnostics_name_the_item_separately_from_the_matched_text() {
        use crate::rules::testing::{matched, run};
        let md = "読むことができる。書くことが出来ます。それは言える。\n";
        let d = run(&PhraseRule::new(&SPEC), md);
        assert_eq!(
            matched(md, &d),
            vec!["ことができる", "ことが出来ます", "言える"]
        );
        let items: Vec<_> = d.iter().map(|d| text_metric(d, "item")).collect();
        let pattern = "/ことが(でき|出来)(る|ます)/";
        assert_eq!(items, vec![Some(pattern), Some(pattern), Some("言える")]);
        let notations: Vec<_> = d.iter().map(|d| text_metric(d, "matched")).collect();
        assert_eq!(
            notations,
            vec![Some("ことができる"), Some("ことが出来ます"), Some("言える")]
        );
        assert_eq!(d[0].severity, Severity::Info);
        assert_eq!(d[2].severity, Severity::Warning);
    }

    #[test]
    fn entries_inherit_or_override_the_signal_of_the_rule() {
        use crate::rules::testing::run;
        let md = "晴れるでしょう。それは言える。\n";
        let d = run(&PhraseRule::new(&SPEC), md);
        let signals: Vec<_> = d
            .iter()
            .map(|d| (text_metric(d, "item"), d.signal))
            .collect();
        assert_eq!(
            signals,
            vec![
                (Some("でしょう"), Some(Signal::Weak)),
                (Some("言える"), Some(Signal::Strong)),
            ]
        );
        // メッセージに強さの断り書きは足さない (強さは出力の札で示す)
        assert!(
            d.iter()
                .all(|d| d.message.starts_with('「') && d.message.ends_with('」'))
        );
    }

    #[test]
    fn rules_without_dictionary_items_use_the_matched_text_as_the_item() {
        let span = Span::new(0, 3);
        let d = diagnostic(
            &SPEC.meta,
            span,
            span,
            "\u{FFFC}を  示す",
            "テスト".into(),
            "テスト",
            Severity::Info,
            Some(Signal::Moderate),
        );
        assert_eq!(text_metric(&d, "item"), Some("…を 示す"));
        assert_eq!(text_metric(&d, "matched"), Some("…を 示す"));
        assert_eq!(d.signal, Some(Signal::Moderate));
    }

    #[test]
    fn a_named_group_narrows_the_match_to_the_group() {
        // 前の文字は条件にだけ使い、指す範囲はグループ m に限る
        const GROUPED: &[Entry] = &[Entry::re(r"(?:^|[^が])(?P<m>大事なのは、)", Severity::Info)];
        let m = Matcher::new(GROUPED);
        let text = "ここで大事なのは、頻度だ。";
        let hits = m.find(text);
        assert_eq!(hits.len(), 1);
        assert_eq!(&text[hits[0].range.clone()], "大事なのは、");
        // 文頭でも条件を満たす
        let text = "大事なのは、頻度だ。";
        assert_eq!(&text[m.find(text)[0].range.clone()], "大事なのは、");
        // 条件を満たさない位置は拾わない
        assert!(m.find("確認が大事なのは、夜に変わるからだ。").is_empty());
        // グループを持たない正規表現は一致の全体を指す (既存の動き)
        let plain = Matcher::new(ENTRIES);
        let text = "読むことができる。";
        assert_eq!(&text[plain.find(text)[0].range.clone()], "ことができる");
    }

    #[test]
    fn grouped_matches_can_share_context() {
        const GROUPED: &[Entry] = &[Entry::re(r"(?:^|。)(?P<m>要約)(?:。|$)", Severity::Info)];
        let matcher = Matcher::new(GROUPED);
        let text = "要約。要約。要約";
        let ranges: Vec<_> = matcher
            .find(text)
            .into_iter()
            .map(|hit| hit.range)
            .collect();
        assert_eq!(ranges, vec![0..6, 9..15, 18..24]);
        // 候補が増えても、同じ位置では長い指摘を優先する。
        let entries = [GROUPED[0], Entry::lit("要約。要約", Severity::Info)];
        let matcher = Matcher::new(&entries);
        let ranges: Vec<_> = matcher
            .find(text)
            .into_iter()
            .map(|hit| hit.range)
            .collect();
        assert_eq!(ranges, vec![0..15, 18..24]);
    }

    #[test]
    fn empty_and_absent_groups_advance_on_character_boundaries() {
        for pattern in [r"(?P<m>要約|)", r"(?P<m>要約)?"] {
            let entries = [Entry::re(pattern, Severity::Info)];
            let matcher = Matcher::new(&entries);
            let text = "あ要約。要約い";
            let ranges: Vec<_> = matcher
                .find(text)
                .into_iter()
                .map(|hit| hit.range)
                .collect();
            assert_eq!(ranges, vec![3..9, 12..18], "{pattern}");
            assert!(matcher.find("").is_empty());
            assert!(matcher.find("あいう").is_empty());
        }
    }

    #[test]
    fn grouped_search_keeps_anchors_relative_to_the_whole_text() {
        const GROUPED: &[Entry] = &[Entry::re(r"^(?P<m>要約)", Severity::Info)];
        let matcher = Matcher::new(GROUPED);
        let hits = matcher.find("要約要約");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].range, 0..6);
    }

    #[test]
    fn entries_keep_their_severity_and_signal() {
        const MARKED: &[Entry] =
            &[Entry::lit("大切です", Severity::Info).with_signal(Signal::Moderate)];
        assert_eq!(MARKED[0].signal, Some(Signal::Moderate));
        assert_eq!(MARKED[0].severity, Severity::Info);
        assert_eq!(MARKED[0].signal_in(&SPEC.meta), Some(Signal::Moderate));
        assert_eq!(ENTRIES[0].signal_in(&SPEC.meta), Some(Signal::Strong));
    }
}
