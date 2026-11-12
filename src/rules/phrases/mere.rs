//! P29: 対象をいったん「単なる〜」と低く置いてから、意味や価値を与え直す言い回し。

use crate::diagnostic::{Lane, Severity, Signal};
use crate::rules::RuleMeta;

use super::engine::{Entry, PhraseRule, PhraseSpec};

pub(super) fn rule() -> PhraseRule {
    PhraseRule::new(&P29).prefer_first(2)
}

pub(super) static P29: PhraseSpec = PhraseSpec {
    meta: RuleMeta {
        id: "P29",
        name: "NOT_MERELY",
        title: "「単なる〜ではない」の持ち上げ",
        lane: Lane::Slop,
        signal: Some(Signal::Strong),
        default_severity: Severity::Info,
        summary: "「単なる〜ではなく」「単なる〜以上の」のように、対象をいったん低く置いてから意味や価値を与え直す言い回しを見直す",
        explanation: r"### 何を見るか

「単なる」から 25 字以内に「ではなく」「にとどまらず」「にとどまらない」「以上の」「を超え」が続く形 (「単なる作業ではなく」「単なる食器以上の」) と、「ではない」「ではありません」「ではなかった」で言い切る形 (「単なる背景ではありません。」) を探します。言い切る形は、直後が句点・読点・感嘆符か文の終わりのときだけ拾い、「単なる誤記ではないか」「単なる偶然ではないでしょうか」のような疑問や推測は拾いません。

「単なる〜だけでなく」「単なる〜に留まらず」も、別の項目として情報・中程度の手掛かりで探します。既存の形に重なる一致は、追加した項目のほうが長い場合や手前から始まる場合でも、既存の範囲と手掛かりの強さを保ちます。

文末記号はまたぎません。段落の中で折り返した行は、ほかの語句ルールと同じく、つないで 1 つの文として読みます。既定は段落だけを見て、リスト・表・引用は語句ルールのスコープ設定に従います。同じブロック内で対応する「」・『』・“”・二重引用符の内側に完全に収まる一致と、開き引用符の直後の一致は、語句への言及として除きます。

### なぜ問題か

「単なる〜ではなく」は、誰も主張していない低い見方を先に置き、それを否定して対象を持ち上げる型です。持ち上げた先 (「自分と向き合う時間」「価値ある体験」) が抽象的なままでも、深いことを言ったように見えます。生成された文章では、随筆の結びや解説の段落の頭に繰り返し現れます。ただし、読み手が実際に持ちそうな誤解を正す対比なら、意味のある書き方です。

### 直し方

否定している「単なる〜」が、読み手の実際の誤解に当たるかを確かめます。当たらなければ否定を外し、対象がどう違うのか、何が起きるのかを具体的に書きます。誤解を正す必要がある対比や、定義として区別する必要がある場合は、そのままでかまいません。

### 例

- 直す前: 週に一度の掃除は、単なる作業ではなく、自分と向き合う大切な時間です。
- 直した後: 週に一度の掃除の間は手を動かすだけなので、考えごとがはかどる。

### 根拠

人の文書 440 本、2025 年頃のモデルなどの生成文書 390 本、2026 年のモデルの生成文書 2,576 本で測りました。指摘が出た文書は、人 3 本 (講演資料の「単なる〜にとどまらない」、技術記事、古い随筆)、2025 年頃の生成 34 本 (8.7%)、2026 年の生成 257 本 (10.0%) です。2026 年の生成では、随筆・業務文書・一般向けの解説・技術記事のどのジャンルにも出て、モデルの系統 (Gemini・GPT・Claude ほか) を問いません。言い切る形 (「単なる〜ではない。」) だけでも、人 0 本に対して生成 7 本・42 本です。拾わない疑問・推測の形は、人の文書には出ず、2026 年の生成でも 1 本だけでした。人の書き手も普通に使う言い回しなので、情報で扱います。

手掛かりの強さは「強い」です。人の文書 440 本と生成文書 390 本で測ると、指摘が出た文書は人 3 本・生成 34 本で、生成の割合が人の 3 倍以上・人の割合が 5% 以下という強い手掛かりの基準を満たします。

追加した 2 項目は、人 507 本 (随筆の追加 63 本を含む)・2025 年頃の生成 390 本・2026 年の生成 2,576 本で、同梱辞書を固定して測りました。「単なる〜だけでなく」は人 1 本・生成 1 本と 14 本、「単なる〜に留まらず」は人 0 本・生成 0 本と 4 本です。人の文書の 1 件は、知識に加えて新規性も提供するという正確な対比でした。誤解を正す対比と同じく、意味があれば残せます。

この 2 項目は、既存の型の言い換えを確かめた下調べに基づきます。以前の生成群では合計 5 本に届かず、項目ごとの強さを測れません。2026 年の生成群でも、「だけでなく」は人の割合の 3 倍に届かず、「に留まらず」は 4 本だけです。既存項目の強さを継がず、どちらも中程度にします。新しい文書での再現は今後も確かめる必要があります。",
    },
    // 「単なる」から続きの語までに挟める字は 25 字まで。文末記号と、解析用テキストに残る改行は
    // またがない (段落の中の折り返しは解析用テキストでつながるので、ほかの語句ルールと同じく
    // またいで読む)。下調べで測った範囲から広げない
    entries: &[
        Entry::re(
            r"単なる[^。！？!?\r\n]{1,25}?(?:ではなく|にとどまらず|にとどまらない|以上の|を超え)",
            Severity::Info,
        ),
        // 言い切る形だけを拾い、「〜ではないか」「〜ではないでしょうか」の疑問・推測は拾わない
        Entry::re(
            r"(?P<m>単なる[^。！？!?\r\n]{1,25}?(?:ではない|ではありません|ではなかった))(?:[。、，,！!]|$)",
            Severity::Info,
        ),
        // 観測した言い換えを別の項目で測る。既存項目の強さは継がない。
        Entry::re(r"単なる[^。！？!?\r\n]{1,25}?だけでなく", Severity::Info)
            .with_signal(Signal::Moderate),
        Entry::re(r"単なる[^。！？!?\r\n]{1,25}?に留まらず", Severity::Info)
            .with_signal(Signal::Moderate),
    ],
    message: "「{m}」は、対象をいったん低く置いてから持ち上げる言い回しです",
    hint: "否定している「単なる〜」が読み手の実際の誤解でなければ外し、何がどう違うのかを具体的に書いてください。誤解を正す対比なら残せます",
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::testing::{Options, matched, run, run_with};
    use crate::rules::{Rule, RuleUnit, Scope};

    #[test]
    fn flags_mere_framings_with_exact_spans() {
        for (md, expected) in [
            (
                "掃除は**単なる作業ではなく**、自分と向き合う時間です。",
                "単なる作業ではなく",
            ),
            (
                "このマグカップは単なる食器以上の価値を持つ。",
                "単なる食器以上の",
            ),
            (
                "引き継ぎは単なる手順の伝達にとどまらず、経緯も渡す。",
                "単なる手順の伝達にとどまらず",
            ),
            (
                "窓の外の景色は、単なる背景ではありません。",
                "単なる背景ではありません",
            ),
            (
                "それは単なる偶然ではなかった、と今は思う。",
                "単なる偶然ではなかった",
            ),
            ("その店は単なる「本屋」ではない", "単なる「本屋」ではない"),
        ] {
            let findings = run(&rule(), md);
            assert_eq!(matched(md, &findings), [expected], "{md}");
            assert_eq!(findings[0].severity, Severity::Info);
            assert_eq!(findings[0].signal, Some(Signal::Strong));
        }
    }

    #[test]
    fn ignores_questions_other_uses_and_long_gaps() {
        for md in [
            // 疑問・推測
            "これは単なる誤記ではないか。",
            "単なる偶然ではないでしょうか。",
            "単なる偶然ではないだろうか。",
            // 否定や比較の続かない「単なる」
            "単なる思いつきで決めた。",
            "単なる確認なので、すぐ終わる。",
            // 文をまたがない・25 字を超えて離れた語は結ばない
            "単なる確認です。作業ではなく、見直しです。",
            "単なる、あいだにとても長い説明を挟んで二十六字よりも長く離れた文ではなく、続く。",
            // コード
            "`単なる作業ではなく` は例です。",
        ] {
            assert!(run(&rule(), md).is_empty(), "{md}");
        }
    }

    #[test]
    fn a_quoted_mention_does_not_hide_a_later_framing() {
        let md = "「単なる作業」と、これは単なる道具ではなく、体験だ。";
        assert_eq!(matched(md, &run(&rule(), md)), ["単なる道具ではなく"]);
    }

    #[test]
    fn reads_wrapped_lines_in_a_paragraph_as_one_sentence() {
        let md = "掃除は単なる作業\nではなく、自分と向き合う時間です。";
        assert_eq!(matched(md, &run(&rule(), md)), ["単なる作業\nではなく"]);
    }

    #[test]
    fn follows_phrase_scope_and_accepts_fragments() {
        let md = "# 単なる作業ではない\n\n- 単なる作業ではなく\n\n> 単なる作業ではなく\n";
        let rule = rule();
        assert!(run(&rule, md).is_empty());
        let all = Options {
            scope: Scope::ALL,
            ..Options::default()
        };
        assert_eq!(
            matched(md, &run_with(&rule, md, all)),
            ["単なる作業ではなく"; 2]
        );
        assert_eq!(rule.unit(), RuleUnit::Sentence);
    }

    #[test]
    fn flags_observed_variants_with_moderate_signals_and_exact_spans() {
        for (md, expected) in [
            (
                "改装は**単なる設備の交換だけでなく**、町の新しい出発でもある。",
                "単なる設備の交換だけでなく",
            ),
            (
                "点検は単なる手順の確認に留まらず、経験を伝える機会にもなる。",
                "単なる手順の確認に留まらず",
            ),
            (
                "改装は単なる設備の交換\nだけでなく、町の新しい出発でもある。",
                "単なる設備の交換\nだけでなく",
            ),
        ] {
            let findings = run(&rule(), md);
            assert_eq!(matched(md, &findings), [expected], "{md}");
            assert_eq!(findings[0].severity, Severity::Info);
            assert_eq!(findings[0].signal, Some(Signal::Moderate));
        }
    }

    #[test]
    fn variants_respect_sentence_boundaries_and_quoted_mentions() {
        for md in [
            "単なる作業です。手順の確認だけでなく、記録も残す。",
            "単なる作業です。手順の確認に留まらず、記録も残す。",
            "「単なる作業だけでなく」という表現を調べた。",
            "「単なる作業に留まらず」という表現を調べた。",
            "`単なる作業だけでなく` は例です。",
            "単なる、あいだにとても長い説明を挟んで二十六字よりも長く離れた作業だけでなく、記録もある。",
            "ただの作業だけでなく、記録もある。",
            "単なる作業に留まる。",
        ] {
            assert!(run(&rule(), md).is_empty(), "{md}");
        }
        let md = "「単なる作業」と違い、点検は単なる確認だけでなく、記録も残す。";
        assert_eq!(matched(md, &run(&rule(), md)), ["単なる確認だけでなく"]);
    }

    #[test]
    fn variants_follow_scope_and_leave_original_signals_unchanged() {
        let rule = rule();
        let md = "- 単なる作業だけでなく\n\n> 単なる作業に留まらず\n";
        assert!(run(&rule, md).is_empty());
        let findings = run_with(
            &rule,
            md,
            Options {
                scope: Scope::ALL,
                ..Options::default()
            },
        );
        assert_eq!(
            matched(md, &findings),
            ["単なる作業だけでなく", "単なる作業に留まらず"]
        );
        assert!(findings.iter().all(|d| d.signal == Some(Signal::Moderate)));
        let md = "単なる作業ではなく、経験だ。単なる確認だけでなく、記録も残す。";
        let findings = run(&rule, md);
        assert_eq!(
            matched(md, &findings),
            ["単なる作業ではなく", "単なる確認だけでなく"]
        );
        assert_eq!(findings[0].signal, Some(Signal::Strong));
        assert_eq!(findings[1].signal, Some(Signal::Moderate));
    }

    #[test]
    fn a_longer_variant_keeps_the_original_hit_and_later_nested_starts() {
        let md = "単なる作業ではなく、単なる確認だけでなく、経験だ。";
        let findings = run(&rule(), md);
        assert_eq!(
            matched(md, &findings),
            ["単なる作業ではなく", "単なる確認だけでなく"]
        );
        assert_eq!(findings[0].signal, Some(Signal::Strong));
        assert_eq!(findings[1].signal, Some(Signal::Moderate));
        // 既存項目どうしは最長を選ぶ。追加の終端はその既存範囲を延ばさない。
        let md = "単なる作業ではなく、確認ではない。";
        let findings = run(&rule(), md);
        assert_eq!(matched(md, &findings), ["単なる作業ではなく、確認ではない"]);
        assert_eq!(findings[0].signal, Some(Signal::Strong));
        for (md, expected) in [
            (
                "これは単なる準備ではなく、知識だけでなく経験も伝える仕事だ。",
                "単なる準備ではなく",
            ),
            (
                "これは単なる準備ではなく知識だけでなく経験も伝える仕事だ。",
                "単なる準備ではなく",
            ),
            (
                "この会は単なる集会ではなく、一時の相談に留まらず続いている。",
                "単なる集会ではなく",
            ),
            (
                "単なる記録だけでなく判断の材料になるのは偶然ではない。",
                "単なる記録だけでなく判断の材料になるのは偶然ではない",
            ),
        ] {
            let findings = run(&rule(), md);
            assert_eq!(matched(md, &findings), [expected], "{md}");
            assert_eq!(findings[0].signal, Some(Signal::Strong));
        }
    }

    #[test]
    fn an_earlier_variant_cannot_hide_an_overlapping_original_start() {
        let md = "単なる思いつきや単なる記録だけでなく判断の材料になるのは偶然ではない。";
        let findings = run(&rule(), md);
        assert_eq!(
            matched(md, &findings),
            ["単なる記録だけでなく判断の材料になるのは偶然ではない"]
        );
        assert_eq!(findings[0].signal, Some(Signal::Strong));
    }
}
