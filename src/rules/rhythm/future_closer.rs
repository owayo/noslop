//! R15: 文書の結びの「課題は残る → 今後に期待する」という型。

use std::sync::LazyLock;

use regex::Regex;

use crate::diagnostic::{Diagnostic, Lane, RuleStatus, Severity, Span};
use crate::rules::{Rule, RuleContext, RuleMeta};

static META: RuleMeta = RuleMeta {
    id: "R15",
    name: "FORMULAIC_FUTURE_CLOSER",
    title: "課題から期待へ流す結び",
    lane: Lane::Slop,
    status: RuleStatus::Experimental,
    default_severity: Severity::Info,
    summary: "文書の結びで「課題は残る」から「今後の発展が期待される」へ流す型を指摘する (実験的)",
    explanation: r"### 何を見るか

文書の最後の地の文で、「課題は残ります」「課題がある」などの文の直後に、「今後」「将来」「これから」を含み、「発展」「進展」「成長」「改善」「活用」「普及」への期待で終わる文が続く型を探します。同じ文の中で読点を挟んで続く型も拾います。別段落の場合は最後の 2 段落が空白だけを挟んで隣接している必要があります。見出し・箇条書き・コード・引用を挟んでつなぎません。

否定や問い、具体的な数値を含む結び、コードやリンクなどを含む結びは対象外です。文書の途中の展望や「課題」「期待」という語だけでは指摘しません。

### なぜ問題か

課題を挙げたあとに漠然とした期待を添える型は、何を解決すればよいのかを示さずに文章を閉じられます。ただし、研究や事業の展望として必要な場合もあります。

### 直し方

期待の一文が必要かを確かめ、残すなら次に確かめることや取り組むことを書いてください。書かれていない期限や成果を補ってはいけません。

### 例

- 直す前: 運用には課題が残ります。しかし、今後の普及が期待されます。
- 直した後: 運用上の課題は、夜間の問い合わせに対応できないことです。

### 根拠

未校正の実験的なルールです。Wikipedia の編集者向け観察集 Signs of AI writing の Outline-like conclusions about challenges and future prospects を着想にしています (https://en.wikipedia.org/wiki/Wikipedia:Signs_of_AI_writing)。日本語での型と除外条件は独自のもので、誤検知率は測っていません。情報として確認を促します。",
};

static CHALLENGE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
    r"課題(?:は|が|も)(?:まだ)?(?:残(?:る|ります|っている|っています)|ある|あります)(?:が|ものの)?[、，。]?\s*$",
).expect("challenge regex")
});

static FUTURE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
    r"^(?:しかし[、，]?|それでも[、，]?|一方で[、，]?)?(?:今後|将来|これから)[^。！？!?「」『』]{0,40}(?:発展|進展|成長|改善|活用|普及)(?:が|も)(?:大いに)?期待され(?:る|ます)[。]?\s*$",
).expect("future regex")
});

pub struct FormulaicFutureCloser;

impl Rule for FormulaicFutureCloser {
    fn meta(&self) -> &'static RuleMeta {
        &META
    }

    fn check(&self, ctx: &RuleContext<'_>, out: &mut Vec<Diagnostic>) {
        let doc = ctx.doc;
        let Some(last) = doc.sentences.last() else {
            return;
        };
        let block = &doc.blocks[last.block];
        if !last.japanese
            || !block.is_prose()
            || doc.blocks.len() != last.block + 1
            || !doc.source[block.span.end..].trim().is_empty()
        {
            return;
        }
        let value = doc.sentence_text(last);
        // 1 文型は読点の後ろから期待の型が始まるものだけ。
        let within = value
            .char_indices()
            .filter(|(_, c)| matches!(c, '、' | '，'))
            .find_map(|(i, c)| {
                let (before, after) = value.split_at(i + c.len_utf8());
                (CHALLENGE.is_match(before) && FUTURE.is_match(after.trim_start()))
                    .then_some(last.span)
            });
        let pair = if within.is_none() {
            doc.sentences.iter().rev().nth(1).and_then(|first| {
                let previous = &doc.blocks[first.block];
                let adjacent = first.block == last.block
                    || (first.block + 1 == last.block
                        && doc.source[previous.span.end..block.span.start]
                            .trim()
                            .is_empty());
                (first.japanese
                    && previous.is_prose()
                    && adjacent
                    && CHALLENGE.is_match(doc.sentence_text(first))
                    && FUTURE.is_match(value))
                .then_some(first.span)
            })
        } else {
            None
        };
        let Some(first) = within.or(pair) else {
            return;
        };
        let context = Span::new(first.start, last.span.end);
        let source = doc.slice(context);
        if source.chars().any(|c| {
            c.is_numeric() || matches!(c, '?' | '？' | '「' | '」' | '『' | '』' | '`' | '<' | '>')
        }) || source.contains("http")
            || source.contains("[^")
            || doc
                .blocks
                .iter()
                .skip(
                    doc.sentences
                        .iter()
                        .find(|s| s.span == first)
                        .map_or(last.block, |s| s.block),
                )
                .any(|b| {
                    b.marks.iter().any(|m| {
                        m.span.start < context.end
                            && context.start < m.span.end
                            && matches!(
                                m.kind,
                                crate::document::MarkKind::Link
                                    | crate::document::MarkKind::Code
                                    | crate::document::MarkKind::Math
                                    | crate::document::MarkKind::Image
                            )
                    })
                })
        {
            return;
        }
        let related = if first == last.span {
            Vec::new()
        } else {
            vec![last.span]
        };
        out.push(META.diagnostic(first, "課題を挙げた後、今後への期待で文書を閉じる定型です")
            .with_hint("期待の一文が必要か確かめ、残すなら次に確かめることや取り組むことを書いてください")
            .with_context(context).with_related(related));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::testing::{matched, run};

    const PAIR: &str = "運用には課題が残ります。しかし、今後の普及が期待されます。";

    #[test]
    fn detects_final_pairs_and_one_sentence_form() {
        let d = run(&FormulaicFutureCloser, PAIR);
        assert_eq!(matched(PAIR, &d), vec!["運用には課題が残ります。"]);
        assert_eq!(d[0].related.len(), 1);
        for s in [
            "課題は残るが、今後の発展が期待される。",
            "課題があります。\n\n将来の改善が期待されます。\n",
        ] {
            assert_eq!(run(&FormulaicFutureCloser, s).len(), 1, "{s}");
        }
    }

    #[test]
    fn ignores_facts_negation_mid_document_and_structural_gaps() {
        for s in [
            "課題は解決しました。しかし、今後の普及が期待されます。".into(),
            "課題は残る。しかし、今後の普及は期待されない。".into(),
            "課題は残る。しかし、今後3年間の普及が期待される。".into(),
            "課題は残る。来月までに受付を増やす。".into(),
            format!("{PAIR}次の節で手順を説明する。"),
            "課題は残る。\n\n## 次の節\n\n今後の発展が期待される。".into(),
            "課題は残る。\n\n```\n作業\n```\n\n今後の発展が期待される。".into(),
            format!("- {PAIR}\n"),
            format!("> {PAIR}\n"),
            format!("{PAIR}\n\n```\n作業\n```\n"),
            "「課題は残る。しかし、今後の普及が期待される。」".into(),
        ] {
            assert!(run(&FormulaicFutureCloser, &s).is_empty(), "{s}");
        }
    }
}
