//! P21: 誰の研究・主張かを示さずに権威を借りる文頭の型。

use std::sync::LazyLock;

use regex::Regex;

use crate::diagnostic::{Diagnostic, Lane, RuleStatus, Severity};
use crate::document::{Block, Document, MarkKind, Sentence};
use crate::rules::{Rule, RuleContext, RuleMeta, RuleUnit, quote};

use super::engine::WEAK_SIGNAL_NOTE;

static META: RuleMeta = RuleMeta {
    id: "P21",
    name: "VAGUE_ATTRIBUTION",
    title: "出典をぼかした権威付け",
    lane: Lane::Slop,
    status: RuleStatus::Experimental,
    default_severity: Severity::Info,
    summary: "誰の研究・主張かを示さずに権威を借りる文頭の型を、出典の確認候補として指摘する (実験的)",
    explanation: r"### 何を見るか

文頭の「専門家は〜と指摘しています」「研究者によると」「研究によって明らかになっています」「調査では〜が示されています」などを探します。「一部の」「多くの」「複数の」「最近の」を冠する形も含みます。主語から述語までの間は 40 字以内で、引用符をまたぎません。単なる「研究では」や「と言われています」は拾いません。

同じ文にリンク・URL・脚注参照・著者年形式の引用がある場合は除きます。同じ段落の直前・直後の文が「出典」「参考文献」「参照」「詳しくは」で始まり、これらの参照を含む場合も除きます。「青葉大学の研究では」のように具体名から始まる文はこの型に含みません。リンクが主張を裏付けるか、別の節に出典があるかまでは判断しません。

### なぜ問題か

名前のない専門家や研究を根拠にすると、読み手が主張を確かめられません。ただし、具体的な出典を後でまとめる書き方もあるため、根拠がないとは断定しません。

### 直し方

誰の、いつの研究・発言かを確認し、必要なら出典を添えてください。本文から出典にたどれる場合は、そのまま残して構いません。

### 例

- 直す前: 専門家は、受付時間の短縮が有効だと指摘しています。
- 直した後: 当施設の受付記録では、窓口を二つに増やした週に待ち時間が減った。

### 根拠

未校正の実験的なルールです。Wikipedia の編集者向け観察集 Signs of AI writing の Vague attributions and overgeneralization of opinions を着想にしています (https://en.wikipedia.org/wiki/Wikipedia:Signs_of_AI_writing)。日本語の検出条件は独自に作ったもので、誤検知率は測っていません。人も使う表現のため、弱い手掛かりとして情報で扱います。",
};

static ATTRIBUTION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
    r"^(?:(?:一部|多く|複数|最近)の)?(?:",
    r"(?:専門家|研究者|識者)(?:によると[、，]|(?:は|が)[^。！？!?「」『』\n]{0,40}?(?:指摘|主張|説明|強調)(?:しています|している|しました|した|する))",
    r"|(?:研究|調査)(?:によって|により|では|から)[^。！？!?「」『』\n]{0,40}?(?:明らかになっています|明らかになっている|示されています|示されている|確認されています|確認されている))",
)).expect("attribution regex")
});

static REFERENCE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
    r"https?://|\[\^[^\]\s]+\]|\[[0-9０-９]+\]|[（(][^()（）\n]{1,60}[、,， ](?:19|20)[0-9]{2}[a-z]?[)）]",
).expect("reference regex")
});

fn has_reference(doc: &Document, block: &Block, sentences: &[Sentence], i: usize) -> bool {
    // 句点直後の脚注は Sentence の範囲外になるので、次の文の手前までを見る。
    let sentence = &sentences[i];
    let end = sentences
        .get(i + 1)
        .map_or(block.span.end, |s| s.span.start);
    REFERENCE.is_match(&doc.source[sentence.span.start..end])
        || block.marks.iter().any(|m| {
            m.kind == MarkKind::Link && m.span.start < end && sentence.span.start < m.span.end
        })
}

fn reference_note(doc: &Document, block: &Block, sentences: &[Sentence], i: usize) -> bool {
    ["出典", "参考文献", "参照", "詳しくは"]
        .iter()
        .any(|prefix| doc.sentence_text(&sentences[i]).starts_with(prefix))
        && has_reference(doc, block, sentences, i)
}

pub struct VagueAttribution;

impl Rule for VagueAttribution {
    fn unit(&self) -> RuleUnit {
        RuleUnit::Sentence
    }

    fn meta(&self) -> &'static RuleMeta {
        &META
    }

    fn check(&self, ctx: &RuleContext<'_>, out: &mut Vec<Diagnostic>) {
        for (idx, block) in ctx.scoped_blocks() {
            if block.in_footnote {
                continue;
            }
            let sentences = ctx.doc.block_sentences(idx);
            for (i, sentence) in sentences.iter().enumerate() {
                let value = ctx.doc.sentence_text(sentence);
                let Some(hit) = ATTRIBUTION.find(value) else {
                    continue;
                };
                // 問い・否定・表現そのものへの言及は、肯定的な権威付けとは扱わない。
                let rest = &value[hit.end()..];
                if !rest.is_empty()
                    && !rest.starts_with(['。', '、', '，'])
                    && !hit.as_str().contains("によると")
                {
                    continue;
                }
                if value.contains(['?', '？'])
                    || value.ends_with("でしょうか。")
                    || has_reference(ctx.doc, block, sentences, i)
                    || i.checked_sub(1)
                        .is_some_and(|p| reference_note(ctx.doc, block, sentences, p))
                    || (i + 1 < sentences.len() && reference_note(ctx.doc, block, sentences, i + 1))
                {
                    continue;
                }
                let span = block.to_source(
                    sentence.range.start + hit.start()..sentence.range.start + hit.end(),
                );
                out.push(META.diagnostic(span, format!("「{}」の出典を確認してください", quote(hit.as_str())))
                    .with_hint(format!("誰の研究・発言か、本文から出典をたどれるか確認してください。{WEAK_SIGNAL_NOTE}"))
                    .with_context(sentence.span)
                    .with_metric("item", "匿名の研究・専門家への帰属")
                    .with_metric("matched", hit.as_str()));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::{
        Scope,
        testing::{Options, matched, run, run_with},
    };

    #[test]
    fn flags_anonymous_claims_with_exact_source_positions() {
        let md = "前置きです。\n\n**専門家**は、受付の短縮が有効だと指摘しています。\n";
        let d = run(&VagueAttribution, md);
        assert_eq!(d.len(), 1);
        assert_eq!(
            matched(md, &d),
            vec!["専門家**は、受付の短縮が有効だと指摘しています"]
        );
        assert_eq!(d[0].severity, Severity::Info);
        for s in [
            "研究によって明らかになっています。",
            "複数の調査では効果が示されています。",
            "研究者によると、差は小さい。",
        ] {
            assert_eq!(run(&VagueAttribution, s).len(), 1, "{s}");
        }
    }

    #[test]
    fn accepts_named_attribution_references_and_non_assertions() {
        for s in [
            "青葉大学の研究では効果が示されています。",
            "専門家は誰かを調べています。",
            "研究者によるところが大きい。",
            "専門家は有効だと指摘していません。",
            "専門家は有効だと指摘していますか？",
            "専門家は有効だと指摘するとは限らない。",
            "専門家は有効だと指摘しています（山田, 2024）。",
            "専門家は[報告](https://example.com)で指摘しています。",
            "専門家は有効だと指摘しています。[^report]\n\n[^report]: 記録。\n",
            "専門家は有効だと指摘しています。出典は[報告](https://example.com)です。",
            "参照: https://example.com\n専門家は有効だと指摘しています。",
        ] {
            assert!(run(&VagueAttribution, s).is_empty(), "{s}");
        }
    }

    #[test]
    fn unrelated_links_and_numbers_do_not_supply_a_source() {
        for s in [
            "専門家は3倍になると指摘しています。",
            "[申請窓口](https://example.com)はこちらです。別の説明です。専門家は有効だと指摘しています。",
            "専門家は有効だと指摘しています。\n\n出典は[報告](https://example.com)です。",
        ] {
            assert_eq!(run(&VagueAttribution, s).len(), 1, "{s}");
        }
    }

    #[test]
    fn follows_phrase_scope() {
        let md = "# 専門家は有効だと指摘しています。\n\n- 専門家は有効だと指摘しています。\n\n> 研究によって明らかになっています。\n";
        assert!(run(&VagueAttribution, md).is_empty());
        assert_eq!(
            run_with(
                &VagueAttribution,
                md,
                Options {
                    scope: Scope::ALL,
                    ..Options::default()
                }
            )
            .len(),
            2
        );
    }
}
