//! P23: 太字にならずに記号のまま残った Markdown の強調記法。
//!
//! Markdown の文書では、描画される強調の記号は解析用テキストから外れるので、解析用テキストに
//! 残った `**` の対は描画されなかったもの (括弧に接した `**「語句」**に` など) になる。テキスト・
//! コードの普通のコメント・gws で書き込む値は Markdown として読まないので、記号はすべて残る。

use std::collections::HashSet;
use std::ops::Range;

use crate::diagnostic::{Diagnostic, Lane, RuleStatus, Severity};
use crate::document::SourceFormat;
use crate::rules::{Rule, RuleContext, RuleMeta, RuleUnit, quote};
use crate::text;

use super::engine::diagnostic;

static P23_META: RuleMeta = RuleMeta {
    id: "P23",
    name: "MARKUP_RESIDUE",
    title: "記法の残骸",
    lane: Lane::Slop,
    status: RuleStatus::Experimental,
    default_severity: Severity::Warning,
    summary: "Markdown の強調の記号「**」が、太字にならずに記号のまま残っている箇所を指摘する (実験的)",
    explanation: r"### 何を見るか

Markdown の強調記法 `**…**` が、太字にならずに記号のまま残っている箇所を探します。同じ行の中で、ちょうど 2 個の `*` が対になり、そのあいだに日本語を含む 60 字以内の語句があって、語句の両端が空白でないものに限ります。

- Markdown の文書では、描画されずに残った記号だけが対象です。`**` の内側の端に括弧や句読点が来て、外側に文字が続くと、CommonMark では強調になりません (`予定は**「来週の水曜」**に決めた` の `**` は、そのまま表示されます)。重大度は警告です。
- テキスト、コードの普通のコメント、gws で書き込む値は Markdown として描画されないので、`**語句**` は記号のまま残ります。書き込み先や貼り付け先が Markdown を描画するかは分からないため、重大度は情報です。

見出し・箇条書き・表のセル・引用も、語句ルールのスコープの設定に関わらず見ます。3 個以上続く `*`、対にならない `**`、行をまたぐ対、`__…__`、エスケープした `\*\*`、コードと URL の中の記号は対象にしません。

### なぜ問題か

記号が残ると、読み手には意味のない `**` が並んで見えます。チャットの画面やエディタのプレビューは `**` を太字にして見せるので、書き手は記号が残ったことに気づきにくく、出力を確かめずに貼り付けた跡として目立ちます。Markdown でも、括弧に接した `**` は太字にならず、書き手が確かめた画面と読み手の画面とで見え方が変わることがあります。

### 直し方

Markdown では、`**` を括弧の内側へ移す (「**語句**」) か、太字をやめます。テキストや書き込む値では記号を外し、太字が要るなら出力先の書式 (ワープロやドキュメントの太字) で付けます。記法そのものを説明している箇所なら、コードとして囲むか、抑制コメントで理由を残してください。

### 例

- 直す前: `申請の締め切りは**「月末の3営業日前」**です。`
- 直した後: `申請の締め切りは「**月末の3営業日前**」です。`

### 根拠

実験的です。人の文書 171 本 (Markdown 159 本・テキスト 12 本) では指摘がなく、生成文書 390 本 (すべて Markdown) では 27 本 (6.9%) で 62 件を指摘しました。どれも括弧や句読点に接して描画されなかった記号です。対にならない `**` と行をまたぐ対は拾わないので、生成文書のうち記号がそのような形で残った 4 本は数に入っていません。テキストとして読む文書の型は、生成文書に該当するものがなく、測っていません。昇格の条件 (生成文書での検出率 20% 以上) は満たさないので、実験的に留めています。記号が残るのは出力の記法と表示先の食い違いで、書き手が AI かどうかの判定には使えません。",
};

/// 対の中身の字数の上限。対にならない記号どうしを遠くで結ばないための範囲の制限で、
/// 校正した値ではない。
const MAX_INNER_CHARS: usize = 60;

/// 項目 (metrics の `item`): Markdown の文書で、強調として描画されずに残った記号。
const UNRENDERED_ITEM: &str = "描画されない強調の記号";
/// 項目: Markdown として描画しない形式 (テキスト・コメント・書き込む値) に残った強調の記法。
const LITERAL_ITEM: &str = "文字のまま残った強調の記法";

pub(super) struct MarkupResidue;

impl MarkupResidue {
    /// 1 行のテキストから、ちょうど 2 個の `*` の対で挟まれた範囲 (記号を含む) を返す。
    ///
    /// `*` の連なりを左から見て、2 個の連なりと、その次の 2 個の連なりを対にする。次の連なりが
    /// 2 個でないか、中身が条件に合わなければ、その開きは捨てて次の連なりから続ける
    /// (`***` のような 3 個以上の連なりは開きにも閉じにもならない)。
    fn find_pairs(line: &str) -> Vec<Range<usize>> {
        let mut runs: Vec<Range<usize>> = Vec::new();
        for (i, c) in line.char_indices() {
            if c != '*' {
                continue;
            }
            match runs.last_mut() {
                Some(run) if run.end == i => run.end = i + 1,
                _ => runs.push(i..i + 1),
            }
        }
        let mut out = Vec::new();
        let mut i = 0;
        while i + 1 < runs.len() {
            let (open, close) = (&runs[i], &runs[i + 1]);
            if open.len() == 2 && close.len() == 2 && eligible(&line[open.end..close.start]) {
                out.push(open.start..close.end);
                i += 2;
            } else {
                i += 1;
            }
        }
        out
    }
}

/// 強調の記号に挟まれた中身として拾うか。空白で始まる・終わる中身 (有意水準の星印
/// `[-2.53]** [-2.38]**` など) と、日本語を含まない中身、コード・URL を除いた跡
/// ([`text::PLACEHOLDER`]) をまたぐ中身は拾わない。
fn eligible(inner: &str) -> bool {
    let (Some(first), Some(last)) = (inner.chars().next(), inner.chars().next_back()) else {
        return false;
    };
    !first.is_whitespace()
        && !last.is_whitespace()
        && inner.chars().count() <= MAX_INNER_CHARS
        && text::contains_japanese(inner)
        && !inner.contains(text::PLACEHOLDER)
}

impl Rule for MarkupResidue {
    fn unit(&self) -> RuleUnit {
        // 判定はブロックの中の 1 行で閉じ、ブロックをまたいで対を作らない。コメントや表計算のセルの
        // ような断片の集まりにも当てられるので、1 文ずつ判定するルールと同じ扱いにする
        // (P20 もブロックを単位に見て Sentence を返す)。
        RuleUnit::Sentence
    }

    fn meta(&self) -> &'static RuleMeta {
        &P23_META
    }

    fn check(&self, ctx: &RuleContext<'_>, out: &mut Vec<Diagnostic>) {
        let doc = ctx.doc;
        let markdown = doc.format == SourceFormat::Markdown;
        let (item, severity, hint) = if markdown {
            (
                UNRENDERED_ITEM,
                Severity::Warning,
                "`**` の内側の端に括弧や句読点が来ると、CommonMark では太字になりません。`**` を括弧の内側へ移す (「**語句**」) か、太字をやめてください",
            )
        } else {
            (
                LITERAL_ITEM,
                Severity::Info,
                "書き込み先や貼り付け先で太字として表示されないなら、記号を外してください。太字が要るなら、出力先の書式で付けてください",
            )
        };
        let mut seen = HashSet::new();
        // 記号の残骸は置き場所を問わず見えるので、語句ルールのスコープに従わず全ブロックを見る
        for (idx, block) in doc.blocks.iter().enumerate() {
            let mut line_start = 0;
            for line in block.text.split('\n') {
                for pair in Self::find_pairs(line) {
                    let range = (line_start + pair.start)..(line_start + pair.end);
                    let span = block.to_source(range.clone());
                    // 原文でも `**` で始まり `**` で終わる、1 行の中の範囲に限る
                    // (エスケープした `\*\*` や、解析用テキストでだけ隣り合った記号を外す)
                    let source = doc.slice(span);
                    if span.is_empty()
                        || !source.starts_with("**")
                        || !source.ends_with("**")
                        || doc.lines.line(span.start) != doc.lines.line(span.end - 1)
                        || !seen.insert((span.start, span.end))
                    {
                        continue;
                    }
                    let matched = &block.text[range.clone()];
                    let context = doc
                        .block_sentences(idx)
                        .iter()
                        .find(|s| s.range.start <= range.start && range.start < s.range.end)
                        .map_or(block.span, |s| s.span);
                    let message = if markdown {
                        format!(
                            "「{}」は強調として解釈されず、`**` が記号のまま表示されます",
                            quote(matched)
                        )
                    } else {
                        format!(
                            "「{}」は Markdown の強調記法 `**` が文字のまま残っています",
                            quote(matched)
                        )
                    };
                    out.push(
                        diagnostic(
                            &P23_META,
                            span,
                            context,
                            matched,
                            message,
                            hint,
                            severity,
                            P23_META.status,
                        )
                        .with_metric("item", item),
                    );
                }
                line_start += line.len() + 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code::CodeLanguage;
    use crate::diagnostic::Metric;
    use crate::document::{Document, ParseOptions};
    use crate::rules::testing::{Options, matched, run, run_doc};

    fn item(d: &Diagnostic) -> &str {
        match d.metrics.get("item") {
            Some(Metric::Text(s)) => s,
            other => panic!("item: {other:?}"),
        }
    }

    fn run_text(source: &str) -> Vec<Diagnostic> {
        run_doc(
            &MarkupResidue,
            &Document::plain_text(source),
            Options::default(),
        )
    }

    fn run_rust(source: &str) -> Vec<Diagnostic> {
        let doc = Document::parse(
            "<input>.rs",
            source,
            SourceFormat::Code(CodeLanguage::Rust),
            &ParseOptions::default(),
        );
        run_doc(&MarkupResidue, &doc, Options::default())
    }

    #[test]
    fn flags_unrendered_bold_in_markdown_with_the_original_span() {
        let md = "予定は**「来週の水曜」**に決めた。\n";
        let d = run(&MarkupResidue, md);
        assert_eq!(matched(md, &d), ["**「来週の水曜」**"]);
        assert_eq!(d[0].rule_id, "P23");
        assert_eq!(d[0].severity, Severity::Warning);
        assert_eq!(d[0].status, RuleStatus::Experimental);
        assert_eq!(item(&d[0]), UNRENDERED_ITEM);
        assert!(
            d[0].message.contains("記号のまま表示されます"),
            "{}",
            d[0].message
        );
        assert!(d[0].hint.is_some());
        let context = d[0].context.expect("context");
        assert_eq!(&md[context.range()], "予定は**「来週の水曜」**に決めた。");
    }

    #[test]
    fn a_partly_rendered_line_is_reported_once_from_the_first_to_the_last_marker() {
        // 内側の `**` は「と」を太字にし、外側の 2 つが記号のまま残る
        let md = "よく使うのは**Rate Limit（流量の制限）**と**Circuit Breaker（遮断）**です。\n";
        let d = run(&MarkupResidue, md);
        assert_eq!(
            matched(md, &d),
            ["**Rate Limit（流量の制限）**と**Circuit Breaker（遮断）**"]
        );
    }

    #[test]
    fn rendered_bold_is_not_a_residue() {
        for md in [
            "今月は**売上の回復**を優先する。\n",
            "予定は「**来週の水曜**」に決めた。\n",
            "- **「目的」**: 説明する\n",
            "- **目的**: 説明する\n",
        ] {
            assert!(run(&MarkupResidue, md).is_empty(), "{md}");
        }
    }

    #[test]
    fn ignores_escapes_stars_globs_triple_stars_and_code() {
        for md in [
            "エスケープした \\*\\*強調\\*\\* の記法を示す。\n",
            "係数は [-1.20]** と [-0.85]** だった。\n",
            "除外は **/*.md と書く。\n",
            "***「三つの星」***です。\n",
            "記法は `**「強調」**` と書く。\n",
            "```text\n**「強調」**です。\n```\n",
        ] {
            assert!(run(&MarkupResidue, md).is_empty(), "{md}");
        }
    }

    #[test]
    fn looks_at_headings_tables_and_quotes_regardless_of_the_phrase_scope() {
        let md = "## **「見出し」**の比較\n\n| 列 |\n|---|\n| 値は**「重要」**です |\n\n> 引用の**「強調」**です。\n";
        let d = run(&MarkupResidue, md);
        assert_eq!(
            matched(md, &d),
            ["**「見出し」**", "**「重要」**", "**「強調」**"]
        );
    }

    #[test]
    fn flags_literal_bold_in_plain_text_paragraphs_and_list_items() {
        let text = "今月は**売上の回復**を優先する。\n\n- **作業の分担**: 担当を決める\n";
        let d = run_text(text);
        assert_eq!(matched(text, &d), ["**売上の回復**", "**作業の分担**"]);
        for d in &d {
            assert_eq!(d.severity, Severity::Info);
            assert_eq!(d.status, RuleStatus::Experimental);
            assert_eq!(item(d), LITERAL_ITEM);
            assert!(
                d.message.contains("文字のまま残っています"),
                "{}",
                d.message
            );
        }
    }

    #[test]
    fn pairs_stay_within_one_line_but_may_span_sentences() {
        assert!(run_text("**太字の始まり\n続き**です。\n").is_empty());
        let text = "**まず確認します。次に実行します。**\n";
        assert_eq!(
            matched(text, &run_text(text)),
            ["**まず確認します。次に実行します。**"]
        );
    }

    #[test]
    fn requires_japanese_inside_and_a_bounded_length() {
        assert!(run_text("**Note** を読む。\n").is_empty());
        let long = "あ".repeat(MAX_INNER_CHARS + 1);
        assert!(run_text(&format!("これは**{long}**です。\n")).is_empty());
        let limit = "あ".repeat(MAX_INNER_CHARS);
        assert_eq!(run_text(&format!("これは**{limit}**です。\n")).len(), 1);
        // 中身の両端が空白なら拾わない
        assert!(run_text("これは** 強調 **です。\n").is_empty());
    }

    #[test]
    fn plain_comments_keep_the_markers_but_doc_comments_render_them() {
        let d = run_rust("// **注意**: 設定を先に読む\nfn main() {}\n");
        assert_eq!(d.len(), 1);
        assert_eq!(item(&d[0]), LITERAL_ITEM);
        assert_eq!(d[0].severity, Severity::Info);
        assert!(run_rust("/// **注意**: 設定を先に読む\nfn main() {}\n").is_empty());
    }

    #[test]
    fn judges_locally_so_fragments_can_use_it() {
        assert_eq!(MarkupResidue.unit(), RuleUnit::Sentence);
    }
}
