//! 密度ではなく、明示された予告と実体、見出しと節の境界を照合する。

use regex::Regex;
use std::sync::LazyLock;

use crate::diagnostic::{Diagnostic, Lane, RuleStatus, Severity};
use crate::document::BlockKind;
use crate::rules::{Rule, RuleContext, RuleMeta};
use crate::structure::{NodeKind, Structure};

static COUNT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:次|以下)の(?P<n>[0-9０-９]+|[一二三四五六七八九十]+)(?:つ|点|項目|条件|手順)")
        .unwrap()
});

fn number(text: &str) -> Option<usize> {
    let ascii: String = text
        .chars()
        .map(|c| {
            if ('０'..='９').contains(&c) {
                char::from_u32(c as u32 - '０' as u32 + '0' as u32).unwrap()
            } else {
                c
            }
        })
        .collect();
    if let Ok(n) = ascii.parse() {
        return Some(n);
    }
    let digit = |c| {
        "一二三四五六七八九"
            .chars()
            .position(|n| n == c)
            .map(|n| n + 1)
    };
    if let Some((a, b)) = text.split_once('十') {
        let tens = if a.is_empty() {
            1
        } else {
            digit(a.chars().next()?)?
        };
        let units = if b.is_empty() {
            0
        } else {
            digit(b.chars().next()?)?
        };
        if a.chars().count() > 1 || b.chars().count() > 1 {
            return None;
        }
        Some(tens * 10 + units)
    } else if text.chars().count() == 1 {
        digit(text.chars().next()?)
    } else {
        None
    }
}

static COUNT_META: RuleMeta = RuleMeta {
    id: "S11", name: "ENUMERATION_COUNT_MISMATCH", title: "列挙の予告と項目数の不一致",
    lane: Lane::Readability, status: RuleStatus::Stable, default_severity: Severity::Info,
    summary: "直後のリストに対応する明示的な予告と、直接の項目数を照合する",
    explanation: "\
### 何を見るか

段落末尾の文が「次の3項目」などと予告し、同じ親の直後にリストがある場合に、予告した数と直接の項目数を照合します。子リストや項目内の複数段落は足しません。引用・脚注・コードは除きます。

### なぜ問題か

予告と実体が違うと、読み手は項目が欠けたのか、予告した数が誤っているのかを判断できません。

### 直し方

正しい数と必要な項目を確認してください。不足する内容を推測で補わないでください。外部の一覧や、後から説明する内容の予告は対象外です。

### 例

- 直す前: 次の3項目を確認してください。その直後に名前と住所の2項目を並べる。
- 直した後: 必要なのが名前と住所なら、予告を「次の2項目」に直す。

### 根拠

構文木の親子関係と明示した数を照合します。人164本・生成811本・その他278本のMarkdownで指摘は0件でした。自作の対照40文書では3つの不一致を検出し、修正後と正常な構造では指摘しないことを確認しました。実文書での検出精度を推定した数値ではありません。",
};

pub struct EnumerationCount;
impl Rule for EnumerationCount {
    fn meta(&self) -> &'static RuleMeta {
        &COUNT_META
    }
    fn check(&self, ctx: &RuleContext<'_>, out: &mut Vec<Diagnostic>) {
        let structure = Structure::parse(ctx.doc);
        for (id, node) in structure.nodes.iter().enumerate() {
            if node.kind != NodeKind::Paragraph || !structure.in_body(id) {
                continue;
            }
            let Some(next) = structure.next_sibling(id) else {
                continue;
            };
            let list = &structure.nodes[next];
            if list.kind != NodeKind::List {
                continue;
            }
            let Some((block_id, block)) = node.block_ids.iter().rev().find_map(|&i| {
                let block = &ctx.doc.blocks[i];
                (block.kind == BlockKind::Paragraph).then_some((i, block))
            }) else {
                continue;
            };
            let Some(sentence) = ctx.doc.block_sentences(block_id).last() else {
                continue;
            };
            let text = &block.text[sentence.range.clone()];
            let matches: Vec<_> = COUNT.captures_iter(text).collect();
            if matches.len() != 1 {
                continue;
            }
            // 否定・未来の予告・一部だけの提示は、対応が確定しない。
            if [
                "ない",
                "ません",
                "後ほど",
                "後で",
                "一部",
                "例えば",
                "たとえば",
                "うち",
            ]
            .iter()
            .any(|word| text.contains(word))
            {
                continue;
            }
            let n = matches[0].name("n").unwrap();
            if text[matches[0].get(0).unwrap().end()..].starts_with('目') {
                continue;
            }
            let Some(declared) = number(n.as_str()) else {
                continue;
            };
            let actual = list
                .children
                .iter()
                .filter(|&&item| structure.nodes[item].kind == NodeKind::Item)
                .count();
            if declared == actual {
                continue;
            }
            // コードやリスト記号の変更で分断した一覧は、対応を断定しない。
            let mut following = structure.next_sibling(next);
            while let Some(id) = following {
                if structure.nodes[id].kind != NodeKind::Code {
                    break;
                }
                following = structure.next_sibling(id);
            }
            if following.is_some_and(|id| structure.nodes[id].kind == NodeKind::List) {
                continue;
            }
            let start = sentence.range.start + n.start();
            let mut d = COUNT_META.diagnostic(
                block.to_source(start..start + n.len()),
                format!("{declared}項目と予告していますが、直後のリストは{actual}項目です"),
            );
            d.context = Some(crate::diagnostic::Span::new(
                sentence.span.start,
                list.span.end,
            ));
            d.related.push(list.span);
            d.metrics.insert("declared".into(), declared.into());
            d.metrics.insert("actual".into(), actual.into());
            d.hint = Some(
                "予告した数と必要な項目を確認してください。内容は推測で補わないでください".into(),
            );
            out.push(d);
        }
    }
}

static EMPTY_META: RuleMeta = RuleMeta {
    id: "S12", name: "EMPTY_SECTION", title: "本文のない節",
    lane: Lane::Readability, status: RuleStatus::Stable, default_severity: Severity::Info,
    summary: "見出しの後に本文も子の節もない箇所を示す",
    explanation: "\
### 何を見るか

Markdownの見出しから次の同じ階層以上の見出しまでが空白とコメントだけの場合を示します。子見出しやリスト・表・コードなどがある節は空とはみなしません。引用と脚注の見出しは除きます。

### なぜ問題か

見出しを頼りに探した読み手が、その内容を確認できません。

### 直し方

必要な説明を書くか、使わない見出しを削ってください。意図した空欄や書きかけなら残せます。変換した文書では、表示上の本文や見出し階層も確認してください。

### 例

- 直す前: 「申込み方法」の見出しの直後に、同じ階層の「問い合わせ先」がある。
- 直した後: 「申込み方法」の節に、受付フォームへ提出する手順を書く。

### 根拠

明示的な節の範囲を確認します。人164本・生成811本・その他278本のMarkdownで13文書39箇所に一致し、該当範囲を確認しました。保存時に見出し階層・コード柵・画像などを失った文書も含むため、原文の説明の欠落や書き手の誤りとは断定しません。構文上の空欄を確認する情報として既定で有効にします。",
};

pub struct EmptySection;
impl Rule for EmptySection {
    fn meta(&self) -> &'static RuleMeta {
        &EMPTY_META
    }
    fn requires_full_document(&self) -> bool {
        true
    }
    fn check(&self, ctx: &RuleContext<'_>, out: &mut Vec<Diagnostic>) {
        let structure = Structure::parse(ctx.doc);
        for (id, node) in structure.nodes.iter().enumerate() {
            if !matches!(node.kind, NodeKind::Heading(_)) || !structure.in_body(id) {
                continue;
            }
            let section = structure.section_span(id).unwrap();
            let body = &ctx.doc.source[node.span.end..section.end];
            let without_comments = remove_comments(body);
            if !without_comments.trim().is_empty() {
                continue;
            }
            let mut d = EMPTY_META.diagnostic(
                node.span,
                "この見出しの節には、Markdown上の本文がありません",
            );
            d.context = Some(section);
            d.hint = Some(
                "意図した空欄や書きかけなら残せます。変換した文書では、表示上の本文や見出し階層も確認してください".into(),
            );
            out.push(d);
        }
    }
}

fn remove_comments(text: &str) -> String {
    let mut remaining = text;
    let mut out = String::new();
    while let Some(start) = remaining.find("<!--") {
        out.push_str(&remaining[..start]);
        let Some(end) = remaining[start + 4..].find("-->") else {
            out.push_str(&remaining[start..]);
            return out;
        };
        remaining = &remaining[start + 4 + end + 3..];
    }
    out.push_str(remaining);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::rules::testing::run;

    #[test]
    fn counts_parent_items_and_points_to_original_digit() {
        let source = "次の三項目を確認してください。\n\n- 氏名\n\n  補足です。\n\n  - 子\n- 住所\n";
        let d = run(&EnumerationCount, source);
        assert_eq!(d.len(), 1);
        assert_eq!(&source[d[0].span.range()], "三");
        assert_eq!(d[0].metrics["actual"], 2usize.into());
        assert_eq!(run(&EnumerationCount, &source.replace('三', "２")).len(), 0);
    }

    #[test]
    fn excludes_other_sections_quotes_code_and_nonfinal_promises() {
        for source in [
            "次の3項目です。\n\n## 別の節\n- 氏名\n- 住所\n",
            "> 次の3項目です。\n>\n> - 氏名\n> - 住所\n",
            "```text\n次の3項目です。\n\n- 氏名\n- 住所\n```\n",
            "次の3項目です。今日は晴れです。\n\n- 氏名\n- 住所\n",
            "次の3項目のうち一部を示します。\n\n- 氏名\n- 住所\n",
            "以下の3つ目の例です。\n\n- 氏名\n- 住所\n",
            "次の3項目です。\n\n---\n\n- 氏名\n- 住所\n",
            "次の3項目です。\n\n<div>別の案内</div>\n\n- 氏名\n- 住所\n",
            "次の3手順で行います。\n\n1. 起動\n\n```sh\ncommand\n```\n\n2. 登録\n3. 確認\n",
            "次の3項目です。\n\n- 起動\n* 登録\n* 確認\n",
        ] {
            assert!(run(&EnumerationCount, source).is_empty(), "{source}");
        }
    }

    #[test]
    fn empty_sections_keep_parent_sections_code_and_comments() {
        let source = "# 案内\n\n## 申込み\n<!-- メモ -->\n\n## 手順\n```text\n操作\n```\n";
        let d = run(&EmptySection, source);
        assert_eq!(d.len(), 1);
        assert!(source[d[0].span.range()].contains("申込み"));
    }

    #[test]
    #[cfg(feature = "cli")]
    fn relation_is_returned_when_only_the_list_changed() {
        let source = "次の3項目です。\n\n- 名前\n- 住所\n\n別の段落です。\n";
        let d = run(&EnumerationCount, source).remove(0);
        let start = source.find("住所").unwrap();
        assert!(crate::changed::touches(
            &d,
            &[crate::diagnostic::Span::new(start, start + 6)]
        ));
        let unrelated = source.find("別の段落").unwrap();
        assert!(!crate::changed::touches(
            &d,
            &[crate::diagnostic::Span::new(unrelated, source.len())]
        ));
    }

    #[test]
    fn structure_rules_are_stable_and_require_the_right_coverage() {
        use crate::engine::{Coverage, Engine, EngineOptions, Selection};
        let doc = Document::markdown("次の3項目です。\n\n- 名前\n- 住所\n\n# 空の節\n");
        let default = Engine::new(EngineOptions::default())
            .unwrap()
            .lint(doc.clone());
        let structure: Vec<_> = default
            .diagnostics
            .iter()
            .filter(|d| matches!(d.rule_id.as_str(), "S11" | "S12"))
            .collect();
        assert_eq!(structure.len(), 2);
        assert!(
            structure
                .iter()
                .all(|d| { d.status == RuleStatus::Stable && d.severity == Severity::Info })
        );
        let engine = Engine::new(EngineOptions {
            selection: Selection {
                only: Some(vec!["S11".into(), "S12".into()]),
                ..Selection::default()
            },
            ..EngineOptions::default()
        })
        .unwrap();
        assert_eq!(engine.lint(doc.clone()).diagnostics.len(), 2);
        let partial = engine.lint_with(doc, Coverage::Partial);
        assert_eq!(partial.diagnostics.len(), 1);
        assert_eq!(partial.diagnostics[0].rule_id, "S11");
        let mut fragments = Document::markdown("# 空の節\n");
        fragments.kind = crate::document::DocumentKind::Fragments;
        assert!(engine.lint(fragments).diagnostics.is_empty());
    }

    #[test]
    fn default_structure_findings_can_be_disabled_and_suppressed() {
        use crate::engine::{Engine, EngineOptions, Selection};
        let source = "次の3項目です。\n\n- 名前\n- 住所\n\n# 空の節\n";
        for selection in [
            Selection {
                no_readability: true,
                ..Selection::default()
            },
            Selection {
                cli_disable: vec!["S11".into(), "S12".into()],
                ..Selection::default()
            },
        ] {
            let report = Engine::new(EngineOptions {
                selection,
                ..EngineOptions::default()
            })
            .unwrap()
            .lint(Document::markdown(source));
            assert!(
                !report
                    .diagnostics
                    .iter()
                    .any(|d| matches!(d.rule_id.as_str(), "S11" | "S12"))
            );
        }
        let report = Engine::new(EngineOptions::default())
            .unwrap()
            .lint(Document::markdown(format!(
                "<!-- noslop-disable-file S11,S12 -- 下書き -->\n\n{source}"
            )));
        let structure: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| matches!(d.rule_id.as_str(), "S11" | "S12"))
            .collect();
        assert_eq!(structure.len(), 2);
        assert!(structure.iter().all(|d| d.suppressed.is_some()));
    }

    #[test]
    fn positions_survive_front_matter_entities_crlf_and_ordered_lists() {
        let source =
            "---\r\ntitle: 案内\r\n---\r\n\r\n次の&#51;項目です。\r\n\r\n1. 名前\r\n2. 住所\r\n";
        let d = run(&EnumerationCount, source);
        assert_eq!(d.len(), 1);
        assert_eq!(&source[d[0].span.range()], "&#51;");
        assert!(source[d[0].related[0].range()].starts_with("1. "));
        assert!(run(&EnumerationCount, "次の2項目です。\n\n- 名前\n- 住所\n").is_empty());
    }
}
