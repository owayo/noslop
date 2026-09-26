//! P22: 表示用の出典に変換されずに残った引用マーカー。

use crate::diagnostic::{Lane, RuleStatus, Severity};
use crate::rules::RuleMeta;

use super::engine::{Entry, PhraseSpec};

pub(super) static P22: PhraseSpec = PhraseSpec {
    meta: RuleMeta {
        id: "P22",
        name: "CITATION_ARTIFACT",
        title: "引用マーカーの残骸",
        lane: Lane::Slop,
        status: RuleStatus::Experimental,
        default_severity: Severity::Warning,
        summary: "生成ツールの引用マーカーが、出典のリンクに変換されず本文に残っている箇所を指摘する (実験的)",
        explanation: r"### 何を見るか

本文に残った [cite_start]、[cite: 数字]、:contentReference[oaicite:数字]{index=数字}、専用の区切り文字で囲まれた cite マーカー、【数字†source】や【数字†L数字-L数字】を探します。cite マーカーの参照先は turn で始まる内部 ID に限ります。

通常の脚注 [1]・[^1]、著者と年の引用、単独の内部 ID、コードブロックとインラインコードは対象にしません。リスト・表・引用ブロックは語句ルールのスコープ設定に従います。

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::phrases::engine::PhraseRule;
    use crate::rules::testing::{Options, matched, run, run_with};
    use crate::rules::{Rule, RuleUnit, Scope};

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
            let d = run(&PhraseRule::new(&P22), &md);
            assert_eq!(matched(&md, &d), [marker], "{md}");
            assert_eq!(d[0].status, RuleStatus::Experimental);
        }
        // 日本語を含まない単独の記号も拾う。
        assert_eq!(run(&PhraseRule::new(&P22), "[cite_start]").len(), 1);
        let escaped = r"根拠です [cite\_start]。";
        assert_eq!(
            matched(escaped, &run(&PhraseRule::new(&P22), escaped)),
            [r"[cite\_start]"]
        );
        assert_eq!(PhraseRule::new(&P22).unit(), RuleUnit::Sentence);
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
            assert!(run(&PhraseRule::new(&P22), md).is_empty(), "{md}");
        }
    }

    #[test]
    fn respects_phrase_scope() {
        let md = "- 結果 [cite: 1]\n\n> 結果 [cite: 2]\n\n| 結果 |\n| --- |\n| [cite: 3] |\n";
        assert!(run(&PhraseRule::new(&P22), md).is_empty());
        let d = run_with(
            &PhraseRule::new(&P22),
            md,
            Options {
                scope: Scope::ALL,
                ..Options::default()
            },
        );
        assert_eq!(matched(md, &d), ["[cite: 1]", "[cite: 2]", "[cite: 3]"]);
    }
}
