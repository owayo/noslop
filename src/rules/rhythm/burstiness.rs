//! R01 LOW_BURSTINESS: 地の文の文長がそろいすぎている。

use crate::diagnostic::{Diagnostic, Lane, Severity, Signal};
use crate::document::Sentence;
use crate::genre::Genre;
use crate::rules::{Fires, Measure, Rule, RuleContext, RuleMeta};

use super::{mean_sd, option_count, option_f64_in, unknown_option};

static META: RuleMeta = RuleMeta {
    id: "R01",
    name: "LOW_BURSTINESS",
    title: "文長の単調さ",
    lane: Lane::Slop,
    signal: Some(Signal::Strong),
    default_severity: Severity::Warning,
    summary: "地の文の文の長さがそろいすぎていて、リズムが機械的に均されている疑いがある",
    explanation: EXPLANATION,
};

const EXPLANATION: &str = "\
### 何を見るか

地の文 (見出し・箇条書き・引用・表を除いた本文の段落) の文をすべて集め、文の長さ \
(空白と文末記号を除いた文字数) のばらつきを burstiness で測ります。「**報告日**: 2026年10月1日」\
のように、ラベルと値を改行で並べた項目の行は地の文に数えません。burstiness は \
(標準偏差 − 平均) ÷ (標準偏差 + 平均) で、−1 に近いほど長さがそろっています。変動係数 \
(標準偏差 ÷ 平均) を −1〜1 の範囲に写したもので、両者は同じ情報を持ちます。

地の文が 20 文以上なら −0.50 未満で警告します。10〜19 文の文書は、同じ −0.50 未満のときに\
参考値として情報を出し、10 文未満は判定しません。

### なぜ問題か

人が書く文章には、言い切るだけの短い文と、条件や経緯を抱えた長い文が混ざります。生成された\
文章は一文に載せる情報量が均され、似た長さの文が同じ調子で並びがちです。読み手はこの均一さを\
「機械が書いたようなリズム」として感じ取ります。

### 直し方

いちばん伝えたい文を思い切って短く言い切り、背景を説明する文はつなげて長くするなど、長短を\
意図して隣り合わせてください。文を削るのではなく、情報の重さに合わせて長さを配分し直します。

### 例

- 直す前: 「新しい検索画面を公開しました。表示の速さを改善しました。絞り込みの条件を増やしました。\
操作の方法は従来と同じです。」
- 直した後: 「新しい検索画面を公開した。表示は以前の半分ほどの時間で終わり、絞り込みには期間と\
担当者の条件が加わった。操作は変わらない。」

### 根拠

校正済みです。文字数で数えた文長で比べています。最初の校正 (人の書いた文書 71 本と生成された文書 \
381 本) では、地の文が 20 文以上の文書で、閾値 −0.38 の誤検知が 1.4%、検出が約 58% でした。人の文書を \
176 本 (一般・技術・業務・随筆・スライド) に増やして測り直すと、−0.38 では 17 本 (9.7%) に反応し、\
誤検知の目安 (5%) を超えました。人の講演・解説・随筆には、似た長さの文を落ち着いて並べる書き方も\
あるためです。閾値を −0.45 にすると人の文書は 7 本 (4.0%) になり、その後に集めた書式を保った人の\
技術記事 130 本でも 3 本 (2.3%) でした。ところが、さらに集めた書式を保った人の一般向けの記事 108 本では \
11 本 (10.2%) に反応し、人の文書 176 本のうち一般向けの 26 本でも 2 本 (7.7%) と、一般向けでは 1 割\
ほどに出ていました。閾値を −0.50 にすると、人の文書は 176 本で 1 本 (0.6%)、書式を保った人の技術記事で \
1 本 (0.8%)、一般向けの記事で 4 本 (3.7%) になり、生成文書は 2025 年頃のモデルなどの 390 本で 59 本 \
(15.1%)、2026 年のモデルの 1,339 本で 154 本 (11.5%) を検出します。−0.45 より検出は半分ほどに減り\
ますが、既定の警告では人の文書への誤検知を抑えることを優先しました。一般向けの記事 108 本は、この閾値を\
決めるのにも使ったので、−0.50 での値は独立した検証の結果ではありません。人の文書の\
標本はほとんどが 20 文以上で、短い文書での誤検知率はまだ測れていません。10〜19 文を参考値に留め、\
10 文未満を判定しないのはそのためです。文長をモーラで数えても判定力はほぼ変わらないことを確かめています。\
隣り合う文の長さの相関 (短い文の後に短い文が続くか) は、校正で予想と逆の傾向が出たため使っていません。

手掛かりの強さは「強い」です。人の文書 440 本と生成文書 390 本で測ると、警告以上の指摘が出た文書は\
人 6 本・生成 59 本でした。
";

const HINT: &str = "いちばん伝えたい文を短く言い切り、背景を説明する文は長くつなげるなど、\
文の長短を意図して並べてください";

/// R01 LOW_BURSTINESS。
pub struct LowBurstiness {
    /// 警告を出す最小の文数。
    min_sentences: usize,
    /// 警告の閾値 (これ未満で警告)。
    threshold: f64,
    /// 参考値 (情報) を出す最小の文数。
    info_min_sentences: usize,
    /// 参考値の閾値 (これ未満で情報)。
    info_threshold: f64,
}

impl LowBurstiness {
    pub fn new(_genre: Genre) -> Self {
        Self {
            min_sentences: 20,
            threshold: -0.50,
            info_min_sentences: 10,
            info_threshold: -0.50,
        }
    }

    /// 文数に応じて使う閾値 (設定キー, 閾値, 重大度)。判定しない文数なら `None`。
    fn path(&self, n: usize) -> Option<(&'static str, f64, Severity)> {
        if n >= self.min_sentences {
            Some(("threshold", self.threshold, Severity::Warning))
        } else if n >= self.info_min_sentences {
            Some(("info_threshold", self.info_threshold, Severity::Info))
        } else {
            None
        }
    }
}

/// 判定の母集団 (地の文の、長さが 0 でない文)。
fn prose<'a>(ctx: &RuleContext<'a>) -> Vec<&'a Sentence> {
    ctx.doc.prose_sentences().filter(|s| s.length > 0).collect()
}

/// 文長の (平均, 標準偏差, burstiness)。平均が 0 以下なら `None`。
fn burstiness_of(sentences: &[&Sentence]) -> Option<(f64, f64, f64)> {
    let lengths: Vec<f64> = sentences.iter().map(|s| s.length as f64).collect();
    let (mean, sd) = mean_sd(&lengths)?;
    if mean <= 0.0 {
        return None;
    }
    Some((mean, sd, (sd - mean) / (sd + mean)))
}

impl Rule for LowBurstiness {
    fn meta(&self) -> &'static RuleMeta {
        &META
    }

    fn configure(&mut self, key: &str, value: &toml::Value) -> Result<(), String> {
        match key {
            "min_sentences" => self.min_sentences = option_count(key, value)?.max(2),
            "threshold" => self.threshold = option_f64_in(key, value, -1.0, 1.0)?,
            "info_min_sentences" => self.info_min_sentences = option_count(key, value)?.max(2),
            "info_threshold" => self.info_threshold = option_f64_in(key, value, -1.0, 1.0)?,
            _ => return Err(unknown_option(&META, key)),
        }
        Ok(())
    }

    fn options(&self) -> Vec<(&'static str, String)> {
        vec![
            ("min_sentences", self.min_sentences.to_string()),
            ("threshold", self.threshold.to_string()),
            ("info_min_sentences", self.info_min_sentences.to_string()),
            ("info_threshold", self.info_threshold.to_string()),
        ]
    }

    /// 文数が `min_sentences` 以上なら `threshold` と、`info_min_sentences` 以上なら
    /// `info_threshold` と比べる burstiness を返す (その文書で使われる閾値だけ)。
    fn measure(&self, ctx: &RuleContext<'_>) -> Vec<Measure> {
        let sentences = prose(ctx);
        let Some((key, _, _)) = self.path(sentences.len()) else {
            return Vec::new();
        };
        let Some((_, _, burstiness)) = burstiness_of(&sentences) else {
            return Vec::new();
        };
        vec![Measure::new("burstiness", burstiness, key, Fires::Below)]
    }

    fn check(&self, ctx: &RuleContext<'_>, out: &mut Vec<Diagnostic>) {
        let sentences = prose(ctx);
        let n = sentences.len();
        let Some((_, limit, severity)) = self.path(n) else {
            return;
        };
        let Some((mean, sd, burstiness)) = burstiness_of(&sentences) else {
            return;
        };
        if burstiness >= limit {
            return;
        }
        let mut message = format!(
            "地の文 {n} 文の長さがそろいすぎています (burstiness {burstiness:.2}、平均 {mean:.1} 字、標準偏差 {sd:.1} 字)"
        );
        if severity == Severity::Info {
            message.push_str("。文数が少ないため参考値です");
        }
        let mut d = META
            .diagnostic(sentences[0].span, message)
            .with_hint(HINT)
            .with_metric("sentences", n)
            .with_metric("mean", mean)
            .with_metric("sd", sd)
            .with_metric("cv", sd / mean)
            .with_metric("burstiness", burstiness)
            .with_metric("threshold", limit);
        d.severity = severity;
        out.push(d);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostic::Metric;
    use crate::rules::testing::run;

    /// 指定した長さ (文字数) の文を並べた段落。
    fn paragraph(lengths: &[usize]) -> String {
        lengths
            .iter()
            .map(|&n| format!("{}。", "文".repeat(n)))
            .collect()
    }

    /// 長さ 10 と 25 を交互に並べた文長 (burstiness ≈ −0.40)。
    fn alternating(count: usize) -> Vec<usize> {
        (0..count)
            .map(|i| if i % 2 == 0 { 10 } else { 25 })
            .collect()
    }

    fn rule() -> LowBurstiness {
        LowBurstiness::new(Genre::General)
    }

    #[test]
    fn warns_on_uniform_lengths_with_enough_sentences() {
        let text: String = (0..22)
            .map(|i| format!("担当者が手順書の{}番目の項目を読んで確かめた。", i % 10))
            .collect();
        let d = run(&rule(), &text);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].severity, Severity::Warning);
        assert_eq!(d[0].rule_id, "R01");
        assert_eq!(d[0].metrics.get("sentences"), Some(&Metric::Int(22)));
        assert!(d[0].message.contains("22 文"));
    }

    #[test]
    fn stays_quiet_when_lengths_vary() {
        let mut lengths = Vec::new();
        for i in 0..24 {
            lengths.push(if i % 3 == 0 {
                4
            } else if i % 3 == 1 {
                60
            } else {
                25
            });
        }
        assert!(run(&rule(), &paragraph(&lengths)).is_empty());
    }

    #[test]
    fn moderate_uniformity_is_not_flagged() {
        // burstiness ≈ −0.40: 人の文書にもよく出る程度のそろい方なので、文数によらず出さない (閾値 −0.50)
        assert!(run(&rule(), &paragraph(&alternating(20))).is_empty());
        assert!(run(&rule(), &paragraph(&alternating(15))).is_empty());
        // 長さ 11 と 23 を交互に並べた文長 (burstiness ≈ −0.47) も、人の一般向けの記事に出るので出さない
        let lengths: Vec<usize> = (0..20).map(|i| if i % 2 == 0 { 11 } else { 23 }).collect();
        assert!(run(&rule(), &paragraph(&lengths)).is_empty());
    }

    #[test]
    fn strong_uniformity_warns_with_twenty_sentences() {
        // 長さ 12 と 20 を交互に並べた文長 (burstiness ≈ −0.60) は 20 文以上で警告
        let lengths: Vec<usize> = (0..20).map(|i| if i % 2 == 0 { 12 } else { 20 }).collect();
        let d = run(&rule(), &paragraph(&lengths));
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].severity, Severity::Warning);
    }

    #[test]
    fn short_documents_get_info_only_when_very_uniform() {
        let d = run(&rule(), &paragraph(&[20; 15]));
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].severity, Severity::Info);
        assert!(d[0].message.contains("参考値"));
    }

    #[test]
    fn fewer_than_ten_sentences_are_not_judged() {
        assert!(run(&rule(), &paragraph(&[20; 9])).is_empty());
    }

    #[test]
    fn ignores_lists_and_headings() {
        let mut md = String::from("# 見出し\n\n");
        for _ in 0..25 {
            md.push_str("- 同じ長さの項目を並べて書いた。\n");
        }
        assert!(run(&rule(), &md).is_empty());
    }

    #[test]
    fn configure_overrides_and_rejects_unknown_keys() {
        let mut r = rule();
        r.configure("threshold", &toml::Value::Float(-0.55))
            .unwrap();
        r.configure("min_sentences", &toml::Value::Integer(15))
            .unwrap();
        // −0.40 は −0.55 より大きいので出ない
        assert!(run(&r, &paragraph(&alternating(20))).is_empty());
        let err = r.configure("nope", &toml::Value::Integer(1)).unwrap_err();
        assert!(err.contains("nope") && err.contains("R01"));
        assert!(r.configure("threshold", &toml::Value::Float(-2.0)).is_err());
        assert!(
            r.configure("threshold", &toml::Value::String("x".into()))
                .is_err()
        );
        assert!(
            r.options()
                .iter()
                .any(|(k, v)| *k == "threshold" && v == "-0.55")
        );
    }
}
