//! R19: 文書末尾で実践を促す定型の呼びかけ。

use std::{ops::Range, sync::LazyLock};

use regex::Regex;

use crate::diagnostic::{Diagnostic, Lane, RuleStatus, Severity};
use crate::genre::Genre;
use crate::rules::{Rule, RuleContext, RuleMeta};
use crate::text;

use super::{final_prose_sentence, has_embedded_content};

static META: RuleMeta = RuleMeta {
    id: "R19",
    name: "CLOSING_CALL_TO_ACTION",
    title: "実践を促す定型の結び",
    lane: Lane::Slop,
    status: RuleStatus::Experimental,
    default_severity: Severity::Info,
    summary: "文書末尾の「まずは〜始めてみてください」など、実践を促す定型の呼びかけを指摘する (実験的)",
    explanation: r"### 何を見るか

文書の最後の文が地の文で、「まずは」「さっそく」「早速」「今日から」「あなたも」「皆さんも」「ぜひ」「是非」「一緒に」を含み、始める・試す・実践する・取り入れる・挑戦する・踏み出す・取り組むよう勧めて終わる型を探します。「始めてみてください」「試してみましょう」「踏み出しましょう」などの形です。呼びかけの語と動詞の間は 90 字以内に限ります。

general と tech で有効です。本文の途中、リスト・表・引用・脚注、括弧や算用数字を含む文、コード・リンク・数式・画像を含む文は除きます。本文中の http://・https:// も除外します。後ろに見出し・コードブロック・区切り線などがあれば、文書の結びとはみなしません。一文だけの文書も対象です。

### なぜ問題か

内容の異なる記事を同じ勧誘で締めると、本文から何を持ち帰ればよいかが曖昧になります。ただし、人の解説でも普通に使う表現です。定型であることだけで書き手や執筆方法は判断できません。

### 直し方

読者が行うことが具体的に書かれているか、最後の呼びかけが必要かを確認してください。本文に沿った助言なら残してかまいません。書かれていない体験・手順・効果を補う必要はありません。

### 例

- 直す前: まずは自分に合う方法から、試してみましょう。
- 直した後: 本文に書いた手順を確認できる結びにするか、必要のない呼びかけを削る。

### 根拠

実験的なルールです。2026 年 9 月の比較では、人の一般記事 26 件・技術記事 45 件に指摘はなく、生成した一般記事 68 件中 6 件、技術記事 32 件中 0 件で指摘しました。別に取得した生成を明記した公開記事 10 件では 2 件でした。このうち 1 件は、既存の実験的ルールを含めても AI 臭さの指摘がなかった記事です。

候補の選定にも使った資料なので、独立した評価ではありません。人の資料で指摘がなくても、誤検知率の片側 95% 上限は一般記事で 9.4%、技術記事で 5.7% です。読者への勧めが必要な記事にも出るため、情報として見直しを促します。辞書は使いません。",
};

static LEAD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"まずは|さっそく|早速|今日から|あなたも|皆さんも|ぜひ|是非|一緒に")
        .expect("action lead regex")
});

static CLOSER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^(?P<m>[^。！？!?\n]{0,90}(?:(?:(?:始め|はじめ|試し|ためし|実践し|取り入れ|挑戦し|踏み出し)て|取り組んで)み(?:てください|ましょう|ませんか)|(?:始め|はじめ|試し|ためし|実践し|取り入れ|挑戦し|踏み出し)ま(?:しょう|せんか)))[。！？!?]?\s*$",
    )
    .expect("action closer regex")
});

/// 文中の除外条件と呼びかけの語義を確かめ、解析用テキストの範囲を返す。
fn invitation(value: &str) -> Option<Range<usize>> {
    if value.chars().any(|c| {
        c.is_numeric()
            || text::closing_bracket(c).is_some()
            || text::is_closing_bracket(c)
            || c == text::PLACEHOLDER
    }) || value.contains("https://")
        || value.contains("http://")
    {
        return None;
    }
    LEAD.find_iter(value).find_map(|lead| {
        let before = value[..lead.start()].trim_end();
        let after = &value[lead.end()..];
        // 同行者を表す「と一緒に」と、名詞の「是非」は呼びかけに数えない。
        if (lead.as_str() == "一緒に" && before.ends_with('と'))
            || (lead.as_str() == "是非"
                && after
                    .trim_start()
                    .starts_with(['は', 'が', 'を', 'も', 'の']))
        {
            return None;
        }
        CLOSER
            .captures(after)
            .and_then(|c| c.name("m").map(|m| lead.start()..lead.end() + m.end()))
    })
}

pub struct ClosingCallToAction;

impl Rule for ClosingCallToAction {
    fn meta(&self) -> &'static RuleMeta {
        &META
    }

    fn allowed_in(&self, genre: Genre) -> bool {
        matches!(genre, Genre::General | Genre::Tech)
    }

    fn check(&self, ctx: &RuleContext<'_>, out: &mut Vec<Diagnostic>) {
        let doc = ctx.doc;
        let Some((last, block)) = final_prose_sentence(doc) else {
            return;
        };
        if has_embedded_content(block, last.span) {
            return;
        }
        let Some(range) = invitation(doc.sentence_text(last)) else {
            return;
        };
        let span = block.to_source(last.range.start + range.start..last.range.start + range.end);
        out.push(
            META.diagnostic(span, "文書の末尾に、実践を促す定型の呼びかけがあります")
                .with_hint("人の解説にもある表現です。行うことが具体的か、本文に沿った助言かを確認し、必要なら残してください")
                .with_context(last.span),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::RuleUnit;
    use crate::rules::testing::{Options, matched, run, run_doc};

    #[test]
    fn detects_invitation_at_the_actual_document_end() {
        for (md, expected) in [
            (
                "読める範囲は以上です。まずは身近な方法から試してみましょう。",
                "まずは身近な方法から試してみましょう",
            ),
            (
                "さあ、一緒に小さな一歩を踏み出しましょう！",
                "一緒に小さな一歩を踏み出しましょう",
            ),
            (
                "今日から気軽に始めてみませんか？",
                "今日から気軽に始めてみませんか",
            ),
            (
                "まずは**身近な方法**から試してみてください。\n",
                "まずは**身近な方法**から試してみてください",
            ),
            (
                "まずは、\n自分に合う方法を取り入れてみましょう。",
                "まずは、\n自分に合う方法を取り入れてみましょう",
            ),
            (
                "まずは一つ試してみてください。",
                "まずは一つ試してみてください",
            ),
            (
                "導入の是非はさておき、まずは小さく試してみてください。",
                "まずは小さく試してみてください",
            ),
            (
                "是非とも身近な方法から試してみてください。",
                "是非とも身近な方法から試してみてください",
            ),
        ] {
            let d = run(&ClosingCallToAction, md);
            assert_eq!(matched(md, &d), [expected], "{md}");
            assert_eq!((d[0].severity, d[0].lane), (Severity::Info, Lane::Slop));
        }
        let plain = crate::Document::plain_text("まずは身近な方法から試してみましょう。");
        assert_eq!(
            run_doc(&ClosingCallToAction, &plain, Options::default()).len(),
            1
        );
    }

    #[test]
    fn excludes_quotes_specific_values_and_reported_speech() {
        for md in [
            "まずは一度試してみましょう、とは勧めません。",
            "先輩は「まずは身近な方法から試してみましょう」と言った。",
            "まずは（可能な範囲で）試してみましょう。",
            "まずは3日間、記録を始めてみましょう。",
            "まずは３日間、記録を始めてみましょう。",
            "まずは `sample` を試してみましょう。",
            "まずは https://example.com を試してみましょう。",
            "まずは <https://example.com> を試してみましょう。",
            "まずは[体験版](https://example.com)を試してみましょう。",
            "設定後、次のコマンドを実行してください。",
            "まずは身近な方法から試してみるつもりです。",
            "まずは身近な方法から試してみましょうか。",
            "子どもと一緒に試してみてください。",
            "是非はともかく、身近な方法から試してみてください。",
            "ぜひご参加ください。",
        ] {
            assert!(run(&ClosingCallToAction, md).is_empty(), "{md}");
        }
    }

    #[test]
    fn does_not_treat_mid_document_or_fragments_as_a_closer() {
        let call = "まずは身近な方法から試してみましょう。";
        for md in [
            format!("{call}次に結果を確認します。"),
            format!("{call}\n\n# 補足\n"),
            format!("{call}\n\n---\n"),
            format!("{call}\n\n```text\n補足\n```\n"),
            format!("- {call}\n"),
            format!("> {call}\n"),
            format!("| 補足 |\n| --- |\n| {call} |\n"),
            format!("補足[^1]。\n\n[^1]: {call}\n"),
        ] {
            assert!(run(&ClosingCallToAction, &md).is_empty(), "{md}");
        }
        assert_eq!(ClosingCallToAction.unit(), RuleUnit::Document);
        assert!(!ClosingCallToAction.allowed_in(Genre::Business));
        assert!(!ClosingCallToAction.allowed_in(Genre::Essay));
    }
}
