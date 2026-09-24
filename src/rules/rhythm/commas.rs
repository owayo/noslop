//! R13 COMMA_PROFILE: 地の文の読点の多さと、読点の直前の字の偏り。

use std::cmp::Reverse;
use std::collections::HashMap;

use crate::diagnostic::{Diagnostic, Lane, RuleStatus, Severity};
use crate::document::Sentence;
use crate::genre::Genre;
use crate::rules::{Fires, Measure, Rule, RuleContext, RuleMeta};
use crate::text;

use super::{option_count, option_f64_in, unknown_option};

static META: RuleMeta = RuleMeta {
    id: "R13",
    name: "COMMA_PROFILE",
    title: "読点の使い方",
    lane: Lane::Slop,
    status: RuleStatus::Experimental,
    default_severity: Severity::Info,
    summary: "地の文の読点が 1 文あたり平均 2 個以上 (暫定) あり、読点を打つ癖が文章全体に出ている",
    explanation: EXPLANATION,
};

const EXPLANATION: &str = "\
### 何を見るか

地の文が 20 文以上ある文書で、1 文あたりの読点 (「、」「，」) の数の平均が 2.0 以上なら 1 件出します。\
あわせて読点の直前の 1〜2 字 (「は、」「ため、」など) を数え、多いものから 5 つを metrics の \
`comma_leads` に載せます。直前の字は文字の並びで数えるだけで、品詞は判定しません。

### なぜ問題か

主題の「は」の後や接続の語の後ごとに読点を打つと、文が細かく区切られ、どこが意味の切れ目なのかが\
かえって見えにくくなります。読点の多さは、人が「AI らしい」と感じる特徴のうち実測でも差が確かめられた\
数少ないもので、どこに読点を打つかにも書き手の癖が出ます。

### 直し方

`comma_leads` に出た直前の字を手がかりに、打たなくても読める読点を外してください。意味の切れ目や、\
読み違いを防ぐ位置の読点は残します。

### 例

- 直す前: 「今回は、申請の手順を、担当者ごとに、分けて説明します。」
- 直した後: 「今回は申請の手順を担当者ごとに分けて説明します。」

### 根拠

実験的です。公開の研究で、読点の多さや読点の直前の語の分布は人の文章と生成された文章で異なると\
報告されています。ただし既定の閾値 2.0 は校正前の暫定値で、noslop のコーパスでは校正していません。
";

/// metrics に載せる、読点の直前の字の数。
const TOP_LEADS: usize = 5;

/// R13 COMMA_PROFILE。
pub struct CommaProfile {
    min_sentences: usize,
    /// 1 文あたりの読点の平均がこれ以上で指摘する (校正前の暫定値)。
    per_sentence_threshold: f64,
}

impl CommaProfile {
    pub fn new(_genre: Genre) -> Self {
        Self {
            min_sentences: 20,
            per_sentence_threshold: 2.0,
        }
    }
}

/// 読点の直前の 1〜2 字。ひらがなが続くなら最大 2 字、それ以外は直前の 1 字。
fn comma_lead(before: &str) -> Option<String> {
    let mut chars = before.chars().rev();
    let last = chars.next()?;
    let mut lead = String::new();
    if text::is_hiragana(last)
        && let Some(prev) = chars.next().filter(|&c| text::is_hiragana(c))
    {
        lead.push(prev);
    }
    lead.push(last);
    Some(lead)
}

/// 地の文の読点の集計。
struct Profile<'a> {
    sentences: Vec<&'a Sentence>,
    /// 文ごとの読点の数 (`sentences` と同じ順)。
    counts: Vec<usize>,
    /// 読点の直前の 1〜2 字ごとの (出現数, 初出の順番)。
    leads: HashMap<String, (usize, usize)>,
}

impl<'a> Profile<'a> {
    fn new(ctx: &RuleContext<'a>) -> Self {
        let doc = ctx.doc;
        let sentences: Vec<&Sentence> = doc.prose_sentences().filter(|s| s.length > 0).collect();
        let mut counts = Vec::with_capacity(sentences.len());
        let mut leads: HashMap<String, (usize, usize)> = HashMap::new();
        for s in &sentences {
            let body = doc.sentence_text(s);
            let mut n = 0;
            for (i, c) in body.char_indices() {
                if !text::is_comma(c) {
                    continue;
                }
                n += 1;
                if let Some(lead) = comma_lead(&body[..i]) {
                    let order = leads.len();
                    leads.entry(lead).or_insert((0, order)).0 += 1;
                }
            }
            counts.push(n);
        }
        Profile {
            sentences,
            counts,
            leads,
        }
    }

    fn total(&self) -> usize {
        self.counts.iter().sum()
    }

    fn per_sentence(&self) -> f64 {
        self.total() as f64 / self.sentences.len().max(1) as f64
    }

    /// 読点の直前の字を多い順に `n` 件 (同数なら先に出たもの)。「は、×12 / ため、×5」の形。
    fn top_leads(&self, n: usize) -> String {
        let mut leads: Vec<(&String, &(usize, usize))> = self.leads.iter().collect();
        leads.sort_by_key(|&(_, &(count, order))| (Reverse(count), order));
        leads
            .iter()
            .take(n)
            .map(|(lead, (count, _))| format!("{lead}、×{count}"))
            .collect::<Vec<_>>()
            .join(" / ")
    }
}

impl Rule for CommaProfile {
    fn meta(&self) -> &'static RuleMeta {
        &META
    }

    fn configure(&mut self, key: &str, value: &toml::Value) -> Result<(), String> {
        match key {
            "min_sentences" => self.min_sentences = option_count(key, value)?,
            "per_sentence_threshold" => {
                self.per_sentence_threshold = option_f64_in(key, value, 0.0, 10.0)?
            }
            _ => return Err(unknown_option(&META, key)),
        }
        Ok(())
    }

    fn options(&self) -> Vec<(&'static str, String)> {
        vec![
            ("min_sentences", self.min_sentences.to_string()),
            (
                "per_sentence_threshold",
                self.per_sentence_threshold.to_string(),
            ),
        ]
    }

    /// 地の文が `min_sentences` 以上ある文書で、1 文あたりの読点の平均を
    /// `per_sentence_threshold` と比べる値として返す。読点が 1 つもない文書は、閾値をどう
    /// 変えても指摘しないので値を返さない。
    fn measure(&self, ctx: &RuleContext<'_>) -> Vec<Measure> {
        let p = Profile::new(ctx);
        if p.sentences.len() < self.min_sentences || p.total() == 0 {
            return Vec::new();
        }
        vec![Measure::new(
            "commas_per_sentence",
            p.per_sentence(),
            "per_sentence_threshold",
            Fires::AtOrAbove,
        )]
    }

    fn check(&self, ctx: &RuleContext<'_>, out: &mut Vec<Diagnostic>) {
        let p = Profile::new(ctx);
        let n = p.sentences.len();
        if n < self.min_sentences || p.total() == 0 {
            return;
        }
        let mean = p.per_sentence();
        if mean < self.per_sentence_threshold {
            return;
        }
        // 読点がいちばん多い文 (同数なら先の文) を代表位置にする
        let Some(worst) = (0..n).max_by_key(|&i| (p.counts[i], Reverse(i))) else {
            return;
        };
        out.push(
            META.diagnostic(
                p.sentences[worst].span,
                format!(
                    "地の文 {n} 文の読点が 1 文あたり平均 {mean:.1} 個あります (合計 {} 個)",
                    p.total()
                ),
            )
            .with_hint(
                "読点の直前の字 (comma_leads) を手がかりに、打たなくても読める読点を外してください",
            )
            .with_metric("sentences", n)
            .with_metric("commas", p.total())
            .with_metric("per_sentence", mean)
            .with_metric("threshold", self.per_sentence_threshold)
            .with_metric("max_in_sentence", p.counts[worst])
            .with_metric("comma_leads", p.top_leads(TOP_LEADS)),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostic::Metric;
    use crate::rules::testing::{matched, measure_md, run};

    fn rule() -> CommaProfile {
        CommaProfile::new(Genre::General)
    }

    const HEAVY: &str = "今回は、申請の手順を、担当者ごとに、分けて説明します。";
    const LIGHT: &str = "今回は申請の手順を、担当者ごとに分けて説明します。";

    #[test]
    fn comma_leads_are_one_or_two_characters() {
        assert_eq!(comma_lead("企業では").as_deref(), Some("では"));
        assert_eq!(comma_lead("私は").as_deref(), Some("は"));
        assert_eq!(comma_lead("2024年").as_deref(), Some("年"));
        assert_eq!(comma_lead("さらに").as_deref(), Some("らに"));
        assert_eq!(comma_lead(""), None);
    }

    #[test]
    fn flags_comma_heavy_prose_with_the_leading_characters() {
        let mut md = HEAVY.repeat(19);
        md.push_str("締め日は、毎月、変わります、ご注意を、お願いします。\n");
        let d = run(&rule(), &md);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].rule_id, "R13");
        assert_eq!(d[0].status, RuleStatus::Experimental);
        assert_eq!(
            matched(&md, &d),
            vec!["締め日は、毎月、変わります、ご注意を、お願いします。"]
        );
        assert_eq!(d[0].metrics.get("max_in_sentence"), Some(&Metric::Int(4)));
        assert_eq!(
            d[0].metrics.get("comma_leads"),
            Some(&Metric::Text(
                "は、×20 / を、×20 / とに、×19 / 月、×1 / ます、×1".into()
            ))
        );
    }

    #[test]
    fn stays_quiet_with_few_commas_or_few_sentences() {
        let light = LIGHT.repeat(20) + "\n";
        assert!(run(&rule(), &light).is_empty());
        let short = HEAVY.repeat(19) + "\n";
        assert!(run(&rule(), &short).is_empty());
        assert!(measure_md(&rule(), &short).is_empty());
    }

    #[test]
    fn measures_mean_commas_per_sentence() {
        let md = LIGHT.repeat(10) + &"申請は来週です。".repeat(10) + "\n";
        let m = measure_md(&rule(), &md);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].threshold_key, "per_sentence_threshold");
        assert!((m[0].value - 0.5).abs() < 1e-9);
        assert!(!m[0].fires_at(2.0));
        assert!(run(&rule(), &md).is_empty());

        // 読点が 1 つもない文書は、閾値を 0 にしても指摘しないので値を返さない
        let plain = "申請は来週です。".repeat(20) + "\n";
        assert!(measure_md(&rule(), &plain).is_empty());
        let mut zero = rule();
        zero.configure("per_sentence_threshold", &toml::Value::Float(0.0))
            .unwrap();
        assert!(run(&zero, &plain).is_empty());
    }

    #[test]
    fn configure_overrides_and_rejects_unknown_keys() {
        let mut r = rule();
        r.configure("min_sentences", &toml::Value::Integer(2))
            .unwrap();
        r.configure("per_sentence_threshold", &toml::Value::Float(1.0))
            .unwrap();
        assert_eq!(run(&r, "今回は、説明します。次に、試します。\n").len(), 1);
        assert!(
            r.configure("per_sentence_threshold", &toml::Value::Float(-1.0))
                .is_err()
        );
        assert!(
            r.configure("min_sentences", &toml::Value::Integer(0))
                .is_err()
        );
        assert!(r.configure("nope", &toml::Value::Integer(1)).is_err());
        assert_eq!(
            r.options(),
            vec![
                ("min_sentences", "2".to_string()),
                ("per_sentence_threshold", "1".to_string()),
            ]
        );
    }
}
