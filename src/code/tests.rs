//! コードのコメントを読み取る処理のテスト (言語ごとの取り出し、まとめ方、記号の外し方、原文上の位置)。

use super::*;
use crate::document::{BlockKind, DirectiveKind, Document, ParseOptions, SourceFormat};
use crate::text;

/// コードとして読んだ文書 (解析用テキストの日本語の字が原文の同じ字に戻ることも確かめる)。
fn doc(src: &str, lang: CodeLanguage) -> Document {
    let doc = Document::parse(
        format!("<input>.{}", lang.extensions()[0]),
        src,
        SourceFormat::Code(lang),
        &ParseOptions::default(),
    );
    assert_exact(&doc);
    doc
}

/// 解析用テキストの日本語の字が、1 字ずつ原文の同じ字に戻るか。
fn assert_exact(doc: &Document) {
    for block in &doc.blocks {
        for (i, c) in block.text.char_indices() {
            if text::is_japanese(c) {
                let span = block.to_source(i..i + c.len_utf8());
                assert_eq!(
                    doc.slice(span),
                    c.to_string(),
                    "{:?} の {i} バイト目",
                    block.text
                );
            }
        }
    }
}

/// ブロックを「種類:解析用テキスト」で並べる (P は段落、L は箇条書きの項目、H1 は見出し 1)。
fn outline(src: &str, lang: CodeLanguage) -> Vec<String> {
    doc(src, lang)
        .blocks
        .iter()
        .map(|b| {
            let kind = match b.kind {
                BlockKind::Paragraph => "P".to_string(),
                BlockKind::ListItem => "L".to_string(),
                BlockKind::TableCell => "T".to_string(),
                BlockKind::Heading(level) => format!("H{level}"),
            };
            format!("{kind}:{}", b.text)
        })
        .collect()
}

/// 文の解析用テキストと、その原文上の範囲の文字列。
fn sentences(src: &str, lang: CodeLanguage) -> Vec<(String, String)> {
    let d = doc(src, lang);
    d.sentences
        .iter()
        .map(|s| (d.sentence_text(s).to_string(), d.slice(s.span).to_string()))
        .collect()
}

#[test]
fn extensions_map_to_one_language() {
    let mut seen = std::collections::BTreeMap::new();
    for lang in CodeLanguage::ALL {
        for ext in lang.extensions() {
            assert_eq!(ext.to_ascii_lowercase(), *ext, "{ext} は小文字で書く");
            if let Some(other) = seen.insert(*ext, lang) {
                panic!("{ext} が {other:?} と {lang:?} の両方にある");
            }
            assert_eq!(CodeLanguage::from_extension(ext), Some(lang));
        }
    }
    assert_eq!(CodeLanguage::from_extension("RS"), Some(CodeLanguage::Rust));
    // 文書の拡張子はコードにしない
    for ext in ["md", "markdown", "txt", "mdx"] {
        assert_eq!(CodeLanguage::from_extension(ext), None);
    }
}

#[test]
fn every_language_is_readable() {
    let known: Vec<&str> = CodeLanguage::known_extensions().collect();
    for lang in CodeLanguage::ALL {
        for ext in lang.extensions() {
            assert!(known.contains(ext), "{ext}");
        }
    }
    assert!(known.contains(&"sh") && known.contains(&"swift"));
}

#[test]
fn every_grammar_loads_and_reads_a_comment() {
    // 文法の ABI が tree-sitter と合わないと、読み込みで止まる
    let samples = [
        (CodeLanguage::Rust, "// 説明です。\n"),
        (CodeLanguage::JavaScript, "// 説明です。\n"),
        (CodeLanguage::TypeScript, "// 説明です。\n"),
        (CodeLanguage::Tsx, "// 説明です。\n"),
        (CodeLanguage::Python, "# 説明です。\n"),
        (CodeLanguage::Go, "// 説明です。\n"),
        (CodeLanguage::Java, "// 説明です。\n"),
        (CodeLanguage::C, "// 説明です。\n"),
        (CodeLanguage::Cpp, "// 説明です。\n"),
        (CodeLanguage::CSharp, "// 説明です。\n"),
        (CodeLanguage::Ruby, "# 説明です。\n"),
        (CodeLanguage::Php, "<?php\n// 説明です。\n"),
        (CodeLanguage::Swift, "// 説明です。\n"),
        (CodeLanguage::Kotlin, "// 説明です。\n"),
        (CodeLanguage::Bash, "# 説明です。\n"),
        (CodeLanguage::Yaml, "# 説明です。\n"),
        (CodeLanguage::Toml, "# 説明です。\n"),
        (CodeLanguage::Html, "<!-- 説明です。 -->\n"),
        (CodeLanguage::Css, "/* 説明です。 */\n"),
        (CodeLanguage::Lua, "-- 説明です。\n"),
    ];
    assert_eq!(samples.len(), CodeLanguage::ALL.len());
    for (lang, src) in samples {
        assert_eq!(outline(src, lang), ["P:説明です。"], "{lang:?}");
    }
}

#[test]
fn rust_comments_doc_comments_and_strings() {
    let src = r##"//! クレートの説明です。
//! 二行目も同じまとまりです。

/// 設定を読み込む。
///
/// `Config::load` を呼び、失敗したら既定値を使う。
/// ```
/// let x = "// 文字列の中";
/// ```
fn load() {
    let s = "// 文字列はコメントではない";
    let r = r#"/* 生の文字列 */"#;
    let c = '"';
    // 普通のコメント。
    // 続きの行。
    let x = 1; // 行末のコメント。
    /* ブロック /* 入れ子 */ の続き。 */
    //// 四本の斜線は普通のコメント。
}

/**
 * 星の飾りのある文書。
 */
struct S;
"##;
    assert_eq!(
        outline(src, CodeLanguage::Rust),
        [
            "P:クレートの説明です。二行目も同じまとまりです。",
            "P:設定を読み込む。",
            "P:\u{FFFC} を呼び、失敗したら既定値を使う。",
            "P:普通のコメント。続きの行。",
            "P:行末のコメント。",
            "P:ブロック /* 入れ子 */ の続き。",
            "P:四本の斜線は普通のコメント。",
            "P:星の飾りのある文書。",
        ]
    );
    // インラインコードは原文のバッククォートを含む範囲に戻る
    let d = doc(src, CodeLanguage::Rust);
    let code = &d.blocks[2];
    assert_eq!(d.slice(code.to_source(0..3)), "`Config::load`");
    // 2 行にまたがる段落は、行の境目 (改行と記号) を含む原文の範囲に戻る
    let para = &d.blocks[3];
    assert_eq!(
        d.slice(para.to_source(0..para.text.len())),
        "普通のコメント。\n    // 続きの行。"
    );
}

#[test]
fn javascript_jsdoc_regex_and_template_literals() {
    let src = "#!/usr/bin/env node
/**
 * 値を二倍にする。
 * @param {number} a 元の値
 * @returns {number} 二倍の値
 */
function double(a) {
  const re = /\\/\\/ 正規表現の中/g; // 正規表現の後ろのコメント。
  const t = `テンプレート // でない ${a /* 式の中のコメント。 */}`;
  return a * 2;
}
";
    assert_eq!(
        outline(src, CodeLanguage::JavaScript),
        [
            "P:値を二倍にする。 @param {number} a 元の値 @returns {number} 二倍の値",
            "P:正規表現の後ろのコメント。",
            "P:式の中のコメント。",
        ]
    );
    // タグで始まる行の前で文を切る
    let texts: Vec<String> = sentences(src, CodeLanguage::JavaScript)
        .into_iter()
        .map(|(t, _)| t)
        .collect();
    assert_eq!(
        &texts[..3],
        [
            "値を二倍にする。",
            "@param {number} a 元の値",
            "@returns {number} 二倍の値"
        ]
    );
}

#[test]
fn typescript_and_tsx_comments() {
    // 参照の指示は外し、`/*!` の `!` は記号として外す
    let src = "/// <reference types=\"node\" />\n/*! 圧縮しても残すコメント。 */\n// 型を定義する。\ntype A = string; // 行末のコメント。\n";
    assert_eq!(
        outline(src, CodeLanguage::TypeScript),
        [
            "P:圧縮しても残すコメント。",
            "P:型を定義する。",
            "P:行末のコメント。"
        ]
    );
    let src = "// 行のコメント。\nconst x = <div>{/* JSX の中のコメント。 */}</div>;\n";
    assert_eq!(
        outline(src, CodeLanguage::Tsx),
        ["P:行のコメント。", "P:JSX の中のコメント。"]
    );
}

#[test]
fn python_comments_and_docstrings() {
    let src = r##"#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""モジュールの説明です。"""

# 普通のコメント。
def f():
    """関数の説明です。

    二段落目です。
    """
    s = "# 文字列の中"
    x = """ただの三重引用符の文字列です。"""
    return 1  # 行末のコメント。


class C:
    '''クラスの説明です。'''

    def g(self):
        # 本体の前のコメント。
        r"""生の文字列の説明です。"""

    def h(self):
        f"""f 文字列は docstring ではありません。"""
"##;
    assert_eq!(
        outline(src, CodeLanguage::Python),
        [
            "P:モジュールの説明です。",
            "P:普通のコメント。",
            "P:関数の説明です。",
            "P:二段落目です。",
            "P:行末のコメント。",
            "P:クラスの説明です。",
            "P:本体の前のコメント。",
            "P:生の文字列の説明です。",
        ]
    );
}

#[test]
fn go_comments_and_build_directives() {
    let src = "//go:build linux

// Package sample は例のパッケージです。
package sample

/*
複数行のブロックの
コメントです。
*/
func f() string { return \"// 文字列の中\" } // 行末のコメント。
";
    assert_eq!(
        outline(src, CodeLanguage::Go),
        [
            "P:Package sample は例のパッケージです。",
            "P:複数行のブロックのコメントです。",
            "P:行末のコメント。",
        ]
    );
}

#[test]
fn javadoc_is_read_without_html_tags() {
    let src = "/**
 * 設定を読み込みます。
 * <p>
 * 失敗したら {@code null} を返します。
 * @param path 設定のパス
 */
class A {
    // 行のコメント。
    String s = \"/* 文字列の中 */\";
}
";
    assert_eq!(
        outline(src, CodeLanguage::Java),
        [
            "P:設定を読み込みます。",
            "P:失敗したら \u{FFFC} を返します。 @param path 設定のパス",
            "P:行のコメント。",
        ]
    );
    let d = doc(src, CodeLanguage::Java);
    let pos = d.blocks[1].text.find('\u{FFFC}').unwrap();
    assert_eq!(d.slice(d.blocks[1].to_source(pos..pos + 3)), "{@code null}");
}

#[test]
fn c_doxygen_and_ordinary_comments_are_separate() {
    let src = "/** 文書のコメントです。 */\n/// 三本の斜線の文書です。\n// 普通のコメントです。\nint main(void) { char *s = \"// 文字列\"; return 0; /* 末尾のコメント。 */ }\n";
    assert_eq!(
        outline(src, CodeLanguage::C),
        [
            "P:文書のコメントです。",
            "P:三本の斜線の文書です。",
            "P:普通のコメントです。",
            "P:末尾のコメント。",
        ]
    );
}

#[test]
fn bash_comments_skip_heredocs_and_expansions() {
    let src = "#!/bin/bash
# 普通のコメントです。
# 続きの行です。
echo \"# 文字列の中\" # 行末のコメントです。
cat <<EOS
# ヒアドキュメントの中
EOS
x=${y#pre} # 展開の後ろのコメントです。
";
    assert_eq!(
        outline(src, CodeLanguage::Bash),
        [
            "P:普通のコメントです。続きの行です。",
            "P:行末のコメントです。",
            "P:展開の後ろのコメントです。",
        ]
    );
}

#[test]
fn yaml_and_toml_comments_skip_strings() {
    let src = "# 先頭のコメントです。\nkey: \"# 文字列の中\" # 行末のコメントです。\ntext: |\n  # ブロックの文字列の中\n";
    assert_eq!(
        outline(src, CodeLanguage::Yaml),
        ["P:先頭のコメントです。", "P:行末のコメントです。"]
    );
    let src = "# 先頭のコメントです。\nkey = \"# 文字列の中\" # 行末のコメントです。\n";
    assert_eq!(
        outline(src, CodeLanguage::Toml),
        ["P:先頭のコメントです。", "P:行末のコメントです。"]
    );
}

#[test]
fn html_css_and_lua_comments() {
    let src = "<!-- HTML のコメントです。 -->\n<p>本文は読みません。</p>\n";
    assert_eq!(
        outline(src, CodeLanguage::Html),
        ["P:HTML のコメントです。"]
    );
    let src = "/* CSS のコメントです。 */\na { content: \"/* 文字列の中 */\"; }\n";
    assert_eq!(outline(src, CodeLanguage::Css), ["P:CSS のコメントです。"]);
    let src = "-- 行のコメントです。\n--[[ ブロックの\nコメントです。 ]]\n--[==[ 等号付きのブロックです。 ]==]\nlocal s = \"-- 文字列の中\"\n";
    assert_eq!(
        outline(src, CodeLanguage::Lua),
        [
            "P:行のコメントです。",
            "P:ブロックのコメントです。",
            "P:等号付きのブロックです。",
        ]
    );
}

#[test]
fn cpp_comments() {
    let src = "/// 三本の斜線の文書です。\n// 普通のコメントです。\nauto s = \"// 文字列\"; // 行末のコメントです。\n";
    assert_eq!(
        outline(src, CodeLanguage::Cpp),
        [
            "P:三本の斜線の文書です。",
            "P:普通のコメントです。",
            "P:行末のコメントです。",
        ]
    );
}

#[test]
fn csharp_xml_documentation_comments() {
    let src = "/// <summary>
/// 設定を読み込みます。
/// </summary>
/// <param name=\"path\">設定のパス。<see cref=\"Path\"/> を渡します。</param>
// 普通のコメントです。
class A { string s = \"// 文字列の中\"; }
";
    assert_eq!(
        outline(src, CodeLanguage::CSharp),
        [
            "P:設定を読み込みます。",
            "P:設定のパス。\u{FFFC} を渡します。",
            "P:普通のコメントです。",
        ]
    );
}

#[test]
fn ruby_comments_and_embedded_documents() {
    let src = "#!/usr/bin/env ruby
# frozen_string_literal: true
# 普通のコメントです。
=begin
複数行の
コメントです。
=end
s = \"# 文字列の中\"
x = <<~EOS
  # ヒアドキュメントの中
EOS
y = 1 # 行末のコメントです。
";
    assert_eq!(
        outline(src, CodeLanguage::Ruby),
        [
            "P:普通のコメントです。",
            "P:複数行のコメントです。",
            "P:行末のコメントです。",
        ]
    );
}

#[test]
fn php_comments() {
    let src = "<?php\n/** 文書のコメントです。 */\n// 行のコメントです。\n# シャープのコメントです。\n$s = \"// 文字列の中\";\n";
    assert_eq!(
        outline(src, CodeLanguage::Php),
        [
            "P:文書のコメントです。",
            "P:行のコメントです。",
            "P:シャープのコメントです。",
        ]
    );
}

#[test]
fn swift_and_kotlin_comments() {
    let src = "/// 文書のコメントです。\n// 普通のコメントです。\n/* ブロック /* 入れ子 */ の続きです。 */\nlet s = \"// 文字列の中\"\n";
    assert_eq!(
        outline(src, CodeLanguage::Swift),
        [
            "P:文書のコメントです。",
            "P:普通のコメントです。",
            "P:ブロック /* 入れ子 */ の続きです。",
        ]
    );
    let src = "/** 文書のコメントです。 */\n// 普通のコメントです。\nval s = \"// 文字列の中\"\n";
    assert_eq!(
        outline(src, CodeLanguage::Kotlin),
        ["P:文書のコメントです。", "P:普通のコメントです。"]
    );
}

#[test]
fn line_comments_group_only_when_adjacent_aligned_and_alike() {
    let src = "# 一つ目のまとまりです。
# 同じまとまりです。

# 空いた行の後は別のまとまりです。
echo 1
# コードの後も別です。
  # 字下げが違えば別です。
echo 2 # 行末のコメントは単独です。
# 行末のコメントの次の行も別です。
";
    assert_eq!(
        outline(src, CodeLanguage::Bash),
        [
            "P:一つ目のまとまりです。同じまとまりです。",
            "P:空いた行の後は別のまとまりです。",
            "P:コードの後も別です。",
            "P:字下げが違えば別です。",
            "P:行末のコメントは単独です。",
            "P:行末のコメントの次の行も別です。",
        ]
    );
}

#[test]
fn comments_without_japanese_license_headers_and_tool_directives_are_skipped() {
    let src = "# Copyright (c) 2026 著作権者の名前
# 利用条件は LICENSE を見てください。

# SPDX-License-Identifier: MIT
# 説明の文です。

# English only comment.
# shellcheck disable=SC2034
# 残す説明です。
";
    assert_eq!(outline(src, CodeLanguage::Bash), ["P:残す説明です。"]);
}

#[test]
fn directives_in_comments_suppress_by_line() {
    let src = "# noslop-disable-next-line P01 -- 引用なので残す
# これは結論と言えるでしょう。
: # <!-- noslop-disable-line R03 -->
# 抑制は noslop-disable-next-line のように書く。
";
    let d = doc(src, CodeLanguage::Bash);
    let kinds: Vec<_> = d.directives.iter().map(|d| d.kind).collect();
    assert_eq!(
        kinds,
        [DirectiveKind::DisableNextLine, DirectiveKind::DisableLine]
    );
    assert_eq!(d.directives[0].rules, ["P01"]);
    assert_eq!(d.directives[0].reason.as_deref(), Some("引用なので残す"));
    assert_eq!(
        d.slice(d.directives[0].span),
        "# noslop-disable-next-line P01 -- 引用なので残す"
    );
    assert_eq!(d.lines.line(d.directives[1].span.start), 3);
    // 抑制コメントは本文にならず、行のまとまりも切る。文の途中の記法は本文のまま
    assert_eq!(
        outline(src, CodeLanguage::Bash),
        [
            "P:これは結論と言えるでしょう。",
            "P:抑制は noslop-disable-next-line のように書く。",
        ]
    );
}

#[test]
fn directives_in_doc_comments_use_both_forms() {
    let src = "/// noslop-disable-file R03
/// 説明の文と言えるでしょう。<!-- noslop-disable-line P01 -->
/* noslop-enable */
fn main() {}
";
    let d = doc(src, CodeLanguage::Rust);
    let found: Vec<_> = d
        .directives
        .iter()
        .map(|x| (x.kind, d.slice(x.span)))
        .collect();
    assert_eq!(
        found,
        [
            (DirectiveKind::DisableFile, "/// noslop-disable-file R03"),
            (
                DirectiveKind::DisableLine,
                "<!-- noslop-disable-line P01 -->"
            ),
            (DirectiveKind::Enable, "/* noslop-enable */"),
        ]
    );
    assert_eq!(
        outline(src, CodeLanguage::Rust),
        ["P:説明の文と言えるでしょう。"]
    );
}

#[test]
fn doc_comments_without_japanese_still_carry_directives() {
    let src = "/// Reads the config. <!-- noslop-disable-file -->\nfn main() {}\n";
    let d = doc(src, CodeLanguage::Rust);
    assert!(d.blocks.is_empty());
    assert_eq!(d.directives.len(), 1);
    assert_eq!(
        d.slice(d.directives[0].span),
        "<!-- noslop-disable-file -->"
    );
}

#[test]
fn doc_comments_are_markdown() {
    let src = "/// # 例
///
/// - 一つ目の項目です。
/// - 二つ目の項目です。
///
/// 本文の **強調** と [リンク](https://example.com) です。
///
///     let indented = \"字下げしたコード\";
fn main() {}
";
    assert_eq!(
        outline(src, CodeLanguage::Rust),
        [
            "H1:例",
            "L:一つ目の項目です。",
            "L:二つ目の項目です。",
            "P:本文の 強調 と リンク です。",
        ]
    );
    let d = doc(src, CodeLanguage::Rust);
    let strong = d.blocks[3]
        .marks
        .iter()
        .find(|m| m.kind == crate::document::MarkKind::Strong)
        .unwrap();
    assert_eq!(d.slice(strong.span), "**強調**");
}

#[test]
fn ordinary_comments_are_not_markdown() {
    // 見出しの記号はそのまま段落の本文。Markdown の記法にない箇条書きの記号 (・) の行は、同じ段落の
    // まま前で文を切る (Markdown の文書と同じ)。Markdown の箇条書きの記号で始まる行は項目にする
    let src = "# ## 見出しではありません。
# 前の行は丁寧体でない
# ・中黒の行の前で文を切ります
# - 箇条書きの項目です。
";
    assert_eq!(
        outline(src, CodeLanguage::Bash),
        [
            "P:## 見出しではありません。前の行は丁寧体でない ・中黒の行の前で文を切ります",
            "L:箇条書きの項目です。",
        ]
    );
    let texts: Vec<String> = sentences(src, CodeLanguage::Bash)
        .into_iter()
        .map(|(t, _)| t)
        .collect();
    assert_eq!(
        texts,
        [
            "## 見出しではありません。",
            "前の行は丁寧体でない",
            "・中黒の行の前で文を切ります",
            "箇条書きの項目です。",
        ]
    );
}

#[test]
fn positions_survive_bom_crlf_and_tabs() {
    let src = "\u{FEFF}#\t一行目の説明です。\r\n#   二行目です。\r\necho 1\t# 行末です。\r\n";
    let d = doc(src, CodeLanguage::Bash);
    let got: Vec<_> = d
        .sentences
        .iter()
        .map(|s| (d.sentence_text(s), d.slice(s.span)))
        .collect();
    assert_eq!(
        got,
        [
            ("一行目の説明です。", "一行目の説明です。"),
            ("二行目です。", "二行目です。"),
            ("行末です。", "行末です。"),
        ]
    );
    // ファイル上の位置は BOM の分だけずれる
    let second = &d.sentences[1];
    assert_eq!(
        d.file_offset(second.span.start),
        src.find("二行目").unwrap()
    );
    assert_eq!(d.line_col(second.span.start), (2, 5));
}

#[test]
fn fragments_do_not_get_document_statistics() {
    use crate::engine::{Engine, EngineOptions};
    use crate::morph::MorphologyMode;
    use crate::rules::{RuleUnit, builtin_rules};

    // 同じ長さの文が 24 続く (Markdown なら文長の単調さ R01 などの文書の統計に当たる)
    let lines: Vec<String> = (0..24)
        .map(|i| {
            format!(
                "設定の値を順に読み込んで{}番目の結果を返す処理です。",
                ["一", "二", "三", "四"][i % 4]
            )
        })
        .collect();
    let markdown = lines.join("\n\n");
    let code: String = lines.iter().map(|l| format!("# {l}\n")).collect();
    let mut options = EngineOptions::default();
    options.morphology.mode = MorphologyMode::Off;
    let engine = Engine::new(options).unwrap();
    let document_rules: Vec<&str> = builtin_rules(crate::genre::Genre::General)
        .iter()
        .filter(|r| r.unit() == RuleUnit::Document)
        .map(|r| r.meta().id)
        .collect();
    let prose = engine.lint_source("a.md".into(), markdown, SourceFormat::Markdown);
    assert!(
        prose
            .diagnostics
            .iter()
            .any(|d| document_rules.contains(&d.rule_id.as_str())),
        "Markdown では文書の統計のルールが指摘する"
    );
    let comments = engine.lint_source("a.sh".into(), code, SourceFormat::Code(CodeLanguage::Bash));
    assert_eq!(comments.doc.sentences.len(), 24);
    assert!(
        comments
            .diagnostics
            .iter()
            .all(|d| !document_rules.contains(&d.rule_id.as_str())),
        "{:?}",
        comments.diagnostics
    );
}
