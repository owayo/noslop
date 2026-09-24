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
    default_severity: Severity::Warning,
    summary: "「〜ではなく」「〜だけでなく〜も」の対比が文書全体で繰り返され、構文のリズムが目立っている",
    explanation: EXPLANATION,
};

const EXPLANATION: &str = "\
### 何を見るか

「〜ではなく」「〜だけでなく〜も」という、否定してから肯定へ転じる対比を文書全体で数えます。\
3 回以上あれば、対比の数を対象の総文数で割った比率で重大度を決めます。比率が 2% 未満なら情報、\
3% 未満なら警告、それ以上なら重大です (技術文書は 4.5% 以上で重大)。

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

校正済みです。回数だけで重く判定すると、長い文書では薄い頻度でも指摘が並び、質の高い書き手の\
修辞にも反応していました。人の書いた質の高い文書では、3 回以上使っている文書でも総文数に対する\
比率の中央値は 1.5%、最大でも 4.6% でした。生成された文書では中央値 8.3%、最小でも 2.65% で、\
両者の分布はほとんど重なりません。2%・3% の区切りで人の文書が重大になる割合は 4.9% です。技術文書\
では人の記事の 11% が重大になったため、重大の区切りを 4.5% に緩めています。
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
    /// この比率未満なら情報。
    info_below: f64,
    /// この比率以上なら重大。
    error_above: f64,
}

impl AntithesisRepetition {
    pub fn new(genre: Genre) -> Self {
        let error_above = match genre {
            Genre::Tech => 0.045,
            Genre::General | Genre::Business | Genre::Essay => 0.03,
        };
        Self {
            min_count: 3,
            info_below: 0.02,
            error_above,
        }
    }

    fn severity(&self, ratio: f64) -> Severity {
        if ratio < self.info_below {
            Severity::Info
        } else if ratio < self.error_above {
            Severity::Warning
        } else {
            Severity::Error
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
            "info_below" => self.info_below = option_f64_in(key, value, 0.0, 1.0)?,
            "error_above" => self.error_above = option_f64_in(key, value, 0.0, 1.0)?,
            _ => return Err(unknown_option(&META, key)),
        }
        Ok(())
    }

    fn options(&self) -> Vec<(&'static str, String)> {
        vec![
            ("min_count", self.min_count.to_string()),
            ("info_below", self.info_below.to_string()),
            ("error_above", self.error_above.to_string()),
        ]
    }

    /// 校正の基準は重大。元の校正は「人の文書で重大になる割合」(4.9%) で区切りを決めた。
    fn calibration_basis(&self) -> Severity {
        Severity::Error
    }

    /// 対比の回数を `min_count` と比べる値として返す。回数が `min_count` 以上で、総文数比が
    /// `info_below` 以上の文書では、重大になるかを決める総文数比も `error_above` と比べる値
    /// (重大への切り替え) として返す。総文数比が `info_below` 未満の文書は `error_above` を
    /// どう変えても情報のままなので、総文数比を返さない。`info_below` は情報と警告の境目で、
    /// 見直しの基準 (重大) に関わらないので返さない。
    fn measure(&self, ctx: &RuleContext<'_>) -> Vec<Measure> {
        let (hits, total_sentences) = collect_hits(ctx);
        let count = hits.len();
        let mut out = vec![Measure::new(
            "count",
            count as f64,
            "min_count",
            Fires::AtOrAbove,
        )];
        let ratio = ratio_of(count, total_sentences);
        if count >= self.min_count && ratio >= self.info_below {
            out.push(
                Measure::new("ratio", ratio, "error_above", Fires::AtOrAbove).at(Severity::Error),
            );
        }
        out
    }

    fn check(&self, ctx: &RuleContext<'_>, out: &mut Vec<Diagnostic>) {
        let (hits, total_sentences) = collect_hits(ctx);
        if hits.len() < self.min_count {
            return;
        }
        let count = hits.len();
        let ratio = ratio_of(count, total_sentences);
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
    use crate::rules::testing::{Options, matched, measure_md, run, run_with};

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

    #[test]
    fn severity_follows_the_ratio() {
        // 3/200 = 1.5% → 情報
        let d = run(&rule(Genre::General), &text_with(200));
        assert_eq!(d.len(), 3);
        assert!(d.iter().all(|d| d.severity == Severity::Info));
        // 3/120 = 2.5% → 警告
        let d = run(&rule(Genre::General), &text_with(120));
        assert!(d.iter().all(|d| d.severity == Severity::Warning));
        // 3/10 = 30% → 重大
        let d = run(&rule(Genre::General), &text_with(10));
        assert!(d.iter().all(|d| d.severity == Severity::Error));
        assert_eq!(d[0].related.len(), 3);
    }

    #[test]
    fn tech_genre_has_a_higher_error_threshold() {
        // 3/75 = 4% → 一般なら重大、技術文書なら警告
        let md = text_with(75);
        assert!(
            run(&rule(Genre::General), &md)
                .iter()
                .all(|d| d.severity == Severity::Error)
        );
        let tech = Options {
            genre: Genre::Tech,
            ..Options::default()
        };
        assert!(
            run_with(&rule(Genre::Tech), &md, tech)
                .iter()
                .all(|d| d.severity == Severity::Warning)
        );
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

    #[test]
    fn measures_the_count_and_the_ratio_that_switches_to_error() {
        let r = rule(Genre::General);
        // 3/200 = 1.5%: 情報のままなので、重大への切り替えの値は返さない
        let m = measure_md(&r, &text_with(200));
        assert_eq!(m.len(), 1);
        assert_eq!((m[0].threshold_key, m[0].value), ("min_count", 3.0));
        assert_eq!(m[0].switches_to, None);
        // 3/120 = 2.5%: 警告。error_above と比べる値を返す
        let m = measure_md(&r, &text_with(120));
        assert_eq!(m.len(), 2);
        assert_eq!(m[1].threshold_key, "error_above");
        assert_eq!(m[1].switches_to, Some(Severity::Error));
        assert!((m[1].value - 0.025).abs() < 1e-9);
        assert!(!m[1].fires_at(0.03));
        // 回数が足りない文書は回数だけ
        let m = measure_md(&r, "これは通知ではなく、仕組みだ。\n");
        assert_eq!(m.len(), 1);
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
        assert!(
            run(&r, &text_with(10))
                .iter()
                .all(|d| d.severity == Severity::Warning)
        );
        assert!(r.configure("info_below", &toml::Value::Float(2.0)).is_err());
        assert!(r.configure("unknown", &toml::Value::Float(0.1)).is_err());
    }
}
