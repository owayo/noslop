//! R12 OVERCORRECTION: 直しすぎで生まれる別の均一さ (文長の機械的な交互・体言止めの入れすぎ)。

use crate::diagnostic::{Diagnostic, Lane, RuleStatus, Severity};
use crate::document::Sentence;
use crate::genre::Genre;
use crate::rules::{Fires, Measure, Rule, RuleContext, RuleMeta};

use super::{is_nominal_ending, mean_sd, option_count, option_f64_in, unknown_option};

static META: RuleMeta = RuleMeta {
    id: "R12",
    name: "OVERCORRECTION",
    title: "直しすぎの均一さ",
    lane: Lane::Slop,
    status: RuleStatus::Experimental,
    default_severity: Severity::Info,
    summary: "文の長短が機械的に交互に並ぶか体言止めが 4 割を超え、直し方の型が新しい均一さになっている",
    explanation: EXPLANATION,
};

const EXPLANATION: &str = "\
### 何を見るか

地の文について、次の 2 つをそれぞれ見ます。

- 地の文が 20 文以上あり、隣り合う文の長さの自己相関 (lag-1) が −0.5 以下のもの。長い文と短い文が\
ほぼ交互に並んでいる状態です。
- 地の文が 10 文以上あり、名詞らしい語で終わる文 (体言止め) が 40% 以上を占めるもの。名詞かどうかは \
R06 と同じく、辞書を使わずに文末の文字の種類で推定します。

### なぜ問題か

文の長さがそろいすぎている (R01)、体言止めがない (R06) という指摘に機械的に応えると、今度は「長い文の\
次は必ず短い文」「数文おきに体言止め」という別の型ができます。型を入れ替えただけの文章は、元の均一さと\
同じように、読み手に作為を感じさせます。

### 直し方

文の長さは内容に合わせて決め、長短を交互に並べること自体を目的にしないでください。体言止めは、段落の\
要点を印象づけたい文だけに絞ります。

### 例

- 直す前: 「申請の窓口を一本化した。理由は単純。部署ごとに受付の締め日が違い、差し戻しが月に 40 件近く出ていたからだ。結果は良好。」
- 直した後: 「部署ごとに受付の締め日が違い、差し戻しが月に 40 件近く出ていたので、申請の窓口を一本化した。翌月の差し戻しは 12 件まで減った。」

### 根拠

実験的です。noslop の指摘どおりに直した文章に起こりうる副作用として設けた指標で、閾値 (−0.5 と 0.4) は\
コーパスで校正していません。R01 の校正では、隣り合う文の長さの相関は人の文章と生成された文章を\
見分ける指標として使えませんでした。ここで見るのは判別ではなく、長短がほぼ交互になった極端な場合\
だけです。
";

/// R12 OVERCORRECTION。
pub struct Overcorrection {
    /// (1) 文長の交互を判定する最小の文数。
    alternation_min_sentences: usize,
    /// (1) 隣り合う文の長さの自己相関がこれ以下で指摘する。
    alternation_threshold: f64,
    /// (2) 体言止めの比率を判定する最小の文数。
    nominal_min_sentences: usize,
    /// (2) 体言止めの比率がこれ以上で指摘する。
    nominal_ratio_threshold: f64,
}

impl Overcorrection {
    pub fn new(_genre: Genre) -> Self {
        Self {
            alternation_min_sentences: 20,
            alternation_threshold: -0.5,
            nominal_min_sentences: 10,
            nominal_ratio_threshold: 0.4,
        }
    }

    /// (1) 隣り合う文の長さの自己相関。前提の文数に満たないか、長さがすべて同じなら `None`。
    fn alternation(&self, sentences: &[&Sentence]) -> Option<f64> {
        if sentences.len() < self.alternation_min_sentences {
            return None;
        }
        let lengths: Vec<f64> = sentences.iter().map(|s| s.length as f64).collect();
        lag1_autocorrelation(&lengths)
    }

    /// (2) 体言止めの文。前提の文数に満たなければ `None`。
    fn nominal<'a>(
        &self,
        ctx: &RuleContext<'a>,
        sentences: &[&'a Sentence],
    ) -> Option<Vec<&'a Sentence>> {
        if sentences.len() < self.nominal_min_sentences {
            return None;
        }
        Some(
            sentences
                .iter()
                .copied()
                .filter(|s| is_nominal_ending(ctx.doc.sentence_text(s)))
                .collect(),
        )
    }
}

/// 判定の母集団 (長さのある地の文の文)。
fn prose<'a>(ctx: &RuleContext<'a>) -> Vec<&'a Sentence> {
    ctx.doc.prose_sentences().filter(|s| s.length > 0).collect()
}

/// 隣り合う値の lag-1 自己相関 (標本自己相関)。値がすべて同じなら `None`。
fn lag1_autocorrelation(values: &[f64]) -> Option<f64> {
    let (mean, _) = mean_sd(values)?;
    let denom: f64 = values.iter().map(|v| (v - mean).powi(2)).sum();
    if denom <= 0.0 {
        return None;
    }
    let num: f64 = values
        .windows(2)
        .map(|w| (w[0] - mean) * (w[1] - mean))
        .sum();
    Some(num / denom)
}

impl Rule for Overcorrection {
    fn meta(&self) -> &'static RuleMeta {
        &META
    }

    fn configure(&mut self, key: &str, value: &toml::Value) -> Result<(), String> {
        match key {
            "alternation_min_sentences" => {
                self.alternation_min_sentences = option_count(key, value)?.max(2)
            }
            "alternation_threshold" => {
                self.alternation_threshold = option_f64_in(key, value, -1.0, 1.0)?
            }
            "nominal_min_sentences" => self.nominal_min_sentences = option_count(key, value)?,
            "nominal_ratio_threshold" => {
                self.nominal_ratio_threshold = option_f64_in(key, value, 0.0, 1.0)?
            }
            _ => return Err(unknown_option(&META, key)),
        }
        Ok(())
    }

    fn options(&self) -> Vec<(&'static str, String)> {
        vec![
            (
                "alternation_min_sentences",
                self.alternation_min_sentences.to_string(),
            ),
            (
                "alternation_threshold",
                self.alternation_threshold.to_string(),
            ),
            (
                "nominal_min_sentences",
                self.nominal_min_sentences.to_string(),
            ),
            (
                "nominal_ratio_threshold",
                self.nominal_ratio_threshold.to_string(),
            ),
        ]
    }

    /// (1) 隣り合う文の長さの自己相関を `alternation_threshold` と、(2) 体言止めの比率を
    /// `nominal_ratio_threshold` と比べる値として返す (どちらも前提の文数を満たす文書だけ)。
    /// 体言止めが 1 つもない文書は、比率の閾値をどう変えても (2) で指摘しないので、比率を返さない。
    fn measure(&self, ctx: &RuleContext<'_>) -> Vec<Measure> {
        let sentences = prose(ctx);
        let mut measures = Vec::new();
        if let Some(r) = self.alternation(&sentences) {
            measures.push(Measure::new(
                "length_autocorrelation",
                r,
                "alternation_threshold",
                Fires::AtOrBelow,
            ));
        }
        if let Some(nominal) = self.nominal(ctx, &sentences)
            && !nominal.is_empty()
        {
            measures.push(Measure::new(
                "nominal_ratio",
                nominal.len() as f64 / sentences.len() as f64,
                "nominal_ratio_threshold",
                Fires::AtOrAbove,
            ));
        }
        measures
    }

    fn check(&self, ctx: &RuleContext<'_>, out: &mut Vec<Diagnostic>) {
        let sentences = prose(ctx);
        let n = sentences.len();
        if let Some(r) = self.alternation(&sentences)
            && r <= self.alternation_threshold
        {
            out.push(
                META.diagnostic(
                    sentences[0].span,
                    format!(
                        "地の文 {n} 文で、長い文と短い文がほぼ交互に並んでいます (隣り合う文の長さの自己相関 {r:.2})"
                    ),
                )
                .with_hint("文の長さは内容に合わせて決め、長短を交互に並べること自体を目的にしないでください")
                .with_metric("sentences", n)
                .with_metric("autocorrelation", r)
                .with_metric("threshold", self.alternation_threshold),
            );
        }
        if let Some(nominal) = self.nominal(ctx, &sentences) {
            let ratio = nominal.len() as f64 / n as f64;
            if nominal.is_empty() || ratio < self.nominal_ratio_threshold {
                return;
            }
            out.push(
                META.diagnostic(
                    nominal[0].span,
                    format!(
                        "地の文 {n} 文のうち {} 文 ({:.0}%) が名詞で終わっています",
                        nominal.len(),
                        ratio * 100.0
                    ),
                )
                .with_hint("体言止めは段落の要点を印象づけたい文だけに絞り、ほかは述語で言い切ってください")
                .with_related(nominal.iter().map(|s| s.span).collect())
                .with_metric("sentences", n)
                .with_metric("nominal", nominal.len())
                .with_metric("ratio", ratio)
                .with_metric("threshold", self.nominal_ratio_threshold),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::testing::{matched, measure_md, run};

    fn rule() -> Overcorrection {
        Overcorrection::new(Genre::General)
    }

    /// 指定した長さ (「あ」の数) の、述語で終わる文を並べた段落。
    fn predicate_sentences(lengths: &[usize]) -> String {
        let mut text: String = lengths
            .iter()
            .map(|&n| format!("{}た。", "あ".repeat(n)))
            .collect();
        text.push('\n');
        text
    }

    fn alternating(count: usize) -> Vec<usize> {
        (0..count)
            .map(|i| if i % 2 == 0 { 10 } else { 40 })
            .collect()
    }

    /// 10 文のうち 5 文が体言止め。
    const HALF_NOMINAL: &str = "申請の窓口を一本化した。理由は単純。締め日が部署ごとに違った。結果は良好。差し戻しは減った。次の課題は夜間の対応。担当者も増やした。問い合わせは半分。記入例も添えた。効果は明白。\n";

    /// 10 文のうち 2 文が体言止め。
    const FEW_NOMINAL: &str = "申請の窓口を一本化した。理由は単純。締め日が部署ごとに違った。差し戻しは減った。次の課題は夜間の対応。担当者も増やした。問い合わせは半分に減った。記入例も添えた。効果ははっきり出た。来月も続ける。\n";

    #[test]
    fn lag1_autocorrelation_of_alternating_values() {
        let r = lag1_autocorrelation(&[1.0, 3.0, 1.0, 3.0]).unwrap();
        assert!((r + 0.75).abs() < 1e-9);
        assert!(lag1_autocorrelation(&[2.0, 2.0, 2.0]).is_none());
        assert!(lag1_autocorrelation(&[]).is_none());
    }

    #[test]
    fn flags_mechanical_alternation_of_long_and_short_sentences() {
        let md = predicate_sentences(&alternating(20));
        let d = run(&rule(), &md);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].rule_id, "R12");
        assert!(
            d[0].message
                .contains("地の文 20 文で、長い文と短い文がほぼ交互")
        );
        let m = measure_md(&rule(), &md);
        // 体言止めが 1 つもないので、比率は返さない (閾値を 0 にしても (2) では指摘しない)
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].threshold_key, "alternation_threshold");
        // 完全な交互 20 文の標本自己相関は −19/20
        assert!((m[0].value + 0.95).abs() < 1e-9);
        assert!(m[0].fires_at(-0.5));
        let mut zero = rule();
        zero.configure("nominal_ratio_threshold", &toml::Value::Float(0.0))
            .unwrap();
        zero.configure("alternation_threshold", &toml::Value::Float(-1.0))
            .unwrap();
        assert!(run(&zero, &md).is_empty());
    }

    #[test]
    fn ignores_clustered_lengths_and_short_texts() {
        let clustered: Vec<usize> = (0..20).map(|i| 10 + (i / 5) * 10).collect();
        let md = predicate_sentences(&clustered);
        assert!(run(&rule(), &md).is_empty());
        assert!(measure_md(&rule(), &md)[0].value > 0.0);

        // 19 文では交互を判定せず、体言止めもないので値を返さない
        let md = predicate_sentences(&alternating(19));
        assert!(run(&rule(), &md).is_empty());
        assert!(measure_md(&rule(), &md).is_empty());
    }

    #[test]
    fn flags_too_many_nominal_endings() {
        let d = run(&rule(), HALF_NOMINAL);
        assert_eq!(d.len(), 1);
        assert!(d[0].message.contains("10 文のうち 5 文 (50%)"));
        assert_eq!(matched(HALF_NOMINAL, &d), vec!["理由は単純。"]);
        assert_eq!(d[0].related.len(), 5);
        let m = measure_md(&rule(), HALF_NOMINAL);
        assert_eq!(m.len(), 1);
        assert!((m[0].value - 0.5).abs() < 1e-9);

        assert!(run(&rule(), FEW_NOMINAL).is_empty());
        let nine: String = HALF_NOMINAL.split_inclusive('。').skip(1).collect();
        assert!(run(&rule(), &nine).is_empty());
        assert!(measure_md(&rule(), &nine).is_empty());
    }

    #[test]
    fn configure_overrides_and_rejects_bad_values() {
        let mut r = rule();
        r.configure("nominal_ratio_threshold", &toml::Value::Float(0.2))
            .unwrap();
        assert_eq!(run(&r, FEW_NOMINAL).len(), 1);
        r.configure("alternation_min_sentences", &toml::Value::Integer(4))
            .unwrap();
        r.configure("alternation_threshold", &toml::Value::Float(-0.9))
            .unwrap();
        assert!(run(&r, &predicate_sentences(&alternating(6))).is_empty());
        assert!(
            r.configure("alternation_threshold", &toml::Value::Float(-1.5))
                .is_err()
        );
        assert!(
            r.configure("nominal_ratio_threshold", &toml::Value::Float(1.5))
                .is_err()
        );
        assert!(
            r.configure("nominal_min_sentences", &toml::Value::Integer(0))
                .is_err()
        );
        assert!(r.configure("nope", &toml::Value::Integer(1)).is_err());
        assert_eq!(r.options().len(), 4);
        assert_eq!(
            r.options()[1],
            ("alternation_threshold", "-0.9".to_string())
        );
    }
}
