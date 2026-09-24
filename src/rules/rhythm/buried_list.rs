//! R04 BURIED_LIST: 同格の項目が読点でつながれ、一文に埋まっている。

use std::ops::Range;

use crate::diagnostic::{Diagnostic, Lane, RuleStatus, Severity};
use crate::genre::Genre;
use crate::rules::{Fires, Measure, Rule, RuleContext, RuleMeta};
use crate::text;

use super::{is_nounish, option_count, option_f64_in, unknown_option};

static META: RuleMeta = RuleMeta {
    id: "R04",
    name: "BURIED_LIST",
    title: "一文に埋もれた列挙",
    lane: Lane::Readability,
    status: RuleStatus::Experimental,
    default_severity: Severity::Info,
    summary: "同格の項目が読点で 3 つ以上つながれ、列挙が長い一文に埋まっている可能性がある",
    explanation: EXPLANATION,
};

const EXPLANATION: &str = "\
### 何を見るか

読点で区切った区画のうち、名詞らしく終わる区画 (末尾が漢字・カタカナ・英数字。末尾の括弧は\
まとめて飛ばす) が続き、その後にさらに区画がある並びを列挙とみなします。最後の項目は述語に\
溶けるのが普通なので (「A、B、C を行う」の C)、名詞で終わる区画の数に 1 を足して項目数とします。\
項目が 4 つ以上なら 50 字以上の文、3 つなら 80 字以上の文だけを指します。項目の長さが極端に\
違う並び、日付や数値だけの区画、「一方」「現在」のような文頭の副詞的な語、\
「〜へ急ぐ途中」「〜した際」のように時や条件を表す節で終わる区画は列挙とみなしません。括弧の内側の\
読点では区切りません。

### なぜ問題か

同格の項目が一文に埋まっていると、読み手はどこからどこまでが一つの項目かを自分で切り分け\
ながら読むことになります。読点が多いこと自体が問題なのではなく、並列の構造が見えないことが\
読みにくさの原因です。

### 直し方

項目が本当に同格なら箇条書きに開いてください。ただし「**項目**: 説明」の型にはしないでください \
(その型自体が生成された文章の特徴になります)。項目が因果や順序でつながっているなら、箇条書き\
にせず文を分けます。

### 例

- 直す前: 「本機能は、監視対象のサーバーから集めたログの収集、しきい値を超えたかどうかの判定、\
当番の担当者への通知、対応履歴の記録を自動で行います。」
- 直した後: 「本機能は次の作業を自動で行います。」のあとに、ログの収集・しきい値の判定・担当者\
への通知・対応履歴の記録を箇条書きで並べる。

### 根拠

実験的です。品詞で名詞を判定する方式で生成文書 241 本を調べた結果、読点の個数で指す方式は\
文書の 6 割に反応し、中身はほぼ同格の列挙だったため、この形の検出に置き換えました。その調査では\
項目が 4 つ以上の並びはほぼすべて本当の列挙で、3 つの並びは精度が落ちたため、3 項目の場合だけ\
長い文に絞っています。辞書を使わないこの実装は区画の末尾の文字種で名詞を推定しているため、同じ\
精度は保証できません。動詞の連用形で並ぶ手順 (「読み込み、解決し、初期化して」) は拾えません。
";

/// 列挙の項目とみなさない、文頭などに置かれる副詞的な名詞。
const ADVERBIAL_NOUNS: &[&str] = &[
    "一方", "他方", "結果", "現在", "今回", "前回", "実際", "本来", "当初", "当時", "最近", "近年",
    "昨年", "今年", "来年", "今後", "将来", "従来", "通常", "同時", "以上", "以下", "以前", "以後",
    "以降", "結局", "最後", "最初", "原則", "基本", "一般", "反面", "半面", "先日", "昨日", "今日",
    "明日", "毎日", "毎回", "例", "特", "逆", "次", "要",
];

/// 1 項目の長さ (字) の上限。これより長い区画は節であって列挙の項目ではないとみなす。
const MAX_ITEM_CHARS: usize = 30;

/// 名詞で終わっていても、時や条件を表す節の終わり方 (「〜へ急ぐ途中」「〜した際」) なら
/// 列挙の項目とみなさない。短い区画 (「開始前、実施中、終了後」) は項目として残す。
const CLAUSE_SUFFIXES: &[&str] = &[
    "途中", "最中", "場合", "結果", "一方", "反面", "半面", "以来", "限り", "際", "時", "後", "前",
    "間", "頃", "中", "末", "朝", "晩", "夜",
];

/// [`CLAUSE_SUFFIXES`] で節とみなす区画の最小の長さ (字)。
const CLAUSE_MIN_CHARS: usize = 7;

/// R04 BURIED_LIST。
pub struct BuriedList {
    /// 項目が 4 つ以上のときに指す最小の文長。
    min_chars: usize,
    /// 項目が 3 つのときに指す最小の文長。
    min_chars_three_items: usize,
    /// 項目の長さの最大と最小の比の上限 (並列性の条件)。
    max_length_ratio: f64,
}

impl BuriedList {
    pub fn new(_genre: Genre) -> Self {
        Self {
            min_chars: 50,
            min_chars_three_items: 80,
            max_length_ratio: 4.0,
        }
    }

    /// 文の中で最も項目の多い列挙 (区画の範囲, 項目数) を探す。
    fn best_run(&self, sentence: &str) -> Option<(Range<usize>, usize)> {
        let segments = split_segments(sentence);
        if segments.len() < 3 {
            return None;
        }
        let items: Vec<bool> = segments
            .iter()
            .map(|r| is_list_item(&sentence[r.clone()]))
            .collect();
        let mut best: Option<(Range<usize>, usize)> = None;
        let mut run_start: Option<usize> = None;
        for idx in 0..segments.len() {
            if items[idx] {
                let start = *run_start.get_or_insert(idx);
                let has_tail = idx + 1 < segments.len();
                let run_len = idx - start + 1;
                if has_tail && run_len >= 2 && self.parallel(sentence, &segments[start..=idx]) {
                    let count = run_len + 1;
                    if best.as_ref().is_none_or(|(_, n)| count > *n) {
                        let tail = &segments[idx + 1];
                        best = Some((segments[start].start..tail.end, count));
                    }
                }
            } else {
                run_start = None;
            }
        }
        best
    }

    /// 項目の長さがそろっているか (並列の列挙らしいか)。
    fn parallel(&self, sentence: &str, items: &[Range<usize>]) -> bool {
        let lengths: Vec<usize> = items
            .iter()
            .map(|r| sentence[r.clone()].trim().chars().count())
            .collect();
        let (Some(&min), Some(&max)) = (lengths.iter().min(), lengths.iter().max()) else {
            return false;
        };
        min > 0 && (max as f64) / (min as f64) <= self.max_length_ratio
    }
}

/// 読点で区切った区画 (前後の空白を除いた範囲)。
///
/// 括弧の内側の読点では区切らない。括弧の中だけで完結する列挙を、外側の文の列挙と
/// 取り違えないためである。対応の取れない括弧はただの文字として扱う。
fn split_segments(sentence: &str) -> Vec<Range<usize>> {
    let protected = bracket_ranges(sentence);
    let mut out = Vec::new();
    let mut start = 0;
    let mut push = |range: Range<usize>| {
        let s = &sentence[range.clone()];
        let lead = s.len() - s.trim_start().len();
        let trail = s.len() - s.trim_end().len();
        if lead + trail < s.len() {
            out.push((range.start + lead)..(range.end - trail));
        }
    };
    for (i, c) in sentence.char_indices() {
        if text::is_comma(c) && !protected.iter().any(|r| r.contains(&i)) {
            push(start..i);
            start = i + c.len_utf8();
        }
    }
    push(start..sentence.len());
    out
}

/// 対応の取れた括弧の範囲 (開き括弧から閉じ括弧まで)。
fn bracket_ranges(sentence: &str) -> Vec<Range<usize>> {
    let mut stack: Vec<(char, usize)> = Vec::new();
    let mut ranges = Vec::new();
    for (i, c) in sentence.char_indices() {
        if let Some(close) = text::closing_bracket(c) {
            stack.push((close, i));
        } else if text::is_closing_bracket(c)
            && let Some(k) = stack.iter().rposition(|&(close, _)| close == c)
        {
            ranges.push(stack[k].1..i + c.len_utf8());
            stack.truncate(k);
        }
    }
    ranges
}

/// 区画が列挙の項目らしいか (名詞らしく終わり、長すぎず、日付・数値や副詞的な語だけでない)。
fn is_list_item(segment: &str) -> bool {
    let body = strip_trailing_brackets(segment.trim());
    let chars = body.chars().count();
    if body.is_empty() || chars > MAX_ITEM_CHARS {
        return false;
    }
    if ADVERBIAL_NOUNS.contains(&body) || is_numeric_only(body) {
        return false;
    }
    if chars >= CLAUSE_MIN_CHARS && CLAUSE_SUFFIXES.iter().any(|s| body.ends_with(s)) {
        return false;
    }
    body.chars().next_back().is_some_and(is_nounish)
}

/// 末尾の括弧のまとまり (「確認ポイント（前面に出るか）」の括弧部分) を取り除く。
fn strip_trailing_brackets(s: &str) -> &str {
    let mut s = s;
    loop {
        let Some(last) = s.chars().next_back() else {
            return s;
        };
        if !text::is_closing_bracket(last) {
            return s;
        }
        // 同じ種類の開き括弧まで遡る (見つからなければ閉じ括弧だけを落とす)
        let body = &s[..s.len() - last.len_utf8()];
        let mut depth = 1usize;
        let mut cut = None;
        for (i, c) in body.char_indices().rev() {
            if c == last {
                depth += 1;
            } else if text::closing_bracket(c) == Some(last) {
                depth -= 1;
                if depth == 0 {
                    cut = Some(i);
                    break;
                }
            }
        }
        s = match cut {
            Some(i) => body[..i].trim_end(),
            None => body.trim_end(),
        };
    }
}

/// 日付・数値・単位だけでできた区画か (「2024年」「3月」「10%」)。
fn is_numeric_only(s: &str) -> bool {
    s.chars().all(|c| {
        c.is_ascii_digit()
            || matches!(c, '０'..='９')
            || "〇一二三四五六七八九十百千万億兆年月日時分秒%％.．:：/／-－~〜第回件個人円度"
                .contains(c)
    })
}

impl Rule for BuriedList {
    fn meta(&self) -> &'static RuleMeta {
        &META
    }

    fn configure(&mut self, key: &str, value: &toml::Value) -> Result<(), String> {
        match key {
            "min_chars" => self.min_chars = option_count(key, value)?,
            "min_chars_three_items" => self.min_chars_three_items = option_count(key, value)?,
            "max_length_ratio" => {
                self.max_length_ratio = option_f64_in(key, value, 1.0, 100.0)?;
            }
            _ => return Err(unknown_option(&META, key)),
        }
        Ok(())
    }

    fn options(&self) -> Vec<(&'static str, String)> {
        vec![
            ("min_chars", self.min_chars.to_string()),
            (
                "min_chars_three_items",
                self.min_chars_three_items.to_string(),
            ),
            ("max_length_ratio", self.max_length_ratio.to_string()),
        ]
    }

    /// 列挙らしい並びを含む文のうち、最も長い文の文字数を、項目数に応じた閾値と比べる値として返す。
    /// 項目が 4 つ以上の文は `min_chars` と、3 つの文は `min_chars_three_items` と比べる。
    fn measure(&self, ctx: &RuleContext<'_>) -> Vec<Measure> {
        let mut longest_four: Option<usize> = None;
        let mut longest_three: Option<usize> = None;
        for (idx, _) in ctx.scoped_blocks() {
            for s in ctx.doc.block_sentences(idx) {
                if !s.japanese {
                    continue;
                }
                let Some((_, items)) = self.best_run(ctx.doc.sentence_text(s)) else {
                    continue;
                };
                let slot = if items >= 4 {
                    &mut longest_four
                } else {
                    &mut longest_three
                };
                *slot = Some(slot.map_or(s.length, |n| n.max(s.length)));
            }
        }
        let mut out = Vec::new();
        if let Some(n) = longest_four {
            out.push(Measure::new(
                "list_sentence_length_4plus",
                n as f64,
                "min_chars",
                Fires::AtOrAbove,
            ));
        }
        if let Some(n) = longest_three {
            out.push(Measure::new(
                "list_sentence_length_3",
                n as f64,
                "min_chars_three_items",
                Fires::AtOrAbove,
            ));
        }
        out
    }

    fn check(&self, ctx: &RuleContext<'_>, out: &mut Vec<Diagnostic>) {
        for (idx, block) in ctx.scoped_blocks() {
            for s in ctx.doc.block_sentences(idx) {
                if !s.japanese {
                    continue;
                }
                let sentence = ctx.doc.sentence_text(s);
                let Some((range, items)) = self.best_run(sentence) else {
                    continue;
                };
                let needed = if items >= 4 {
                    self.min_chars
                } else {
                    self.min_chars_three_items
                };
                if s.length < needed {
                    continue;
                }
                let span =
                    block.to_source((s.range.start + range.start)..(s.range.start + range.end));
                out.push(
                    META.diagnostic(
                        span,
                        format!(
                            "{items} 個の項目が読点でつながれています。列挙が一文 ({} 字) に埋まっている可能性があります",
                            s.length
                        ),
                    )
                    .with_hint(
                        "同格の項目なら箇条書きに開くと、読み手が並列の関係を組み立て直さずに済みます \
                         (「**項目**: 説明」の型にはしないでください)",
                    )
                    .with_context(s.span)
                    .with_metric("items", items)
                    .with_metric("length", s.length),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::testing::run;

    fn rule() -> BuriedList {
        BuriedList::new(Genre::General)
    }

    #[test]
    fn flags_four_items_in_a_long_sentence() {
        let md = "本機能は、監視対象のサーバーから集めたログの収集、しきい値を超えたかどうかの判定、当番の担当者への通知、対応履歴の記録を自動で行います。\n";
        let d = run(&rule(), md);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].rule_id, "R04");
        assert!(d[0].message.contains("4 個"));
        let hit = &md[d[0].span.range()];
        assert!(hit.starts_with("監視対象"));
        assert!(hit.ends_with("自動で行います。"));
    }

    #[test]
    fn three_items_need_a_longer_sentence() {
        let short = "予算、日程、担当者の確認を行います。\n";
        assert!(run(&rule(), short).is_empty());
        let long = "来月の説明会に向けて、会場の手配を担当する総務部との予算の調整、講師の先生方の移動を含めた日程の調整、当日の受付と誘導を担う担当者の確認を、今週中に終えておく必要があります。\n";
        let d = run(&rule(), long);
        assert_eq!(d.len(), 1);
        assert!(d[0].message.contains("3 個"));
    }

    #[test]
    fn verb_clauses_are_not_items() {
        let md = "設定を読み込み、依存関係を解決し、初期化の処理を済ませてから、サーバーを起動して、最後に動作を確かめる手順を毎回繰り返しています。\n";
        assert!(run(&rule(), md).is_empty());
    }

    #[test]
    fn dates_and_adverbial_nouns_break_the_run() {
        let md = "2024年、3月、10日、現地で始まった取り組みは、参加した多くの企業の協力を得て、ようやく最初の成果を出せる段階まで進んできました。\n";
        assert!(run(&rule(), md).is_empty());
        let md = "一方、結果、現在、私たちはこの計画を見直し、関係者の意見を集めたうえで、来年度の予算の組み方を最初から考え直すことにしました。\n";
        assert!(run(&rule(), md).is_empty());
    }

    #[test]
    fn uneven_items_are_not_parallel() {
        let md = "東京、長い説明が付いた大阪府の北部にある工業地帯の拠点、名古屋、福岡の各拠点で、来月から新しい手順での点検を始めます。\n";
        assert!(run(&rule(), md).is_empty());
    }

    #[test]
    fn commas_inside_brackets_do_not_split() {
        // 括弧の中の読点で区切ると「〜する時」「〜その他」が項目に見えてしまう
        let md = "（その部品を外して一つずつ確かめる時、特に強い違和感を覚えたのだが）その他、あの通りの装置をすべて備えた一台の機械が、最後まで静かに動き続けていた。\n";
        assert!(run(&rule(), md).is_empty());
        assert_eq!(split_segments("A（B、C）、D").len(), 2);
    }

    #[test]
    fn temporal_clauses_are_not_items() {
        let md = "彼は大勢の相手と戦うことになった試合の朝、会場へ誰よりも先に向かって急ぐ途中、たまたま古い神社の前を通りかかって足を止めた。\n";
        assert!(run(&rule(), md).is_empty());
        assert!(is_list_item("開始前"));
        assert!(!is_list_item("会場へ誰よりも先に向かって急ぐ途中"));
    }

    #[test]
    fn trailing_brackets_are_skipped() {
        assert_eq!(
            strip_trailing_brackets("確認ポイント（前面に出るか）"),
            "確認ポイント"
        );
        assert_eq!(strip_trailing_brackets("手順」"), "手順");
        assert!(is_list_item("確認ポイント（前面に出るか）"));
        assert!(!is_list_item("調べてから"));
    }

    #[test]
    fn configure_thresholds() {
        let mut r = rule();
        r.configure("min_chars_three_items", &toml::Value::Integer(10))
            .unwrap();
        assert_eq!(run(&r, "予算、日程、担当者の確認を行います。\n").len(), 1);
        assert!(
            r.configure("max_length_ratio", &toml::Value::Float(0.5))
                .is_err()
        );
        assert!(r.configure("z", &toml::Value::Integer(1)).is_err());
    }
}
