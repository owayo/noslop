//! 設定ファイルの `[[custom]]` で定義する独自ルール。
//!
//! チームの表記規則や独自の禁止語を、組み込みルールと同じ仕組み (抑制コメント・
//! 出力形式・`--fail-on`) で扱えるようにする。自然度スコアには入れない。

use aho_corasick::{AhoCorasick, MatchKind};
use regex::{Regex, RegexBuilder};

use crate::config::{ConfigError, CustomRuleConfig};
use crate::diagnostic::{Diagnostic, Lane, RuleStatus, Severity};
use crate::rules::{Rule, RuleContext, RuleMeta};

/// パターン文字列の長さの上限 (文字数)。
const MAX_PATTERN_CHARS: usize = 1_000;
/// 正規表現のコンパイル後の大きさの上限 (バイト)。
const REGEX_SIZE_LIMIT: usize = 1 << 20;

enum Matcher {
    Literal(AhoCorasick),
    Regex(Regex),
}

impl Matcher {
    fn find_iter<'h>(&'h self, haystack: &'h str) -> Box<dyn Iterator<Item = (usize, usize)> + 'h> {
        match self {
            Matcher::Literal(ac) => Box::new(ac.find_iter(haystack).map(|m| (m.start(), m.end()))),
            Matcher::Regex(re) => Box::new(
                re.find_iter(haystack)
                    .filter(|m| !m.is_empty())
                    .map(|m| (m.start(), m.end())),
            ),
        }
    }
}

/// 独自ルール。
pub struct CustomRule {
    meta: &'static RuleMeta,
    matcher: Matcher,
    message: String,
    hint: Option<String>,
}

impl CustomRule {
    /// 設定から独自ルールを作る。
    ///
    /// [`RuleMeta`] は `'static` の文字列を要求するため、設定の文字列をプロセスの
    /// 寿命ぶん確保する (設定は 1 回の実行で 1 度しか読まないので量は設定の大きさで頭打ち)。
    pub fn from_config(cfg: &CustomRuleConfig) -> Result<Self, ConfigError> {
        let id = cfg.id.trim();
        if id.is_empty() || id.contains(|c: char| c.is_whitespace() || c == ',') {
            return Err(ConfigError::Invalid(format!(
                "独自ルールの id が正しくありません: 「{}」(空白やカンマを含められません)",
                cfg.id
            )));
        }
        if cfg.pattern.is_empty() {
            return Err(ConfigError::Invalid(format!(
                "独自ルール {id} の pattern が空です"
            )));
        }
        if cfg.pattern.chars().count() > MAX_PATTERN_CHARS {
            return Err(ConfigError::Invalid(format!(
                "独自ルール {id} の pattern が長すぎます ({MAX_PATTERN_CHARS} 文字まで)"
            )));
        }
        let matcher = if cfg.regex {
            let re = RegexBuilder::new(&cfg.pattern)
                .size_limit(REGEX_SIZE_LIMIT)
                .dfa_size_limit(REGEX_SIZE_LIMIT)
                .build()
                .map_err(|e| {
                    ConfigError::Invalid(format!(
                        "独自ルール {id} の正規表現が正しくありません: {e}"
                    ))
                })?;
            Matcher::Regex(re)
        } else {
            let ac = AhoCorasick::builder()
                .match_kind(MatchKind::LeftmostLongest)
                .build([cfg.pattern.as_str()])
                .map_err(|e| {
                    ConfigError::Invalid(format!("独自ルール {id} の pattern を扱えません: {e}"))
                })?;
            Matcher::Literal(ac)
        };
        let severity: Severity = cfg.severity()?;
        let lane: Lane = cfg.lane()?;
        let name = cfg
            .name
            .as_deref()
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .unwrap_or(id);
        let kind = if cfg.regex {
            "正規表現"
        } else {
            "文字列"
        };
        let explanation = format!(
            "### 何を見るか\n\n設定ファイルの `[[custom]]` で定義した独自ルール。{kind} `{}` に一致する箇所を指します。\n\n### 直し方\n\n{}\n",
            cfg.pattern,
            cfg.hint.as_deref().unwrap_or(&cfg.message)
        );
        let meta: &'static RuleMeta = Box::leak(Box::new(RuleMeta {
            id: leak(id),
            name: leak(name),
            title: leak(name),
            lane,
            status: RuleStatus::Stable,
            default_severity: severity,
            summary: leak(&cfg.message),
            explanation: leak(&explanation),
        }));
        Ok(Self {
            meta,
            matcher,
            message: cfg.message.clone(),
            hint: cfg.hint.clone(),
        })
    }
}

fn leak(s: &str) -> &'static str {
    Box::leak(s.to_string().into_boxed_str())
}

impl Rule for CustomRule {
    fn meta(&self) -> &'static RuleMeta {
        self.meta
    }

    fn check(&self, ctx: &RuleContext<'_>, out: &mut Vec<Diagnostic>) {
        for (idx, block) in ctx.scoped_blocks() {
            for sentence in ctx.doc.block_sentences(idx) {
                let text = &block.text[sentence.range.clone()];
                for (start, end) in self.matcher.find_iter(text) {
                    let base = sentence.range.start;
                    let span = block.to_source(base + start..base + end);
                    let mut d = self
                        .meta
                        .diagnostic(span, self.message.clone())
                        .with_context(sentence.span)
                        .with_metric("matched", &text[start..end]);
                    if let Some(hint) = &self.hint {
                        d = d.with_hint(hint.clone());
                    }
                    out.push(d);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::testing;

    fn cfg(pattern: &str, regex: bool) -> CustomRuleConfig {
        CustomRuleConfig {
            id: "X01".into(),
            name: Some("BANNED_TERM".into()),
            pattern: pattern.into(),
            regex,
            message: "表記を統一します".into(),
            hint: Some("「利用者」と書いてください".into()),
            severity: Some("info".into()),
            lane: None,
        }
    }

    #[test]
    fn literal_rule_reports_each_occurrence_with_context() {
        let rule = CustomRule::from_config(&cfg("ユーザー様", false)).unwrap();
        let md = "ユーザー様の声を聞く。次にユーザー様へ返す。\n";
        let diags = testing::run(&rule, md);
        assert_eq!(
            testing::matched(md, &diags),
            vec!["ユーザー様", "ユーザー様"]
        );
        assert_eq!(diags[0].severity, Severity::Info);
        assert_eq!(diags[0].lane, Lane::Custom);
        assert_eq!(diags[0].rule_name, "BANNED_TERM");
        assert_eq!(
            &md[diags[1].context.unwrap().range()],
            "次にユーザー様へ返す。"
        );
    }

    #[test]
    fn regex_rule_matches_and_skips_empty_matches() {
        let rule = CustomRule::from_config(&cfg("サーバー?", true)).unwrap();
        let md = "サーバーとサーバを混ぜる。\n";
        let diags = testing::run(&rule, md);
        assert_eq!(testing::matched(md, &diags), vec!["サーバー", "サーバ"]);
        let empty = CustomRule::from_config(&cfg("x*", true)).unwrap();
        assert!(testing::run(&empty, "あいう。\n").is_empty());
    }

    #[test]
    fn rejects_invalid_definitions() {
        assert!(CustomRule::from_config(&cfg("(", true)).is_err());
        assert!(CustomRule::from_config(&cfg("", false)).is_err());
        let mut bad_id = cfg("a", false);
        bad_id.id = "X 1".into();
        assert!(CustomRule::from_config(&bad_id).is_err());
        let long = "あ".repeat(MAX_PATTERN_CHARS + 1);
        assert!(CustomRule::from_config(&cfg(&long, false)).is_err());
    }

    #[test]
    fn respects_scope() {
        let rule = CustomRule::from_config(&cfg("禁止語", false)).unwrap();
        let md = "- 禁止語を含む項目\n\n禁止語を含む段落。\n";
        assert_eq!(testing::run(&rule, md).len(), 1);
        let all = testing::run_with(
            &rule,
            md,
            testing::Options {
                scope: crate::rules::Scope::ALL,
                ..Default::default()
            },
        );
        assert_eq!(all.len(), 2);
    }
}
