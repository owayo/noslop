//! P22: 表示用の出典に変換されずに残った引用マーカー。

use std::collections::HashSet;

use crate::diagnostic::{Diagnostic, Lane, RuleStatus, Severity, Span};
use crate::document::{Block, Document};
use crate::rules::{Rule, RuleContext, RuleMeta, RuleUnit, quote};

use super::engine::{Entry, Matcher, PhraseSpec, diagnostic};
use super::location::{line_offsets, sentence_context, single_line_span};

static P22: PhraseSpec = PhraseSpec {
    meta: RuleMeta {
        id: "P22",
        name: "CITATION_ARTIFACT",
        title: "引用マーカーの残骸",
        lane: Lane::Slop,
        status: RuleStatus::Experimental,
        default_severity: Severity::Warning,
        summary: "生成ツールの引用マーカーが、出典のリンクに変換されず本文に残っている箇所を指摘する (実験的)",
        explanation: r"### 何を見るか

本文に残った [cite_start]、[cite: 数字]、:contentReference[oaicite:数字]{index=数字}、専用の区切り文字で囲まれた cite マーカー、【数字†source】や【数字†L数字-L数字】を探します。cite マーカーの参照先は turn で始まる内部 ID に限ります。語句ルールのスコープ設定に関わらず、見出し・リスト・表・引用も含め、1 行の中にある完全なマーカーを検査します。

通常の脚注 [1]・[^1]、著者と年の引用、単独の内部 ID、コードブロックとインラインコード、リンクの URL 部分は対象にしません。リンクの表示文字列に完全なマーカーが残る場合は対象です。コードのコメントや表計算のセルなど、独立した断片にも当てますが、行や断片をまたいでマーカーを組み立てません。

### なぜ問題か

内部の引用記号だけでは読者が出典を確認できません。生成文を転記したときなどに残りますが、記法そのものを解説している場合もあるため、著者や執筆方法の判定には使えません。

### 直し方

引用元を開いて本文との対応を確認し、読める出典名とリンクに置き換えてください。記法の説明ならコードとして囲むか、抑制してください。参照先を確かめずに記号だけ消すと、根拠のない断定が残ることがあります。

### 例

- 直す前: 受付は来月からです [cite: 12]。
- 直した後: 受付は来月からです (主催者の募集要項へのリンクを添える)。

### 根拠

公開文書に残った未変換の引用記号を対象とする実験的なルールです。既知の記法を列挙しており、未知の生成ツールの記号を網羅するものではありません。一般の引用や外字を広く検出するためのルールでもありません。",
    },
    entries: &[
        Entry::exp_lit("[cite_start]", Severity::Warning),
        Entry::exp_re(r"\[cite:\s*\d+(?:\s*,\s*\d+)*\s*\]", Severity::Warning),
        Entry::exp_re(
            r":?contentReference\[oaicite:\d+\]\{index=\d+\}",
            Severity::Warning,
        ),
        Entry::exp_re(
            r"\x{E200}cite(?:\x{E202}turn[0-9A-Za-z]+)+\x{E201}",
            Severity::Warning,
        ),
        Entry::exp_re(
            r"【\d+(?::\d+)?†(?:source|L\d+(?:[-–]L?\d+)?)】",
            Severity::Warning,
        ),
    ],
    message: "引用マーカー「{m}」が表示用の出典に変換されずに残っています",
    hint: "参照先と本文の対応を確認し、出典名とリンクに置き換えてください。記法の説明ならコードとして囲めます",
};

/// 記号の残骸は文の語句と異なり、見出しや項目の中でも読者に見える。
pub(super) struct CitationArtifact {
    matcher: Matcher,
}

impl CitationArtifact {
    pub fn new() -> Self {
        Self {
            matcher: Matcher::new(P22.entries, true),
        }
    }

    /// 見出しの属性として除かれた引用記号の末尾を、同じ原文位置の本文から補う。
    fn check_heading(
        &self,
        doc: &Document,
        block: &Block,
        seen: &mut HashSet<Span>,
        out: &mut Vec<Diagnostic>,
    ) {
        if !block.is_heading() {
            return;
        }
        let source = doc.slice(block.span);
        for hit in self.matcher.find(source) {
            let matched = &source[hit.range.clone()];
            let Some((prefix, _)) = matched.rsplit_once("{index=") else {
                continue;
            };
            let span = Span::new(
                block.span.start + hit.range.start,
                block.span.start + hit.range.end,
            );
            let prefix_span = Span::new(span.start, span.start + prefix.len());
            // 原文だけを広く走査すると、コード・コメント・URL の中まで拾ってしまう。
            if !seen.contains(&span)
                && block
                    .text
                    .match_indices(prefix)
                    .any(|(start, _)| block.to_source(start..start + prefix.len()) == prefix_span)
            {
                seen.insert(span);
                out.push(finding(span, block.span, matched, &P22.entries[hit.entry]));
            }
        }
    }
}

fn finding(span: Span, context: Span, matched: &str, entry: &Entry) -> Diagnostic {
    diagnostic(
        &P22.meta,
        span,
        context,
        matched,
        P22.message.replace("{m}", &quote(matched)),
        P22.hint,
        entry.severity,
        entry.status,
    )
    .with_metric("item", entry.pattern.item())
}

impl Rule for CitationArtifact {
    fn meta(&self) -> &'static RuleMeta {
        &P22.meta
    }

    fn unit(&self) -> RuleUnit {
        RuleUnit::Sentence
    }

    fn check(&self, ctx: &RuleContext<'_>, out: &mut Vec<Diagnostic>) {
        let doc = ctx.doc;
        let mut seen = HashSet::new();
        for (idx, block) in doc.blocks.iter().enumerate() {
            for (line_start, line) in line_offsets(&block.text) {
                for hit in self.matcher.find(line) {
                    let range = line_start + hit.range.start..line_start + hit.range.end;
                    let Some(span) = single_line_span(doc, block, range.clone()) else {
                        continue;
                    };
                    if !seen.insert(span) {
                        continue;
                    }
                    let entry = &P22.entries[hit.entry];
                    let matched = &block.text[range.clone()];
                    let context = sentence_context(doc, idx, range.start);
                    out.push(finding(span, context, matched, entry));
                }
            }
            self.check_heading(doc, block, &mut seen, out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::Scope;
    use crate::rules::testing::{Options, matched, run, run_with};

    #[test]
    fn matches_complete_markers_with_original_positions() {
        for marker in [
            "[cite_start]",
            "[cite: 12, 14]",
            ":contentReference[oaicite:0]{index=0}",
            "contentReference[oaicite:18]{index=9}",
            "\u{e200}cite\u{e202}turn0search1\u{e202}turn2view3\u{e201}",
            "【12†source】",
            "【4:2†L10-L20】",
        ] {
            let md = format!("調査結果です。**{marker}**\n");
            let d = run(&CitationArtifact::new(), &md);
            assert_eq!(matched(&md, &d), [marker], "{md}");
            assert_eq!(d[0].status, RuleStatus::Experimental);
        }
        // 日本語を含まない単独の記号も拾う。
        assert_eq!(run(&CitationArtifact::new(), "[cite_start]").len(), 1);
        let escaped = r"根拠です [cite\_start]。";
        assert_eq!(
            matched(escaped, &run(&CitationArtifact::new(), escaped)),
            [r"[cite\_start]"]
        );
        assert_eq!(CitationArtifact::new().unit(), RuleUnit::Sentence);
    }

    #[test]
    fn ignores_normal_citations_code_and_incomplete_tokens() {
        for md in [
            "結果[1]は一致した。",
            "結果[^1]は一致した。\n\n[^1]: 調査報告\n",
            "結果 (著者, 2020) は一致した。",
            "【12ページ】を参照。",
            "turn0search1 という識別子。",
            "外字\u{e200}と\u{e201}を表示する。",
            "\u{e200}cite\u{e202}資料名\u{e201}",
            "[cite: 記号の説明]",
            ":contentReference[oaicite:0]",
            "`[cite_start]` は記号です。",
            "```text\n[cite: 12]\n```\n",
        ] {
            assert!(run(&CitationArtifact::new(), md).is_empty(), "{md}");
        }
    }

    #[test]
    fn checks_visible_markers_in_all_blocks_regardless_of_phrase_scope() {
        let md = "# 結果 [cite: 0]\n\n- 結果 [cite: 1]\n\n> 結果 [cite: 2]\n\n| [cite: 3] |\n| --- |\n| [cite: 4] |\n\n結果 [cite: 5]。\n";
        for scope in [Scope::default(), Scope::ALL] {
            let d = run_with(
                &CitationArtifact::new(),
                md,
                Options {
                    scope,
                    ..Options::default()
                },
            );
            assert_eq!(
                matched(md, &d),
                (0..=5).map(|n| format!("[cite: {n}]")).collect::<Vec<_>>()
            );
            assert!(d.iter().all(|d| d.context.is_some()));
        }
    }

    #[test]
    fn does_not_join_markers_across_lines_or_removed_content() {
        for md in [
            "結果 [cite: 1,\n2]。",
            "結果 [cite: 1,\r\n2]。",
            "結果 [ci`説明`te: 1]。",
            "結果 [cite: 1, `2`]。",
            "- [cite: 1,\n- 2]\n",
            "# `[cite: 1]`\n\n- `[cite: 2]`\n\n> `[cite: 3]`\n",
            "[出典](https://example.com/[cite:1])",
            "![代替文 [cite: 1]](image.png)",
            "<!-- [cite: 1] -->",
        ] {
            assert!(run(&CitationArtifact::new(), md).is_empty(), "{md}");
        }
        let md = "結果 [[cite: 1]](https://example.com/report)。";
        assert_eq!(
            matched(md, &run(&CitationArtifact::new(), md)),
            ["[cite: 1]"]
        );
    }

    #[test]
    fn keeps_complete_markers_when_a_heading_suffix_is_parsed_as_attributes() {
        for marker in [
            ":contentReference[oaicite:0]{index=0}",
            "contentReference[oaicite:18]{index=9}",
        ] {
            for ending in ["", "。", "\r\n"] {
                let md = format!("# 根拠 {marker}{ending}\n");
                let d = run(&CitationArtifact::new(), &md);
                assert_eq!(matched(&md, &d), [marker], "{md}");
            }
        }
        for md in [
            "# `contentReference[oaicite:0]`{index=0}\n",
            "# <!-- contentReference[oaicite:0]{index=0} -->\n",
            "# [出典](https://example.com/contentReference[oaicite:0]{index=0})\n",
            "# 説明 {index=0}\n",
            "# contentReference[oaicite:0]\n",
        ] {
            assert!(run(&CitationArtifact::new(), md).is_empty(), "{md}");
        }
    }
}
