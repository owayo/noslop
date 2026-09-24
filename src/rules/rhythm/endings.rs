//! 文末の形を見るルール。
//!
//! - R02 REPETITIVE_ENDING: 同じ文末が続く
//! - R06 NO_NOMINAL_ENDING: 長い文書なのに名詞で終わる文が 1 つもない

use crate::diagnostic::{Diagnostic, Lane, RuleStatus, Severity, Span};
use crate::document::{Document, Sentence};
use crate::genre::Genre;
use crate::rules::{Fires, Measure, Rule, RuleContext, RuleMeta};

use super::{is_nominal_ending, is_nounish, option_count, strip_sentence_end, unknown_option};

/// 文末の型。
#[derive(Debug, Clone, PartialEq, Eq)]
enum Ending {
    /// 名詞らしい語で終わる (体言止め)。語が違っても同じ型として扱う。
    Nominal,
    /// 末尾の数文字。
    Tail(String),
}

/// 文の文末の型。文末記号・閉じ括弧・空白だけの文なら `None`。
fn ending_of(sentence: &str, tail_chars: usize) -> Option<Ending> {
    let core = strip_sentence_end(sentence);
    let last = core.chars().next_back()?;
    if is_nounish(last) {
        return Some(Ending::Nominal);
    }
    let chars: Vec<char> = core.chars().collect();
    let start = chars.len().saturating_sub(tail_chars.max(1));
    Some(Ending::Tail(chars[start..].iter().collect()))
}

// ---------------------------------------------------------------------------
// R02 REPETITIVE_ENDING
// ---------------------------------------------------------------------------

static REPETITIVE_META: RuleMeta = RuleMeta {
    id: "R02",
    name: "REPETITIVE_ENDING",
    title: "文末の反復",
    lane: Lane::Readability,
    status: RuleStatus::Experimental,
    default_severity: Severity::Info,
    summary: "同じ段落で同じ形の文末が 3 文以上続いている",
    explanation: REPETITIVE_EXPLANATION,
};

const REPETITIVE_EXPLANATION: &str = "\
### 何を見るか

同じ段落の中で続く文の文末 (文末記号と閉じ括弧を除いた最後の 3 字) を比べ、同じ形が \
3 文以上続く箇所を指します。名詞で終わる文 (体言止め) は、語が違っても 1 つの型として数えます。

### なぜ問題か

「〜しています。〜しています。〜しています。」のように同じ文末が続くと、内容より文末の繰り返しが\
耳につきます。生成された文章は直前の文の組み立てをなぞりやすく、文末がそろいがちです。

### 直し方

3 文目の締め方を変えてください。2 文をつないで 1 文にする、問いかけや体言止めで締める、\
内容に合わせて述語の形 (現在・過去・推量) を変える、などの手があります。

### 例

- 直す前: 「各拠点で在庫を数えています。差異は週ごとに集計しています。原因の多くは入力漏れだと\
考えています。」
- 直した後: 「各拠点で在庫を数え、差異を週ごとに集計している。原因の多くは入力漏れだろう。」

### 根拠

実験的です。文末の反復を数える検出器は誤検知率をまだ測っていません。です・ます体では同じ文末が\
続くのが普通なので、比べる長さを 3 字にして「です」「ます」だけの一致では出ないようにしています。\
AI らしさの判定ではなく推敲の手掛かりとして扱い、自然度スコアには入れません。
";

/// R02 REPETITIVE_ENDING。
pub struct RepetitiveEnding {
    /// 比べる文末の字数。
    tail_chars: usize,
    /// 何文続いたら出すか。
    min_run: usize,
}

impl RepetitiveEnding {
    pub fn new(_genre: Genre) -> Self {
        Self {
            tail_chars: 3,
            min_run: 3,
        }
    }

    fn report(run: &[&Sentence], ending: &Ending) -> Diagnostic {
        let first = run[0];
        let last = run[run.len() - 1];
        let span = Span::new(first.span.start, last.span.end);
        let n = run.len();
        let (message, hint, label) = match ending {
            Ending::Nominal => (
                format!("名詞で終わる文 (体言止め) が {n} 文続いています"),
                "述語で言い切る文を交ぜ、文の続きを読み手が予想できるようにしてください",
                "体言止め".to_string(),
            ),
            Ending::Tail(tail) => (
                format!("文末「〜{tail}」が {n} 文続いています"),
                "2 文をつなぐ、問いかけや体言止めで締めるなど、3 文目の締め方を変えてください",
                tail.clone(),
            ),
        };
        REPETITIVE_META
            .diagnostic(span, message)
            .with_hint(hint)
            .with_context(span)
            .with_related(run.iter().map(|s| s.span).collect())
            .with_metric("run", n)
            .with_metric("ending", label)
    }
}

impl Rule for RepetitiveEnding {
    fn meta(&self) -> &'static RuleMeta {
        &REPETITIVE_META
    }

    fn configure(&mut self, key: &str, value: &toml::Value) -> Result<(), String> {
        match key {
            "tail_chars" => self.tail_chars = option_count(key, value)?,
            "min_run" => self.min_run = option_count(key, value)?.max(2),
            _ => return Err(unknown_option(&REPETITIVE_META, key)),
        }
        Ok(())
    }

    fn options(&self) -> Vec<(&'static str, String)> {
        vec![
            ("tail_chars", self.tail_chars.to_string()),
            ("min_run", self.min_run.to_string()),
        ]
    }

    /// 地の文の段落の中で、同じ文末が最も長く続いた文数を `min_run` と比べる値として返す。
    fn measure(&self, ctx: &RuleContext<'_>) -> Vec<Measure> {
        let longest = self
            .runs(ctx.doc)
            .into_iter()
            .map(|(run, _)| run.len())
            .max();
        longest
            .map(|n| {
                vec![Measure::new(
                    "longest_run",
                    n as f64,
                    "min_run",
                    Fires::AtOrAbove,
                )]
            })
            .unwrap_or_default()
    }

    fn check(&self, ctx: &RuleContext<'_>, out: &mut Vec<Diagnostic>) {
        for (run, ending) in self.runs(ctx.doc) {
            if run.len() >= self.min_run {
                out.push(Self::report(&run, &ending));
            }
        }
    }
}

impl RepetitiveEnding {
    /// 地の文の段落ごとに、同じ文末が続く区間 (1 文だけの区間を含む) を並べる。
    fn runs<'a>(&self, doc: &'a Document) -> Vec<(Vec<&'a Sentence>, Ending)> {
        let mut out = Vec::new();
        for (idx, block) in doc.blocks.iter().enumerate() {
            if !block.is_prose() {
                continue;
            }
            let sentences: Vec<&Sentence> = doc
                .block_sentences(idx)
                .iter()
                .filter(|s| s.japanese)
                .collect();
            let endings: Vec<Option<Ending>> = sentences
                .iter()
                .map(|s| ending_of(doc.sentence_text(s), self.tail_chars))
                .collect();
            let mut i = 0;
            while i < sentences.len() {
                let Some(ending) = &endings[i] else {
                    i += 1;
                    continue;
                };
                let mut j = i + 1;
                while j < sentences.len() && endings[j].as_ref() == Some(ending) {
                    j += 1;
                }
                out.push((sentences[i..j].to_vec(), ending.clone()));
                i = j;
            }
        }
        out
    }
}

// ---------------------------------------------------------------------------
// R06 NO_NOMINAL_ENDING
// ---------------------------------------------------------------------------

static NOMINAL_META: RuleMeta = RuleMeta {
    id: "R06",
    name: "NO_NOMINAL_ENDING",
    title: "体言止めの欠如",
    lane: Lane::Slop,
    status: RuleStatus::Stable,
    default_severity: Severity::Info,
    summary: "ある程度の長さがあるのに、名詞で終わる文 (体言止め) が推定で 1 つもない",
    explanation: NOMINAL_EXPLANATION,
};

const NOMINAL_EXPLANATION: &str = "\
### 何を見るか

地の文が 5 文以上、合計 2000 字以上 (エッセイは 1500 字、技術文書・ビジネス文書は 3000 字) の\
文書で、名詞で終わる文 (体言止め) が推定で 1 つもないときに、最後の文の位置へ 1 件だけ出します。\
辞書を使わないため、文末の最後の文字が漢字・カタカナ・英数字か、「こと」「もの」などの形式名詞で\
終わる文を、名詞らしい終止と推定しています。

### なぜ問題か

体言止めは、人の書き手が文章のリズムを締めるために使う技法です。生成された文章にはほとんど\
現れません。ある程度の長さがあるのに一度も使われていないのは、述語で言い切る文だけが並ぶ\
一本調子の兆候です。

### 直し方

体言止めを無理に足す必要はありません。他の指摘と合わせ、文末が同じ調子で続いていないかを\
読み返す材料にしてください。強調したい一文を名詞で締めると効くことがあります。

### 例

- 直す前: 「問い合わせの件数は半分に減りました。担当者の負担も軽くなりました。夜間の一次対応が\
まだ残っています。」
- 直した後: 「問い合わせの件数は半分に減り、担当者の負担も目に見えて軽くなった。残る課題は\
夜間の一次対応。」

### 根拠

校正済みです。体言止めは当初「多用」を警告する想定でしたが、実際の文書では逆でした。エッセイでは\
人の書き手の 60% が体言止めを使い、生成された文章は 0% でした。2000 字前後の文書でも人の 67% が\
使い、生成文書は 0% です。短い文書は人でも体言止めがないことが多いため、文字数の下限を設けています。\
辞書なしの推定は品詞による判定より粗いため、情報に留めます。
";

/// R06 NO_NOMINAL_ENDING。
pub struct NoNominalEnding {
    /// 判定する地の文の合計文字数の下限。
    min_chars: usize,
    /// 判定する地の文の文数の下限。
    min_sentences: usize,
}

impl NoNominalEnding {
    pub fn new(genre: Genre) -> Self {
        let min_chars = match genre {
            Genre::General => 2000,
            Genre::Essay => 1500,
            Genre::Tech | Genre::Business => 3000,
        };
        Self {
            min_chars,
            min_sentences: 5,
        }
    }

    /// 地の文の文と合計文字数。文数が `min_sentences` に満たなければ `None`。
    fn prose<'a>(&self, doc: &'a Document) -> Option<(Vec<&'a Sentence>, usize)> {
        let sentences: Vec<&Sentence> = doc.prose_sentences().collect();
        if sentences.len() < self.min_sentences {
            return None;
        }
        let chars = sentences.iter().map(|s| s.length).sum();
        Some((sentences, chars))
    }
}

/// 名詞らしい終止の文が 1 つでもあるか。
fn has_nominal_ending(doc: &Document, sentences: &[&Sentence]) -> bool {
    sentences
        .iter()
        .any(|s| is_nominal_ending(doc.sentence_text(s)))
}

impl Rule for NoNominalEnding {
    fn meta(&self) -> &'static RuleMeta {
        &NOMINAL_META
    }

    fn configure(&mut self, key: &str, value: &toml::Value) -> Result<(), String> {
        match key {
            "min_chars" => self.min_chars = crate::rules::option_usize(key, value)?,
            "min_sentences" => self.min_sentences = option_count(key, value)?,
            _ => return Err(unknown_option(&NOMINAL_META, key)),
        }
        Ok(())
    }

    fn options(&self) -> Vec<(&'static str, String)> {
        vec![
            ("min_chars", self.min_chars.to_string()),
            ("min_sentences", self.min_sentences.to_string()),
        ]
    }

    /// 体言止めが推定で 0 件の文書に限り、地の文の合計文字数を `min_chars` と比べる値として返す。
    /// 体言止めがある文書は `min_chars` をどう変えても指摘しないので、値を返さない。
    fn measure(&self, ctx: &RuleContext<'_>) -> Vec<Measure> {
        let Some((sentences, chars)) = self.prose(ctx.doc) else {
            return Vec::new();
        };
        if has_nominal_ending(ctx.doc, &sentences) {
            return Vec::new();
        }
        vec![Measure::new(
            "prose_chars",
            chars as f64,
            "min_chars",
            Fires::AtOrAbove,
        )]
    }

    fn check(&self, ctx: &RuleContext<'_>, out: &mut Vec<Diagnostic>) {
        let doc = ctx.doc;
        let Some((sentences, chars)) = self.prose(doc) else {
            return;
        };
        if chars < self.min_chars {
            return;
        }
        if has_nominal_ending(doc, &sentences) {
            return;
        }
        let last = sentences[sentences.len() - 1];
        let n = sentences.len();
        out.push(
            NOMINAL_META
                .diagnostic(
                    last.span,
                    format!(
                        "地の文 {n} 文 (約 {chars} 字) に、名詞で終わる文 (体言止め) が推定で 0 件です"
                    ),
                )
                .with_hint(
                    "体言止めを無理に足す必要はありません。文末が述語の言い切りだけで一本調子に\
                     なっていないか読み返してください",
                )
                .with_metric("sentences", n)
                .with_metric("chars", chars)
                .with_metric("nominal_endings", 0usize)
                .with_metric("min_chars", self.min_chars),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::testing::{Options, matched, run, run_with};

    #[test]
    fn ending_types() {
        assert_eq!(
            ending_of("調べています。", 3),
            Some(Ending::Tail("います".into()))
        );
        assert_eq!(ending_of("成果物の評価。", 3), Some(Ending::Nominal));
        assert_eq!(
            ending_of("「行こう」", 3),
            Some(Ending::Tail("行こう".into()))
        );
        assert_eq!(ending_of("。", 3), None);
    }

    #[test]
    fn r02_flags_three_identical_endings_in_a_paragraph() {
        let md = "在庫を数えています。差異を集計しています。原因を調べています。\n";
        let d = run(&RepetitiveEnding::new(Genre::General), md);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].rule_id, "R02");
        assert_eq!(d[0].related.len(), 3);
        assert!(d[0].message.contains("います"));
        assert_eq!(
            matched(md, &d),
            vec!["在庫を数えています。差異を集計しています。原因を調べています。"]
        );
    }

    #[test]
    fn r02_counts_noun_endings_as_one_type() {
        let md = "成果物の評価。担当者の確認。期限の設定。\n";
        let d = run(&RepetitiveEnding::new(Genre::General), md);
        assert_eq!(d.len(), 1);
        assert!(d[0].message.contains("体言止め"));
    }

    #[test]
    fn r02_ignores_desu_masu_with_different_words() {
        let md = "これは重要な課題です。対応は来月です。担当はまだ未定です。\n";
        assert!(run(&RepetitiveEnding::new(Genre::General), md).is_empty());
    }

    #[test]
    fn r02_does_not_join_runs_across_paragraphs() {
        let md = "在庫を数えています。差異を集計しています。\n\n原因を調べています。\n";
        assert!(run(&RepetitiveEnding::new(Genre::General), md).is_empty());
    }

    #[test]
    fn r02_configure() {
        let mut r = RepetitiveEnding::new(Genre::General);
        r.configure("min_run", &toml::Value::Integer(2)).unwrap();
        let md = "在庫を数えています。差異を集計しています。\n";
        assert_eq!(run(&r, md).len(), 1);
        assert!(r.configure("x", &toml::Value::Integer(2)).is_err());
    }

    /// 動詞で終わる文を `chars` 字ぶん以上並べた本文。
    fn verb_only_text(chars: usize) -> String {
        let sentence = "担当者が毎朝の点検で見つけた不具合を記録して上長に報告した。";
        let per = crate::text::reading_length(sentence);
        let mut s = String::new();
        for _ in 0..=(chars / per) {
            s.push_str(sentence);
        }
        s.push('\n');
        s
    }

    #[test]
    fn r06_flags_long_text_without_noun_endings() {
        let md = verb_only_text(2100);
        let d = run(&NoNominalEnding::new(Genre::General), &md);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].rule_id, "R06");
        assert_eq!(d[0].severity, Severity::Info);
        assert!(d[0].message.contains("推定で 0 件"));
    }

    #[test]
    fn r06_stays_quiet_with_one_noun_ending_or_short_text() {
        let mut md = verb_only_text(2100);
        md.push_str("\n残る課題は夜間の一次対応。\n");
        assert!(run(&NoNominalEnding::new(Genre::General), &md).is_empty());
        assert!(run(&NoNominalEnding::new(Genre::General), &verb_only_text(1200)).is_empty());
    }

    #[test]
    fn r06_threshold_depends_on_genre() {
        let md = verb_only_text(1700);
        assert!(run(&NoNominalEnding::new(Genre::General), &md).is_empty());
        let essay = Options {
            genre: Genre::Essay,
            ..Options::default()
        };
        assert_eq!(
            run_with(&NoNominalEnding::new(Genre::Essay), &md, essay).len(),
            1
        );
        let long = verb_only_text(2500);
        assert!(run(&NoNominalEnding::new(Genre::Tech), &long).is_empty());
        let mut r = NoNominalEnding::new(Genre::Tech);
        r.configure("min_chars", &toml::Value::Integer(1000))
            .unwrap();
        assert_eq!(run(&r, &long).len(), 1);
        assert!(r.configure("y", &toml::Value::Integer(1)).is_err());
    }
}
