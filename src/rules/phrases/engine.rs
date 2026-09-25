//! 語句辞書による照合の共通部品。
//!
//! 辞書の 1 項目 ([`Entry`]) はリテラルか正規表現で、項目ごとに重大度と校正状況を持つ。
//! ルールは項目の並び ([`PhraseSpec`]) を持ち、文ごとに一致を探して診断にする。

use std::ops::Range;

use aho_corasick::{AhoCorasick, AhoCorasickBuilder, MatchKind};
use regex::Regex;

use crate::diagnostic::{Diagnostic, RuleStatus, Severity, Span};
use crate::rules::{Rule, RuleContext, RuleMeta, quote};

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
    pub status: RuleStatus,
    /// メッセージの末尾に添える注記 (弱いシグナルの断り書きなど)。
    pub note: Option<&'static str>,
}

/// 人間の文章にも一定数出るため重大度を下げた語句に添える注記。
pub(super) const WEAK_SIGNAL_NOTE: &str =
    "人間の文章にもよく出るため、弱い手掛かりとして扱っています";

impl Entry {
    /// 校正済みのリテラル。
    pub const fn lit(s: &'static str, severity: Severity) -> Self {
        Self {
            pattern: Pattern::Literal(s),
            severity,
            status: RuleStatus::Stable,
            note: None,
        }
    }

    /// 校正済みの正規表現。
    pub const fn re(s: &'static str, severity: Severity) -> Self {
        Self {
            pattern: Pattern::Regex(s),
            severity,
            status: RuleStatus::Stable,
            note: None,
        }
    }

    /// 校正済みだが人間の文章にも出る弱いシグナル (情報レベル)。
    pub const fn weak(s: &'static str) -> Self {
        Self {
            pattern: Pattern::Literal(s),
            severity: Severity::Info,
            status: RuleStatus::Stable,
            note: Some(WEAK_SIGNAL_NOTE),
        }
    }

    /// 未校正のリテラル。
    pub const fn exp_lit(s: &'static str, severity: Severity) -> Self {
        Self {
            pattern: Pattern::Literal(s),
            severity,
            status: RuleStatus::Experimental,
            note: None,
        }
    }

    /// 未校正の正規表現。
    pub const fn exp_re(s: &'static str, severity: Severity) -> Self {
        Self {
            pattern: Pattern::Regex(s),
            severity,
            status: RuleStatus::Experimental,
            note: None,
        }
    }

    /// メッセージに注記を添える (実験的な項目に弱い手掛かりの断り書きを付けるときなど)。
    pub const fn with_note(self, note: &'static str) -> Self {
        Self {
            note: Some(note),
            ..self
        }
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
}

impl Matcher {
    /// `include_experimental` が偽なら、実験的な項目を照合から外す。
    pub fn new(entries: &[Entry], include_experimental: bool) -> Self {
        let mut literal_patterns = Vec::new();
        let mut literal_entries = Vec::new();
        let mut regexes = Vec::new();
        for (idx, entry) in entries.iter().enumerate() {
            if entry.status == RuleStatus::Experimental && !include_experimental {
                continue;
            }
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
        }
    }

    /// 文のテキストから一致を探す。重なる一致は、先に始まり長いほうを 1 件だけ残す。
    pub fn find(&self, text: &str) -> Vec<Hit> {
        let mut hits = Vec::new();
        if let Some(ac) = &self.literals {
            for m in ac.find_iter(text) {
                hits.push(Hit {
                    range: m.start()..m.end(),
                    entry: self.literal_entries[m.pattern().as_usize()],
                });
            }
        }
        for r in &self.regexes {
            if r.grouped {
                // グループを取り出すのは遅いので、グループを持つ正規表現だけにする
                for caps in r.re.captures_iter(text) {
                    if let Some(m) = caps.name(MATCH_GROUP)
                        && m.start() < m.end()
                    {
                        hits.push(Hit {
                            range: m.start()..m.end(),
                            entry: r.entry,
                        });
                    }
                }
            } else {
                for m in r.re.find_iter(text) {
                    if m.start() < m.end() {
                        hits.push(Hit {
                            range: m.start()..m.end(),
                            entry: r.entry,
                        });
                    }
                }
            }
        }
        select_non_overlapping(hits)
    }
}

/// 重なる一致から、先に始まり長いものを優先して重ならない組を選ぶ。
pub(super) fn select_non_overlapping(mut hits: Vec<Hit>) -> Vec<Hit> {
    hits.sort_by(|a, b| {
        a.range
            .start
            .cmp(&b.range.start)
            .then(b.range.end.cmp(&a.range.end))
            .then(a.entry.cmp(&b.entry))
    });
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
    status: RuleStatus,
) -> Diagnostic {
    let quoted = quote(matched);
    let mut d = meta
        .diagnostic(span, message)
        .with_hint(hint)
        .with_context(context)
        .with_metric("item", quoted.clone())
        .with_metric("matched", quoted);
    d.severity = severity;
    d.status = status;
    d
}

/// 語句辞書で動くルール。
pub(super) struct PhraseRule {
    spec: &'static PhraseSpec,
    /// 校正済みの項目だけの照合器。
    stable: Matcher,
    /// 実験的な項目も含む照合器。
    full: Matcher,
}

impl PhraseRule {
    pub fn new(spec: &'static PhraseSpec) -> Self {
        Self {
            spec,
            stable: Matcher::new(spec.entries, false),
            full: Matcher::new(spec.entries, true),
        }
    }
}

impl Rule for PhraseRule {
    fn meta(&self) -> &'static RuleMeta {
        &self.spec.meta
    }

    fn check(&self, ctx: &RuleContext<'_>, out: &mut Vec<Diagnostic>) {
        // ルール自体が実験的なら、有効にされた時点で全項目を使う
        let use_full = self.spec.meta.status == RuleStatus::Experimental || ctx.experimental;
        let matcher = if use_full { &self.full } else { &self.stable };
        for (idx, block) in ctx.scoped_blocks() {
            for sentence in ctx.doc.block_sentences(idx) {
                let text = &block.text[sentence.range.clone()];
                for hit in matcher.find(text) {
                    let entry = &self.spec.entries[hit.entry];
                    let matched = &text[hit.range.clone()];
                    let abs = (sentence.range.start + hit.range.start)
                        ..(sentence.range.start + hit.range.end);
                    let mut message = self.spec.message.replace("{m}", &quote(matched));
                    if let Some(note) = entry.note {
                        message.push_str(&format!("（{note}）"));
                    }
                    out.push(
                        diagnostic(
                            &self.spec.meta,
                            block.to_source(abs),
                            sentence.span,
                            matched,
                            message,
                            self.spec.hint,
                            entry.severity,
                            entry.status,
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
        Entry::exp_lit("でしょう", Severity::Info),
        Entry::re(r"ことが(でき|出来)(る|ます)", Severity::Info),
    ];

    #[test]
    fn prefers_leftmost_longest_and_drops_overlaps() {
        let m = Matcher::new(ENTRIES, true);
        let hits = m.find("これはと言えるでしょう。");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].entry, 1);
    }

    #[test]
    fn experimental_entries_are_excluded_unless_requested() {
        let stable = Matcher::new(ENTRIES, false);
        assert!(stable.find("晴れるでしょう").is_empty());
        let full = Matcher::new(ENTRIES, true);
        assert_eq!(full.find("晴れるでしょう").len(), 1);
    }

    #[test]
    fn regex_and_literal_hits_are_merged_in_order() {
        let m = Matcher::new(ENTRIES, false);
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
            status: RuleStatus::Stable,
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
            RuleStatus::Stable,
        );
        assert_eq!(text_metric(&d, "item"), Some("…を 示す"));
        assert_eq!(text_metric(&d, "matched"), Some("…を 示す"));
    }

    #[test]
    fn a_named_group_narrows_the_match_to_the_group() {
        // 前の文字は条件にだけ使い、指す範囲はグループ m に限る
        const GROUPED: &[Entry] = &[Entry::re(r"(?:^|[^が])(?P<m>大事なのは、)", Severity::Info)];
        let m = Matcher::new(GROUPED, false);
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
        let plain = Matcher::new(ENTRIES, false);
        let text = "読むことができる。";
        assert_eq!(&text[plain.find(text)[0].range.clone()], "ことができる");
    }

    #[test]
    fn notes_can_be_attached_to_experimental_entries() {
        const NOTED: &[Entry] =
            &[Entry::exp_lit("大切です", Severity::Info).with_note(WEAK_SIGNAL_NOTE)];
        assert_eq!(NOTED[0].note, Some(WEAK_SIGNAL_NOTE));
        assert_eq!(NOTED[0].status, RuleStatus::Experimental);
        assert_eq!(NOTED[0].severity, Severity::Info);
    }
}
