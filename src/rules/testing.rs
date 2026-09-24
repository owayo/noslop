//! ルールのテスト用の近道。

use std::collections::HashMap;

use crate::diagnostic::{Diagnostic, Severity};
use crate::document::Document;
use crate::genre::Genre;
use crate::rules::{Measure, Rule, RuleContext, Scope};

/// ルールの実行条件。
#[derive(Debug, Clone, Copy)]
pub struct Options {
    pub genre: Genre,
    pub scope: Scope,
    pub experimental: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            genre: Genre::General,
            scope: Scope::default(),
            experimental: false,
        }
    }
}

/// Markdown に 1 ルールを既定の条件で当てる。
pub fn run(rule: &dyn Rule, markdown: &str) -> Vec<Diagnostic> {
    run_with(rule, markdown, Options::default())
}

/// Markdown に 1 ルールを指定の条件で当てる。
pub fn run_with(rule: &dyn Rule, markdown: &str, options: Options) -> Vec<Diagnostic> {
    let doc = Document::markdown(markdown);
    run_doc(rule, &doc, options)
}

/// 読み込み済みの文書に 1 ルールを当てる。
pub fn run_doc(rule: &dyn Rule, doc: &Document, options: Options) -> Vec<Diagnostic> {
    let ctx = RuleContext {
        doc,
        genre: options.genre,
        scope: options.scope,
        experimental: options.experimental,
    };
    let mut out = Vec::new();
    rule.check(&ctx, &mut out);
    out
}

/// 診断の指摘箇所の原文 (Markdown の原文から切り出したもの)。
pub fn matched<'a>(markdown: &'a str, diags: &[Diagnostic]) -> Vec<&'a str> {
    diags.iter().map(|d| &markdown[d.span.range()]).collect()
}

/// Markdown に 1 ルールの測定値 (`Rule::measure`) を既定の条件で求める。
pub fn measure_md(rule: &dyn Rule, markdown: &str) -> Vec<Measure> {
    let doc = Document::markdown(markdown);
    rule.measure(&RuleContext::new(&doc))
}

// ---------------------------------------------------------------------------
// 校正用の測定値 (`Rule::measure`) と判定の一致
// ---------------------------------------------------------------------------

/// 現在の設定値のうち、数値として読めるもの (`noslop calibrate` が閾値として扱うもの)。
pub fn numeric_options(rule: &dyn Rule) -> HashMap<&'static str, f64> {
    rule.options()
        .into_iter()
        .filter_map(|(k, v)| v.parse().ok().map(|v| (k, v)))
        .collect()
}

/// 測定値が現在の閾値を越えるか。設定キーが `options()` になければ失敗させる。
fn fires_now(rule: &dyn Rule, current: &HashMap<&'static str, f64>, m: &Measure) -> bool {
    let t = current.get(m.threshold_key).unwrap_or_else(|| {
        panic!(
            "{}: 設定キー {} が options() に数値としてありません",
            rule.meta().id,
            m.threshold_key
        )
    });
    m.fires_at(*t)
}

/// 「測定値が現在の閾値を越える ⇔ check が指摘する」を確かめ、測定値を返す。
///
/// 指摘するかどうかを決める測定値 (`switches_to` が `None`) は、どれか 1 つが閾値を越えることと、
/// 指摘が 1 件以上出ることが一致するかを見る。重大度の切り替えを決める測定値 (`switches_to` が
/// `Some(X)`) は、閾値を越えることと、X 以上の指摘が出ることが一致するかを見る。
///
/// `switches` には、そのルールが使う切り替え先の重大度を渡す。この文書に切り替えの測定値が
/// なくても、渡した重大度は「切り替わらない」として確かめる (`noslop calibrate` は値のない文書を
/// 指摘されない文書として数えるため、X 以上の指摘が出る文書で値が欠けると率が食い違う)。
pub fn assert_measure_consistent(
    rule: &dyn Rule,
    doc: &Document,
    switches: &[Severity],
    label: &str,
) -> Vec<Measure> {
    let ctx = RuleContext::new(doc);
    let measures = rule.measure(&ctx);
    let current = numeric_options(rule);
    let mut diagnostics = Vec::new();
    rule.check(&ctx, &mut diagnostics);
    let id = rule.meta().id;

    let flagged = measures
        .iter()
        .filter(|m| m.switches_to.is_none())
        .any(|m| fires_now(rule, &current, m));
    assert_eq!(
        flagged,
        !diagnostics.is_empty(),
        "{id} ({label}): 測定値 {measures:?} / 設定 {:?} / 指摘 {} 件",
        rule.options(),
        diagnostics.len()
    );
    let mut severities: Vec<Severity> = switches.to_vec();
    severities.extend(measures.iter().filter_map(|m| m.switches_to));
    severities.sort();
    severities.dedup();
    for severity in severities {
        let by_measure = measures
            .iter()
            .filter(|m| m.switches_to == Some(severity))
            .any(|m| fires_now(rule, &current, m));
        let by_check = diagnostics.iter().any(|d| d.severity >= severity);
        assert_eq!(
            by_measure,
            by_check,
            "{id} ({label}): {} への切り替え。測定値 {measures:?} / 設定 {:?} / 重大度 {:?}",
            severity.as_str(),
            rule.options(),
            diagnostics.iter().map(|d| d.severity).collect::<Vec<_>>()
        );
    }
    measures
}

/// 測定値の前後に閾値を置き換える値 (整数の閾値は整数で)。
fn nudged(value: f64, integer: bool) -> Vec<toml::Value> {
    if integer {
        let v = value as i64;
        [v - 1, v, v + 1]
            .into_iter()
            .filter(|&x| x >= 0)
            .map(toml::Value::Integer)
            .collect()
    } else {
        [value - 1e-6, value, value + 1e-6]
            .into_iter()
            .map(toml::Value::Float)
            .collect()
    }
}

/// [`assert_measures_agree`] が確かめた文書の内訳。
#[derive(Debug, Default)]
pub struct Agreement {
    /// 現在の閾値で指摘した文書の数。
    pub fired: usize,
    /// 値を測れたのに指摘しなかった文書の数。
    pub quiet: usize,
    /// どれかの文書の測定値に現れた、切り替え先の重大度 (軽い順)。
    pub switches: Vec<Severity>,
}

/// `make` で作ったルールについて、どの文書でも測定値と判定が一致すること
/// ([`assert_measure_consistent`]) を確かめる。閾値を測定値ちょうどとその前後に動かしても
/// 一致するか (比べる向きの確認) も見る。設定できない値 (範囲外) に動かす場合は飛ばす。
///
/// 重大度の切り替えは、どれかの文書の測定値に現れた重大度を、すべての文書で確かめる
/// (切り替えの値が欠けた文書も見逃さないように)。
pub fn assert_measures_agree(make: impl Fn() -> Box<dyn Rule>, docs: &[Document]) -> Agreement {
    let mut switches: Vec<Severity> = docs
        .iter()
        .flat_map(|doc| make().measure(&RuleContext::new(doc)))
        .filter_map(|m| m.switches_to)
        .collect();
    switches.sort();
    switches.dedup();
    let mut agreement = Agreement {
        switches,
        ..Agreement::default()
    };
    for (i, doc) in docs.iter().enumerate() {
        let rule = make();
        let id = rule.meta().id;
        let measures =
            assert_measure_consistent(rule.as_ref(), doc, &agreement.switches, &format!("例 {i}"));
        let current = numeric_options(rule.as_ref());
        let flagged = measures
            .iter()
            .filter(|m| m.switches_to.is_none())
            .any(|m| fires_now(rule.as_ref(), &current, m));
        if flagged {
            agreement.fired += 1;
        } else if !measures.is_empty() {
            agreement.quiet += 1;
        }
        for m in &measures {
            let now = current[m.threshold_key];
            let integer = m.value.fract() == 0.0 && now.fract() == 0.0;
            for value in nudged(m.value, integer) {
                let mut moved = make();
                if moved.configure(m.threshold_key, &value).is_err() {
                    continue;
                }
                assert_measure_consistent(
                    moved.as_ref(),
                    doc,
                    &agreement.switches,
                    &format!("{id} 例 {i}、{} = {value}", m.threshold_key),
                );
            }
        }
    }
    agreement
}
