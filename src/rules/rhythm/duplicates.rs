//! R14: 文・段落の文字列としての重複。意味の近さは判定しない。

use std::collections::{HashMap, HashSet};

use crate::diagnostic::{Diagnostic, Lane, Severity, Span};
use crate::document::{Block, Document, MarkKind};
use crate::rules::{Fires, Measure, Rule, RuleContext, RuleMeta};
use crate::text;

use super::{option_count, unknown_option};

static META: RuleMeta = RuleMeta {
    id: "R14",
    name: "DUPLICATE_PASSAGE",
    title: "文・段落の重複",
    lane: Lane::Readability,
    signal: None,
    default_severity: Severity::Info,
    summary: "同じ文または段落が再登場する箇所を、初出の位置とともに指摘する",
    explanation: r"### 何を見るか

地の文で、30 字以上 (空白と末尾の文末記号を除く。min_chars で変更可能) の同じ文・段落が再登場した箇所を指します。連続する空白だけを半角空白 1 字にそろえ、語句・数字・否定・句読点は変えずに比べます。段落の重複を指した場合、その中の文は重ねて指しません。初出は関連箇所とメッセージの行・列で示します。

コード・数式・画像・リンク・取り消し線を含む範囲、引用符「」『』で全体を囲んだ範囲、脚注・引用ブロックは対象外です。解析用本文から除かれる HTML・URL・コメントなどを含む範囲も除外します。言い換えによる意味の重複は判定しません。

### なぜ問題か

同じ説明が繰り返されると、読み手は新しい条件や事実が追加されたのかを確かめ直すことになります。AI が書いたかどうかとは別に、読みやすさの問題として扱います。

### 直し方

初出と比較し、要らない繰り返しなら削ってください。節ごとに必要な注意書きや意図した再掲なら残し、抑制コメントで理由を書いてください。

### 例

- 直す前: 冒頭と末尾に「申請書は提出前に担当者が記入漏れと添付資料の不足を確認してください。」を置く。
- 直した後: 冒頭の説明を残し、末尾の同じ文を削る。

### 根拠

一致の条件と 30 字は暫定の値で、校正していません。人の文書にも意図した再掲があるため、情報として示します。",
};

#[derive(Clone, Copy)]
struct Duplicate {
    span: Span,
    first: Span,
    chars: usize,
    paragraph: bool,
    block: usize,
}

/// ブロックの中で、比較から外す装飾 (コード・数式・画像・リンク・取り消し線) の原文上の範囲。
///
/// 始まりの順に並べ、先頭からの終わりの最大を持つ。文ごとに全部の装飾を調べ直さず、範囲と
/// 重なる装飾があるかを二分探索で調べるため (太字などの装飾が多い長い段落で、文の数と装飾の数の
/// 積の手間になっていた)。
struct ErasedMarks {
    /// 装飾の始まり (昇順)。
    starts: Vec<usize>,
    /// `starts` の先頭からその位置までの装飾の、終わりの最大。
    max_ends: Vec<usize>,
}

impl ErasedMarks {
    fn new(block: &Block) -> Self {
        let mut spans: Vec<Span> = block
            .marks
            .iter()
            .filter(|m| {
                matches!(
                    m.kind,
                    MarkKind::Code
                        | MarkKind::Math
                        | MarkKind::Image
                        | MarkKind::Link
                        | MarkKind::Strikethrough
                )
            })
            .map(|m| m.span)
            .collect();
        spans.sort_unstable_by_key(|s| s.start);
        let starts = spans.iter().map(|s| s.start).collect();
        let max_ends = spans
            .iter()
            .scan(0, |max, s| {
                *max = s.end.max(*max);
                Some(*max)
            })
            .collect();
        Self { starts, max_ends }
    }

    /// 原文上の範囲 `span` に重なる装飾があるか (装飾の始まりが範囲の終わりより前で、終わりが
    /// 範囲の始まりより後ろ)。
    fn overlaps(&self, span: Span) -> bool {
        // 始まりが範囲の終わりより前の装飾は、並べた先頭の `before` 個
        let before = self.starts.partition_point(|&start| start < span.end);
        before > 0 && self.max_ends[before - 1] > span.start
    }
}

/// 文の全体を引用符「」『』で囲んでいるか。先頭の引用符に対応する閉じ (同じ種類の入れ子を数えて
/// 求める) が末尾にあるときだけ囲んだとみなす (「申請書」は〜「重要書類」のように、一部ずつを
/// 囲んだ文は外さない)。
fn wholly_quoted(body: &str) -> bool {
    let mut chars = body.char_indices();
    let Some((_, open)) = chars.next() else {
        return false;
    };
    let close = match open {
        '「' => '」',
        '『' => '』',
        _ => return false,
    };
    let mut depth = 1usize;
    for (i, c) in chars {
        if c == open {
            depth += 1;
        } else if c == close {
            depth -= 1;
            if depth == 0 {
                return i + c.len_utf8() == body.len();
            }
        }
    }
    false
}

/// 解析時に消えた内容の違いを同一視しない。装飾だけは比較してよい。
fn comparable(doc: &Document, erased: &ErasedMarks, span: Span, value: &str) -> bool {
    let source = doc.slice(span);
    let body = value.trim_end_matches(|c: char| text::is_sentence_ender(c) || c.is_whitespace());
    !value.contains(text::PLACEHOLDER)
        && !source.contains(['<', '>', '\\'])
        && !source.contains("http://")
        && !source.contains("https://")
        && !source.contains("[^")
        && !wholly_quoted(body)
        && !erased.overlaps(span)
}

fn duplicates(doc: &Document) -> Vec<Duplicate> {
    let mut found = Vec::new();
    // 文と段落は別々に比較する。1 文だけの段落の二重指摘は check で除く。
    for paragraph in [true, false] {
        let mut seen: HashMap<String, Span> = HashMap::new();
        for (idx, block) in doc.blocks.iter().enumerate().filter(|(_, b)| b.is_prose()) {
            let erased = ErasedMarks::new(block);
            let units: Vec<(Span, &str)> = if paragraph {
                vec![(block.to_source(0..block.text.len()), block.text.as_str())]
            } else {
                doc.sentences[block.sentences.clone()]
                    .iter()
                    .map(|s| (s.span, doc.sentence_text(s)))
                    .collect()
            };
            for (span, value) in units {
                if !text::contains_japanese(value) || !comparable(doc, &erased, span, value) {
                    continue;
                }
                let key = value.split_whitespace().collect::<Vec<_>>().join(" ");
                let chars = text::reading_length(&key);
                if chars == 0 {
                    continue;
                }
                if let Some(&first) = seen.get(&key) {
                    found.push(Duplicate {
                        span,
                        first,
                        chars,
                        paragraph,
                        block: idx,
                    });
                } else {
                    seen.insert(key, span);
                }
            }
        }
    }
    found
}

pub struct DuplicatePassage {
    min_chars: usize,
}

impl Default for DuplicatePassage {
    fn default() -> Self {
        Self { min_chars: 30 }
    }
}

impl Rule for DuplicatePassage {
    fn meta(&self) -> &'static RuleMeta {
        &META
    }

    fn configure(&mut self, key: &str, value: &toml::Value) -> Result<(), String> {
        match key {
            "min_chars" => self.min_chars = option_count(key, value)?,
            _ => return Err(unknown_option(&META, key)),
        }
        Ok(())
    }

    fn options(&self) -> Vec<(&'static str, String)> {
        vec![("min_chars", self.min_chars.to_string())]
    }

    fn measure(&self, ctx: &RuleContext<'_>) -> Vec<Measure> {
        duplicates(ctx.doc)
            .iter()
            .map(|d| d.chars)
            .max()
            .map_or_else(Vec::new, |n| {
                vec![Measure::new(
                    "duplicate_chars",
                    n as f64,
                    "min_chars",
                    Fires::AtOrAbove,
                )]
            })
    }

    fn check(&self, ctx: &RuleContext<'_>, out: &mut Vec<Diagnostic>) {
        let found: Vec<_> = duplicates(ctx.doc)
            .into_iter()
            .filter(|d| d.chars >= self.min_chars)
            .collect();
        let paragraphs: HashSet<_> = found
            .iter()
            .filter(|d| d.paragraph)
            .map(|d| d.block)
            .collect();
        for d in &found {
            if !d.paragraph && paragraphs.contains(&d.block) {
                continue;
            }
            let (line, column) = ctx.doc.line_col(d.first.start);
            let kind = if d.paragraph { "段落" } else { "文" };
            out.push(META.diagnostic(d.span, format!("同じ{kind}が再登場しています (初出は {line} 行 {column} 列)"))
                .with_hint("初出と比べ、要らない繰り返しなら削ってください。意図した再掲なら残してください")
                .with_context(d.span).with_related(vec![d.first])
                .with_metric("duplicate_chars", d.chars));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::testing::{matched, run};

    const LONG: &str = "申請書は提出前に担当者が記入漏れと添付資料の不足を確認してください。";

    #[test]
    fn points_to_duplicate_and_original_without_double_reporting() {
        let md = format!("{LONG}\n\n別の説明を挟む。\n\n{LONG}\n\n{LONG}\n");
        let d = run(&DuplicatePassage::default(), &md);
        assert_eq!(matched(&md, &d), vec![LONG, LONG]);
        assert_eq!(d[0].span.start, md.rfind(&format!("{LONG}\n\n")).unwrap());
        assert!(
            d.iter()
                .all(|d| d.related == vec![Span::new(0, LONG.len())])
        );
        assert!(d[0].message.contains("1 行 1 列"));
    }

    #[test]
    fn detects_sentences_and_paragraphs_of_short_sentences() {
        let md = format!("{LONG}締め日は月末です。\n\n受付は午前です。{LONG}\n");
        assert_eq!(
            matched(&md, &run(&DuplicatePassage::default(), &md)),
            vec![LONG]
        );
        let paragraph = "窓口は東側です。受付は九時です。午後は閉めます。翌日は休みです。";
        let md = format!("{paragraph}\n\n{paragraph}\n");
        assert_eq!(
            matched(&md, &run(&DuplicatePassage::default(), &md)),
            vec![paragraph]
        );
    }

    #[test]
    fn preserves_meaningful_differences_and_ignores_non_prose() {
        for other in [
            LONG.replace("確認して", "確認しないで"),
            LONG.replace("担当者", "管理者"),
        ] {
            assert!(
                run(
                    &DuplicatePassage::default(),
                    &format!("{LONG}\n\n{other}\n")
                )
                .is_empty()
            );
        }
        for md in [
            format!("- {LONG}\n- {LONG}\n"),
            format!("> {LONG}\n\n> {LONG}\n"),
            format!("# {LONG}\n\n# {LONG}\n"),
            format!("```\n{LONG}\n{LONG}\n```\n"),
            format!("「{LONG}」\n\n「{LONG}」\n"),
        ] {
            assert!(run(&DuplicatePassage::default(), &md).is_empty(), "{md}");
        }
        let mut rule = DuplicatePassage::default();
        rule.configure("min_chars", &toml::Value::Integer(1))
            .unwrap();
        assert!(run(&rule, "番号は1 2です。番号は12です。\n").is_empty());
        assert!(run(&rule, "申請は10件です。申請は11件です。\n").is_empty());
    }

    #[test]
    fn excludes_only_passages_wholly_enclosed_in_quotes() {
        // 一部の語だけを「」で囲んだ文は、先頭と末尾が引用符でも比べる
        let partly = "「申請書」は提出前に担当者が記入漏れと添付資料の不足を確認する「重要書類」";
        let md = format!("{partly}\n\n別の説明を挟む。\n\n{partly}\n");
        let d = run(&DuplicatePassage::default(), &md);
        assert_eq!(matched(&md, &d), vec![partly]);
        assert_eq!(d[0].span.start, md.rfind(partly).unwrap());
        assert_eq!(d[0].related, vec![Span::new(0, partly.len())]);
        // 全体を囲んだ文は、中に別の引用や同じ種類の入れ子があっても外す
        for whole in [
            format!("「{LONG}」"),
            "「申請書は『重要書類』なので提出前に担当者が記入漏れと添付資料の不足を確認する」"
                .to_string(),
            "「申請書の「重要書類」の欄は提出前に担当者が記入漏れと添付資料の不足を確認する」"
                .to_string(),
            format!("『{LONG}』"),
        ] {
            let md = format!("{whole}\n\n{whole}\n");
            assert!(run(&DuplicatePassage::default(), &md).is_empty(), "{md}");
        }
        assert!(wholly_quoted("「a「b」c」"));
        assert!(!wholly_quoted("「a」c「b」"));
        assert!(!wholly_quoted("「a「b」c"));
        assert!(!wholly_quoted("a「b」"));
        assert!(!wholly_quoted(""));
    }

    #[test]
    fn erased_marks_find_the_same_overlaps_as_a_full_scan() {
        // 入れ子や重なりのある装飾で、二分探索の判定が全件の走査と一致する
        let md = "前置き`code`と[リンク](https://example.com/a)と~~消した語~~と$x$と**太字**を並べ、`a`の後に[`b`](https://example.com/b)を書いた。\n";
        let doc = Document::markdown(md);
        let block = &doc.blocks[0];
        let erased = ErasedMarks::new(block);
        assert!(!erased.starts.is_empty());
        let brute = |span: Span| {
            block.marks.iter().any(|m| {
                m.span.start < span.end
                    && span.start < m.span.end
                    && !matches!(m.kind, MarkKind::Strong | MarkKind::Emphasis)
            })
        };
        let bounds: Vec<usize> = (0..=md.len()).filter(|&i| md.is_char_boundary(i)).collect();
        for (i, &start) in bounds.iter().enumerate() {
            for &end in &bounds[i..] {
                let span = Span::new(start, end);
                assert_eq!(erased.overlaps(span), brute(span), "{start}..{end}");
            }
        }
        // 外す装飾のないブロック
        let doc = Document::markdown("**太字**だけの文。\n");
        let erased = ErasedMarks::new(&doc.blocks[0]);
        assert!(!erased.overlaps(Span::new(0, doc.source.len())));
    }

    #[test]
    fn does_not_compare_erased_content_but_maps_emphasis() {
        for (first, second) in [
            ("`許可`", "`拒否`"),
            ("$a$", "$b$"),
            ("![許可](a.png)", "![拒否](b.png)"),
            (
                "[手順](https://example.com/a)",
                "[手順](https://example.com/b)",
            ),
            ("~~許可~~", "許可"),
            ("許可<!-- 注記 -->", "許可"),
        ] {
            let md = format!(
                "申請書は提出前に{first}の記入漏れと添付資料の不足を確認してください。\n\n申請書は提出前に{second}の記入漏れと添付資料の不足を確認してください。\n"
            );
            assert!(run(&DuplicatePassage::default(), &md).is_empty(), "{md}");
        }
        for md in [format!("{LONG}`one`\n\n{LONG}`two`\n"),
            "詳細は[案内](https://example.com/a)です。\n\n詳細は[案内](https://example.com/b)です。\n".into(),
            format!("{LONG}<span>甲</span>\n\n{LONG}<span>乙</span>\n")] {
            // 段落は除外し、コードの前で既に完結した同一文だけは比較できる。
            let d = run(&DuplicatePassage::default(), &md);
            assert!(d.iter().all(|d| !d.message.contains("同じ段落")));
        }
        let md = format!(
            "{LONG}\n\n申請書は**提出前**に担当者が記入漏れと添付資料の不足を確認してください。\n"
        );
        let d = run(&DuplicatePassage::default(), &md);
        assert_eq!(d.len(), 1);
        assert!(matched(&md, &d)[0].contains("**提出前**"));
    }
}
