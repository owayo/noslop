//! R05 ANTITHESIS_REPETITION: 否定→肯定の対比 (「〜ではなく」) が繰り返される。

use std::sync::LazyLock;

use regex::Regex;

use crate::diagnostic::{Diagnostic, Lane, RuleStatus, Severity, Span};
use crate::genre::Genre;
use crate::rules::{Fires, Measure, Rule, RuleContext, RuleMeta};

use super::{option_count, option_f64_in, unknown_option};

static META: RuleMeta = RuleMeta {
    id: "R05",
    name: "ANTITHESIS_REPETITION",
    title: "否定→肯定の対比の反復",
    lane: Lane::Slop,
    status: RuleStatus::Stable,
    default_severity: Severity::Error,
    summary: "「〜ではなく」「〜だけでなく〜も」の対比が文書全体で繰り返され、構文のリズムが目立っている",
    explanation: EXPLANATION,
};

const EXPLANATION: &str = "\
### 何を見るか

「〜ではなく」「〜だけでなく〜も」という、否定してから肯定へ転じる対比を文書全体で数え、\
対比の数を対象の総文数で割った比率を求めます。対比が 3 回以上あり、比率が 2% 以上の文書で、\
対比を 1 件ずつ指します。重大度は比率で決め、5% 以上なら重大、5% 未満なら情報です。比率が 2% \
未満の文書は、回数が多くても指摘しません。区切りはジャンルによらず同じです。設定で `info_below` を\
指定したときだけ、その比率以上・重大未満を警告にします (既定では指定しません)。

### なぜ問題か

誤解を先に打ち消してから本題を示す対比は、ここぞという場面で使えば効きます。生成された文章は\
これを強調の型として繰り返し、言い換えにすぎない箇所にまで使います。同じ型が続くと、読み手は\
内容より構文のリズムに気づき始めます。

### 直し方

読み手が本当に誤解していそうな箇所の対比だけを残してください。残すか迷ったら、前半の否定を消して\
後半だけを読んでみます。それで言いたいことが立つなら否定は要りません。否定と組になって初めて強く\
見える対比は、素直な肯定文か、数字や実例に置き換えます。

### 例

- 直す前: 「この仕組みは単なる通知ではなく、業務の流れそのものを変える基盤だ。」
- 直した後: 「この仕組みは、承認の止まった申請を自動で差し戻す。担当者は毎朝の確認をしなくて\
よくなる。」

### 根拠

校正済みです。noslop のコーパス (人の文書 176 本、生成文書 1,729 本) で、対比が 3 回以上ある文書の\
比率を測りました。人の文書は 26 本が当たり、2% 未満が 11 本、2〜5% が 13 本、5% 以上が 2 本です。\
生成文書は 79 本が当たり、2% 未満が 3 本、2〜5% が 18 本、5% 以上が 58 本でした。2% 未満の文書は\
ほとんどが人の長い文書なので、指摘しません。2〜5% の文書は人の文書のほうが割合が高い (人 176 本中 \
13 本、生成 1,729 本中 18 本) ので、情報に留めます。5% 以上で重大になるのは、人の文書で 176 本中 \
2 本 (1.1%)、生成文書で 1,729 本中 58 本です。技術文書でも 5% 以上になった人の文書は 50 本中 1 本で、\
区切りはジャンルによらずそろえています。
";

/// 「ではなく」(直後の 30 字まで) と「だけでなく〜も」。
///
/// 数え方は校正のときと同じにする (「ではなく」は直後 30 字までを 1 回のヒットとして消費する)。
static NEGATION_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"ではなく、?.{0,30}").expect("antithesis regex"));
static NOT_ONLY_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"だけでなく.{0,10}も").expect("not-only regex"));

/// R05 ANTITHESIS_REPETITION。
pub struct AntithesisRepetition {
    /// 発火する最小の回数。
    min_count: usize,
    /// 指摘する最小の比率 (これ未満の文書は指摘しない)。
    min_ratio: f64,
    /// この比率以上なら重大 (未満なら情報)。
    error_above: f64,
    /// 設定で指定したときだけ、この比率以上・`error_above` 未満を警告にする。
    ///
    /// 以前の既定 (2〜3% を警告) の設定ファイルをそのまま読めるように残している。既定では
    /// 指定がなく、警告の帯はない。校正では動かさない (`measure` は値を返さない)。
    info_below: Option<f64>,
}

impl AntithesisRepetition {
    pub fn new(_genre: Genre) -> Self {
        Self {
            min_count: 3,
            min_ratio: 0.02,
            error_above: 0.05,
            info_below: None,
        }
    }

    fn severity(&self, ratio: f64) -> Severity {
        if ratio >= self.error_above {
            Severity::Error
        } else if self.info_below.is_some_and(|below| ratio >= below) {
            Severity::Warning
        } else {
            Severity::Info
        }
    }
}

/// 1 件のヒット (原文上の指摘範囲と、それを含む文の範囲)。
struct Hit {
    span: Span,
    context: Span,
}

/// 対象のブロックのヒット (出現順) と、対象のブロックの総文数。
fn collect_hits(ctx: &RuleContext<'_>) -> (Vec<Hit>, usize) {
    let doc = ctx.doc;
    let mut total_sentences = 0usize;
    let mut hits: Vec<Hit> = Vec::new();
    for (idx, block) in ctx.scoped_blocks() {
        let sentences = doc.block_sentences(idx);
        total_sentences += sentences.len();
        let context_of = |pos: usize| {
            sentences
                .iter()
                .find(|s| s.range.start <= pos && pos < s.range.end)
                .map(|s| s.span)
        };
        for m in NEGATION_RE.find_iter(&block.text) {
            // 指摘は「ではなく」の 4 字だけに当てる (直後の 30 字は数え方のための消費)
            let end = m.start() + "ではなく".len();
            let span = block.to_source(m.start()..end);
            hits.push(Hit {
                span,
                context: context_of(m.start()).unwrap_or(span),
            });
        }
        for m in NOT_ONLY_RE.find_iter(&block.text) {
            let span = block.to_source(m.range());
            hits.push(Hit {
                span,
                context: context_of(m.start()).unwrap_or(span),
            });
        }
    }
    hits.sort_by_key(|h| h.span.start);
    (hits, total_sentences)
}

/// ヒット数の総文数に対する比率。
fn ratio_of(count: usize, total_sentences: usize) -> f64 {
    if total_sentences == 0 {
        0.0
    } else {
        count as f64 / total_sentences as f64
    }
}

impl Rule for AntithesisRepetition {
    fn meta(&self) -> &'static RuleMeta {
        &META
    }

    fn configure(&mut self, key: &str, value: &toml::Value) -> Result<(), String> {
        match key {
            "min_count" => self.min_count = option_count(key, value)?,
            "min_ratio" => self.min_ratio = option_f64_in(key, value, 0.0, 1.0)?,
            "error_above" => self.error_above = option_f64_in(key, value, 0.0, 1.0)?,
            "info_below" => self.info_below = Some(option_f64_in(key, value, 0.0, 1.0)?),
            _ => return Err(unknown_option(&META, key)),
        }
        Ok(())
    }

    fn options(&self) -> Vec<(&'static str, String)> {
        let mut options = vec![
            ("min_count", self.min_count.to_string()),
            ("min_ratio", self.min_ratio.to_string()),
            ("error_above", self.error_above.to_string()),
        ];
        if let Some(below) = self.info_below {
            options.push(("info_below", below.to_string()));
        }
        options
    }

    /// 校正の基準は重大。人の文書で重大になる割合で区切り (`error_above`) を決めている。
    fn calibration_basis(&self) -> Severity {
        Severity::Error
    }

    /// 指摘するのは、対比の回数が `min_count` 以上で、総文数比が `min_ratio` 以上の文書。
    /// 2 つの条件のどちらか一方の閾値を動かしたときの結果が測定値から分かるよう、それぞれの値は
    /// もう一方の条件を満たす文書でだけ返す。
    ///
    /// - 回数 (`min_count` と比べる): 対比が 1 回以上あり、総文数比が `min_ratio` 以上の文書
    /// - 総文数比 (`min_ratio` と比べる): 回数が `min_count` 以上の文書
    /// - 総文数比 (`error_above` と比べる、重大への切り替え): 指摘する文書 (情報か重大かを決める)
    fn measure(&self, ctx: &RuleContext<'_>) -> Vec<Measure> {
        let (hits, total_sentences) = collect_hits(ctx);
        let count = hits.len();
        let ratio = ratio_of(count, total_sentences);
        let enough = count >= self.min_count;
        let dense = ratio >= self.min_ratio;
        let mut out = Vec::new();
        if count > 0 && dense {
            out.push(Measure::new(
                "count",
                count as f64,
                "min_count",
                Fires::AtOrAbove,
            ));
        }
        if enough {
            out.push(Measure::new("ratio", ratio, "min_ratio", Fires::AtOrAbove));
        }
        if enough && dense {
            out.push(
                Measure::new("ratio", ratio, "error_above", Fires::AtOrAbove).at(Severity::Error),
            );
        }
        out
    }

    fn check(&self, ctx: &RuleContext<'_>, out: &mut Vec<Diagnostic>) {
        let (hits, total_sentences) = collect_hits(ctx);
        let count = hits.len();
        let ratio = ratio_of(count, total_sentences);
        if count < self.min_count || ratio < self.min_ratio {
            return;
        }
        let severity = self.severity(ratio);
        let related: Vec<Span> = hits.iter().map(|h| h.span).collect();
        for hit in &hits {
            let mut d = META
                .diagnostic(
                    hit.span,
                    format!(
                        "否定→肯定の対比が文書全体で {count} 回あります (総文数 {total_sentences} の {:.1}%)",
                        ratio * 100.0
                    ),
                )
                .with_hint(
                    "読み手の誤解を本当に正している対比だけを残し、言い換えにすぎない対比は\
                     素直な肯定文か具体例に書き換えてください",
                )
                .with_context(hit.context)
                .with_related(related.clone())
                .with_metric("count", count)
                .with_metric("sentences", total_sentences)
                .with_metric("ratio", ratio);
            d.severity = severity;
            out.push(d);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::rules::testing::{
        Options, assert_measures_agree, matched, measure_md, run, run_with,
    };

    /// 対比を 3 回含み、総文数が `total` の本文。
    ///
    /// 「ではなく」は直後 30 字までを 1 回として消費するので、対比の文は段落を分けて置く。
    fn text_with(total: usize) -> String {
        let mut s = String::new();
        s.push_str("これは通知ではなく、仕組みだ。\n\n");
        s.push_str("手順ではなく、考え方を示す。\n\n");
        s.push_str("速さだけでなく、正確さも求められる。\n\n");
        for i in 3..total {
            s.push_str(&format!("補足の文をここに{i}つ目として置いた。"));
        }
        s.push('\n');
        s
    }

    fn rule(genre: Genre) -> AntithesisRepetition {
        AntithesisRepetition::new(genre)
    }

    #[test]
    fn needs_three_hits() {
        let md = "これは通知ではなく、仕組みだ。手順ではなく、考え方を示す。\n";
        assert!(run(&rule(Genre::General), md).is_empty());
    }

    /// 指摘の重大度 (指摘がなければ空)。3 件とも同じ重大度であることも確かめる。
    fn severities(d: &[Diagnostic]) -> Vec<Severity> {
        let mut s: Vec<Severity> = d.iter().map(|d| d.severity).collect();
        s.dedup();
        s
    }

    #[test]
    fn severity_follows_the_ratio() {
        let r = rule(Genre::General);
        // 3/200 = 1.5%: 下限 (2%) 未満なので、回数が足りていても指摘しない
        assert!(run(&r, &text_with(200)).is_empty());
        // 3/150 = 2%: 下限ちょうどから情報
        assert_eq!(severities(&run(&r, &text_with(150))), [Severity::Info]);
        // 3/120 = 2.5%・3/61 ≈ 4.9%: 情報 (警告の帯はない)
        assert_eq!(severities(&run(&r, &text_with(120))), [Severity::Info]);
        assert_eq!(severities(&run(&r, &text_with(61))), [Severity::Info]);
        // 3/60 = 5%: 重大の区切りちょうどから重大
        assert_eq!(severities(&run(&r, &text_with(60))), [Severity::Error]);
        // 3/10 = 30% → 重大
        let d = run(&r, &text_with(10));
        assert_eq!(severities(&d), [Severity::Error]);
        assert_eq!(d.len(), 3);
        assert_eq!(d[0].related.len(), 3);
    }

    #[test]
    fn every_genre_uses_the_same_thresholds() {
        for genre in [Genre::General, Genre::Tech, Genre::Business, Genre::Essay] {
            let options = Options {
                genre,
                ..Options::default()
            };
            let at = |total: usize| severities(&run_with(&rule(genre), &text_with(total), options));
            // 3/75 = 4% は情報、3/50 = 6% は重大
            assert_eq!(at(75), [Severity::Info], "{genre:?}");
            assert_eq!(at(50), [Severity::Error], "{genre:?}");
            assert!(at(200).is_empty(), "{genre:?}");
        }
    }

    #[test]
    fn spans_point_at_the_contrast() {
        let md = text_with(10);
        let d = run(&rule(Genre::General), &md);
        assert_eq!(
            matched(&md, &d),
            vec!["ではなく", "ではなく", "だけでなく、正確さも"]
        );
        assert_eq!(
            &md[d[0].context.unwrap().range()],
            "これは通知ではなく、仕組みだ。"
        );
    }

    #[test]
    fn lists_are_outside_the_default_scope() {
        let md = "- 通知ではなく仕組み\n- 手順ではなく考え方\n- 速さだけでなく正確さも\n";
        assert!(run(&rule(Genre::General), md).is_empty());
    }

    /// 測定値の (設定キー, 値, 切り替え先の重大度)。
    fn keys(m: &[Measure]) -> Vec<(&'static str, f64, Option<Severity>)> {
        m.iter()
            .map(|m| (m.threshold_key, m.value, m.switches_to))
            .collect()
    }

    #[test]
    fn measures_each_condition_where_the_other_one_holds() {
        let r = rule(Genre::General);
        // 3/200 = 1.5%: 回数は足りるが下限未満。回数を動かしても指摘しないので、比率だけを返す
        let m = measure_md(&r, &text_with(200));
        assert_eq!(keys(&m), [("min_ratio", 0.015, None)]);
        assert!(!m[0].fires_at(0.02) && m[0].fires_at(0.015));
        // 3/120 = 2.5%: 回数・下限との比率・重大への切り替えの比率の 3 つ
        let m = measure_md(&r, &text_with(120));
        assert_eq!(
            keys(&m),
            [
                ("min_count", 3.0, None),
                ("min_ratio", 0.025, None),
                ("error_above", 0.025, Some(Severity::Error)),
            ]
        );
        assert!(!m[2].fires_at(0.05) && m[2].fires_at(0.025));
        // 1/1 = 100%: 下限は越えるが回数が足りない。比率の値は返さない
        let m = measure_md(&r, "これは通知ではなく、仕組みだ。\n");
        assert_eq!(keys(&m), [("min_count", 1.0, None)]);
        // 対比のない文書は、どの閾値でも指摘しないので何も返さない
        assert!(measure_md(&r, "仕組みを説明する。\n").is_empty());
        assert_eq!(r.calibration_basis(), Severity::Error);
    }

    #[test]
    fn configure_thresholds() {
        let mut r = rule(Genre::General);
        r.configure("min_count", &toml::Value::Integer(4)).unwrap();
        assert!(run(&r, &text_with(10)).is_empty());
        r.configure("min_count", &toml::Value::Integer(3)).unwrap();
        r.configure("error_above", &toml::Value::Float(0.5))
            .unwrap();
        assert_eq!(severities(&run(&r, &text_with(10))), [Severity::Info]);
        // 下限を 1% に下げると、1.5% の文書も情報として指摘する
        r.configure("min_ratio", &toml::Value::Float(0.01)).unwrap();
        assert_eq!(severities(&run(&r, &text_with(200))), [Severity::Info]);
        assert!(r.configure("min_ratio", &toml::Value::Float(1.5)).is_err());
        assert!(
            r.configure("error_above", &toml::Value::Float(2.0))
                .is_err()
        );
        assert!(r.configure("unknown", &toml::Value::Float(0.1)).is_err());
        assert_eq!(
            r.options().iter().map(|(k, _)| *k).collect::<Vec<_>>(),
            ["min_count", "min_ratio", "error_above"]
        );
    }

    #[test]
    fn info_below_still_configures_a_warning_band() {
        // 以前の設定ファイルの `info_below` は、指定したときだけ警告の帯を作る (既定では帯がない)
        let mut r = rule(Genre::General);
        r.configure("error_above", &toml::Value::Float(0.5))
            .unwrap();
        assert_eq!(severities(&run(&r, &text_with(10))), [Severity::Info]);
        r.configure("info_below", &toml::Value::Float(0.2)).unwrap();
        assert_eq!(severities(&run(&r, &text_with(10))), [Severity::Warning]);
        assert!(r.configure("info_below", &toml::Value::Float(1.5)).is_err());
        assert_eq!(
            r.options().iter().map(|(k, _)| *k).collect::<Vec<_>>(),
            ["min_count", "min_ratio", "error_above", "info_below"]
        );
        let make = || -> Box<dyn Rule> {
            let mut r = rule(Genre::General);
            r.configure("error_above", &toml::Value::Float(0.5))
                .unwrap();
            r.configure("info_below", &toml::Value::Float(0.2)).unwrap();
            Box::new(r)
        };
        let docs = [
            Document::markdown(text_with(10)),
            Document::markdown(text_with(40)),
        ];
        assert_measures_agree(make, &docs);
    }
}
