//! R10 CLEFT_BECAUSE: 「それは〜である。なぜなら〜」の強調構文。

use std::sync::LazyLock;

use regex::Regex;

use crate::diagnostic::{Diagnostic, Lane, RuleStatus, Severity, Span};
use crate::document::Sentence;
use crate::genre::Genre;
use crate::rules::{Rule, RuleContext, RuleMeta};

use super::strip_sentence_end;

static META: RuleMeta = RuleMeta {
    id: "R10",
    name: "CLEFT_BECAUSE",
    title: "「それは〜。なぜなら〜」構文",
    lane: Lane::Slop,
    status: RuleStatus::Experimental,
    default_severity: Severity::Warning,
    summary: "「それは〜である。なぜなら〜」と、英語の強調構文を直訳した 2 文になっている",
    explanation: EXPLANATION,
};

const EXPLANATION: &str = "\
### 何を見るか

「それは〜である。」「これは〜だ。」の直後に、「なぜなら」「というのも」で始まる文が続く 2 文を\
指します。

### なぜ問題か

英語の強調構文 It is … because … をそのまま日本語に移した組み立てです。日本語で強調するときに\
この形を選ぶ書き手は少なく、理由を先に述べるか、一文にまとめるのが普通です。

### 直し方

理由を先に書くか、「〜だからだ」「〜のためだ」で一文にまとめてください。

### 例

- 直す前: 「それは避けられない判断だ。なぜなら、部署ごとに締め日が違うからだ。」
- 直した後: 「部署ごとに締め日が違うので、この判断は避けられない。」

### 根拠

実験的です。構文の形ははっきりしていますが、出現そのものがまれで、人の文書と生成文書を見分ける力を\
コーパスで測れていません。
";

/// 前の文: 「(それ|これ|この)は〜(である|だ)」で終わる。
static HEAD_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:それ|これ|この)は.{0,60}(?:である|だ)$").expect("cleft head regex")
});
/// 次の文: 「なぜなら」「というのも」で始まる。
static BECAUSE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?:なぜなら|というのも)").expect("because regex"));

/// R10 CLEFT_BECAUSE。
pub struct CleftBecause;

impl CleftBecause {
    pub fn new(_genre: Genre) -> Self {
        Self
    }
}

impl Rule for CleftBecause {
    fn meta(&self) -> &'static RuleMeta {
        &META
    }

    fn check(&self, ctx: &RuleContext<'_>, out: &mut Vec<Diagnostic>) {
        let doc = ctx.doc;
        let sentences: Vec<&Sentence> = doc.prose_sentences().collect();
        for pair in sentences.windows(2) {
            let (first, second) = (pair[0], pair[1]);
            let head = strip_sentence_end(doc.sentence_text(first).trim_start());
            let next = doc.sentence_text(second).trim_start();
            if !HEAD_RE.is_match(head) || !BECAUSE_RE.is_match(next) {
                continue;
            }
            out.push(
                META.diagnostic(
                    first.span,
                    "「それは〜だ。なぜなら〜」の 2 文構成になっています (英語の It is … because … の直訳調)",
                )
                .with_hint("理由を先に書くか、「〜だからだ」「〜のためだ」で一文にまとめてください")
                .with_context(Span::new(first.span.start, second.span.end))
                .with_related(vec![second.span]),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::testing::{matched, run};

    #[test]
    fn flags_cleft_followed_by_because() {
        let md = "それは避けられない判断だ。なぜなら、部署ごとに締め日が違うからだ。\n";
        let d = run(&CleftBecause::new(Genre::General), md);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].severity, Severity::Warning);
        assert_eq!(matched(md, &d), vec!["それは避けられない判断だ。"]);
        assert_eq!(d[0].related.len(), 1);
    }

    #[test]
    fn works_across_paragraphs_and_with_de_aru() {
        let md = "これは組織の問題である。\n\nというのも、担当が決まっていないからだ。\n";
        assert_eq!(run(&CleftBecause::new(Genre::General), md).len(), 1);
    }

    #[test]
    fn ignores_other_pairs() {
        let md = "それは簡単な判断ではない。部署ごとに締め日が違う。\n\n締め日が違うのはなぜか。なぜなら歴史的な経緯がある。\n";
        assert!(run(&CleftBecause::new(Genre::General), md).is_empty());
    }

    #[test]
    fn has_no_options() {
        let mut r = CleftBecause::new(Genre::General);
        assert!(r.configure("x", &toml::Value::Integer(1)).is_err());
        assert!(r.options().is_empty());
    }
}
