//! 静的な表示文言の抽出、断片の境界、原文への位置対応の回帰テスト。

use super::*;
use crate::document::{Document, DocumentKind, ParseOptions, SourceFormat};

fn doc(source: &str, language: CodeLanguage) -> Document {
    Document::parse(
        "sample",
        source,
        SourceFormat::Code(language),
        &ParseOptions::default(),
    )
}
fn texts(source: &str, language: CodeLanguage) -> Vec<String> {
    doc(source, language)
        .blocks
        .into_iter()
        .map(|b| b.text.trim().to_string())
        .collect()
}
fn exact(d: &Document) {
    for b in &d.blocks {
        for (at, c) in b
            .text
            .char_indices()
            .filter(|(_, c)| crate::text::is_japanese(*c))
        {
            assert_eq!(
                d.slice(b.to_source(at..at + c.len_utf8())),
                c.to_string(),
                "{}: {:?}",
                d.name,
                b.text
            );
        }
    }
}

#[test]
fn every_language_reads_static_text_at_its_source_position() {
    use CodeLanguage::*;
    let cases = [
        (Rust, r#"let x = "保存します。";"#),
        (JavaScript, r#"const x = "保存します。";"#),
        (TypeScript, r#"const x: string = "保存します。";"#),
        (Tsx, r#"const x = <p>保存します。</p>;"#),
        (Python, r#"x = "保存します。""#),
        (Go, r#"package a; var x = "保存します。""#),
        (Java, r#"class A { String x = "保存します。"; }"#),
        (C, r#"char *x = "保存します。";"#),
        (Cpp, r#"auto x = "保存します。";"#),
        (CSharp, r#"class A { string x = "保存します。"; }"#),
        (Ruby, r#"x = "保存します。""#),
        (Php, r#"<?php $x = "保存します。";"#),
        (Swift, r#"let x = "保存します。""#),
        (Kotlin, r#"val x = "保存します。""#),
        (Bash, r#"echo "保存します。""#),
        (Yaml, "message: 保存します。\n"),
        (Toml, "message = \"保存します。\"\n"),
        (Html, "<p>保存します。</p>"),
        (Css, "p { content: \"保存します。\"; }"),
        (Lua, "local x = \"保存します。\""),
    ];
    assert_eq!(cases.len(), CodeLanguage::ALL.len());
    for (lang, src) in cases {
        let d = doc(src, lang);
        assert_eq!(d.blocks.len(), 1, "{lang:?}: {src}");
        assert_eq!(d.blocks[0].text, "保存します。", "{lang:?}");
        assert_eq!(d.kind, DocumentKind::Fragments);
        exact(&d);
    }
}

#[test]
fn raw_and_multiline_delimiters_are_not_part_of_the_text() {
    use CodeLanguage::*;
    for (lang, src) in [
        (Rust, r##"let x = r#"保存します。"#;"##),
        (Go, "package a; var x = `保存します。`"),
        (Python, "x = r'''保存します。'''"),
        (Java, "class A { String x = \"\"\"\n保存します。\n\"\"\"; }"),
        (Cpp, r#"auto x = R"end(保存します。)end";"#),
        (
            CSharp,
            "class A { string x = \"\"\"\"保存します。\"\"\"\"; }",
        ),
        (Ruby, "x = %q(保存します。)"),
        (Swift, "let x = #\"保存します。\"#"),
        (Kotlin, "val x = \"\"\"保存します。\"\"\""),
        (Bash, "echo '保存します。'"),
        (Toml, "message = '''保存します。'''\n"),
        (Lua, "local x = [=[保存します。]=]"),
    ] {
        assert_eq!(texts(src, lang), ["保存します。"], "{lang:?}");
        exact(&doc(src, lang));
    }
    assert_eq!(texts("local x = [=[[]日本語]=]", Lua), ["[]日本語"]);
}

#[test]
fn interpolation_keeps_static_parts_and_expression_strings_independent() {
    use CodeLanguage::*;
    for (lang, src) in [
        (JavaScript, "const x = `前半${\"式の中\"}後半`;"),
        (Python, "x = f\"前半{\"式の中\"}後半\""),
        (CSharp, "class A { string x = $\"前半{\"式の中\"}後半\"; }"),
        (Ruby, "x = \"前半#{\"式の中\"}後半\""),
        (Swift, "let x = \"前半\\(f(\"式の中\"))後半\""),
        (Kotlin, "val x = \"前半${\"式の中\"}後半\""),
        (Bash, "echo \"前半$(echo '式の中')後半\""),
    ] {
        assert_eq!(texts(src, lang), ["前半", "式の中", "後半"], "{lang:?}");
        exact(&doc(src, lang));
    }
    assert_eq!(
        texts("<?php $x = \"前半{$value}後半\";", Php),
        ["前半", "後半"]
    );
    assert_eq!(
        texts("let x = #\"前半\\#(f(\"式の中\"))後半\"#", Swift),
        ["前半", "式の中", "後半"]
    );
    assert_eq!(
        texts(
            "class A { string x = $$\"\"\"前半{{\"式の中\"}}後半\"\"\"; }",
            CSharp
        ),
        ["前半", "式の中", "後半"]
    );
}

#[test]
fn decoding_follows_each_language_and_preserves_transformed_ranges() {
    use CodeLanguage::*;
    for (lang, src, raw) in [
        (JavaScript, r#"const x = "\u65e5\u672c";"#, r"\u65e5\u672c"),
        (Rust, r#"let x = "\u{65e5}\u{672c}";"#, r"\u{65e5}\u{672c}"),
        (Python, r#"x = "\u65e5\u672c""#, r"\u65e5\u672c"),
        (
            Php,
            r#"<?php $x = "\u{65e5}\u{672c}";"#,
            r"\u{65e5}\u{672c}",
        ),
        (Bash, r#"echo $'\u65e5\u672c'"#, r"\u65e5\u672c"),
        (
            Swift,
            r##"let x = #"\#u{65e5}\#u{672c}"#"##,
            r"\#u{65e5}\#u{672c}",
        ),
        (Css, r#"p { content: "\65e5 \672c "; }"#, "\\65e5 \\672c "),
        (Html, "<p>&#x65e5;&#26412;</p>", "&#x65e5;&#26412;"),
    ] {
        let d = doc(src, lang);
        assert_eq!(d.blocks.len(), 1, "{lang:?}");
        assert_eq!(d.blocks[0].text, "日本", "{lang:?}");
        assert_eq!(
            d.slice(d.blocks[0].to_source(0.."日本".len())),
            raw,
            "{lang:?}"
        );
    }
    for (lang, src) in [
        (Bash, r#"echo "\u65e5\u672c""#),
        (Php, r#"<?php $x = "\u65e5\u672c";"#),
        (Swift, r##"let x = #"\u{65e5}\u{672c}"#"##),
        (Css, r#"p { content: "\u65e5\u672c"; }"#),
        (Python, r#"x = r"\u65e5\u672c""#),
    ] {
        assert!(texts(src, lang).is_empty(), "{lang:?}");
    }
    let d = doc(r#"const x = "\uD840\uDC00日本";"#, JavaScript);
    assert_eq!(d.blocks[0].text, "𠀀日本");
    assert_eq!(d.slice(d.blocks[0].to_source(0..4)), r"\uD840\uDC00");
    let d = doc("<p>日本&NotEqualTilde;語</p>", Html);
    assert_eq!(d.blocks[0].text, "日本≂̸語");
    assert_eq!(d.slice(d.blocks[0].to_source(6..11)), "&NotEqualTilde;");
}

#[test]
fn css_hex_escapes_keep_unicode_spaces_and_their_source_ranges() {
    for space in ['\u{a0}', '\u{b}', '\u{2003}', '\u{3000}'] {
        let source = format!("p {{ content: \"日\\672c{space}語\"; }}");
        let d = doc(&source, CodeLanguage::Css);
        assert_eq!(d.blocks.len(), 1);
        let b = &d.blocks[0];
        assert_eq!(b.text, format!("日本{space}語"));
        assert_eq!(d.slice(b.to_source(3..6)), r"\672c");
        assert_eq!(
            d.slice(b.to_source(6..6 + space.len_utf8())),
            space.to_string()
        );
    }
}

#[test]
fn css_hex_escapes_consume_one_css_whitespace_character() {
    for space in [" ", "\t", "\n", "\r", "\r\n", "\u{c}"] {
        let source = format!("p {{ content: \"日\\672c{space} 語\"; }}");
        let d = doc(&source, CodeLanguage::Css);
        assert_eq!(d.blocks.len(), 1, "{space:?}");
        let b = &d.blocks[0];
        assert_eq!(b.text, "日本 語", "{space:?}");
        assert_eq!(d.slice(b.to_source(3..6)), format!("\\672c{space}"));
        assert_eq!(d.slice(b.to_source(6..7)), " ");
    }
}

#[test]
fn css_line_continuations_follow_css_newline_preprocessing() {
    for newline in ["\n", "\r", "\r\n", "\u{c}"] {
        let source = format!("p {{ content: \"日\\{newline}本語\"; }}");
        let d = doc(&source, CodeLanguage::Css);
        assert_eq!(d.blocks.len(), 1, "{newline:?}");
        assert_eq!(d.blocks[0].text, "日本語", "{newline:?}");
        exact(&d);
    }
}

#[test]
fn css_invalid_code_points_become_replacement_characters() {
    for escape in [r"\0 ", r"\d800 ", r"\dfff ", r"\110000 ", r"\ffffff "] {
        let source = format!("p {{ content: \"日本{escape}語\"; }}");
        let d = doc(&source, CodeLanguage::Css);
        assert_eq!(d.blocks.len(), 1, "{escape}");
        let b = &d.blocks[0];
        assert_eq!(b.text, "日本�語", "{escape}");
        assert_eq!(d.slice(b.to_source(6..9)), escape);
    }
}

#[test]
fn html_entities_do_not_consume_literal_ampersands_or_attribute_markup() {
    let source = "<p title=\"日本&<b>本文;\" aria-label=\"日本&&#x672c;\">表示文</p>";
    let d = doc(source, CodeLanguage::Html);
    assert_eq!(d.blocks[0].text, "日本&<b>本文;");
    assert_eq!(d.blocks[1].text, "日本&本");
    assert_eq!(d.slice(d.blocks[1].to_source(7..10)), "&#x672c;");
    exact(&doc(
        "<p title=\"日本&<b>本文;\">表示文</p>",
        CodeLanguage::Html,
    ));
}

#[test]
fn long_display_attributes_keep_entities_and_their_source_ranges() {
    let ampersands = "&".repeat(16_384);
    let source = format!("<p title=\"日本{ampersands}&amp;&#x672c;\">表示文</p>");
    let d = doc(&source, CodeLanguage::Html);
    let b = &d.blocks[0];
    assert_eq!(b.text, format!("日本{ampersands}&本"));
    assert_eq!(
        d.slice(b.to_source(b.text.len() - 3..b.text.len())),
        "&#x672c;"
    );
}

#[test]
fn swift_extended_delimiters_preserve_escape_lengths_and_source_ranges() {
    for hash in [1, 2, 4] {
        let hashes = "#".repeat(hash);
        let prefix = format!("\\{hashes}");
        let source = format!(
            "let x = {hashes}\"{prefix}u{{65e5}}{prefix}u{{672c}}{prefix}n次{prefix}t文{prefix}0{prefix}\\{prefix}\"\"{hashes}"
        );
        let d = doc(&source, CodeLanguage::Swift);
        assert_eq!(d.blocks.len(), 1, "{source}");
        let b = &d.blocks[0];
        assert_eq!(b.text, "日本\n次\t文\0\\\"", "{source}");
        assert_eq!(d.slice(b.to_source(0..3)), format!("{prefix}u{{65e5}}"));
        assert_eq!(d.slice(b.to_source(3..6)), format!("{prefix}u{{672c}}"));
        assert_eq!(d.slice(b.to_source(6..7)), format!("{prefix}n"));
    }
}

#[test]
fn markup_joins_inline_text_but_keeps_controls_blocks_and_expressions_apart() {
    for lang in [
        CodeLanguage::Html,
        CodeLanguage::Tsx,
        CodeLanguage::JavaScript,
    ] {
        let src = "<div><p>保存を<strong>確認</strong>します。<br/>次の文です。</p><button>開く</button><button>閉じる</button><span>前の文</span><Widget>独立した文</Widget><span>後の文</span></div>";
        assert_eq!(
            texts(src, lang),
            [
                "保存を確認します。\n次の文です。",
                "開く",
                "閉じる",
                "前の文",
                "独立した文",
                "後の文"
            ],
            "{lang:?}"
        );
        let d = doc(src, lang);
        assert_eq!(d.blocks[0].sentence_breaks.len(), 1);
        assert_eq!(
            d.slice(d.blocks[0].to_source(0.."保存を確認します。".len())),
            "保存を<strong>確認</strong>します。"
        );
        exact(&d);
    }
    assert_eq!(
        texts(
            "const x = <p>前の文{\"式の中\"}後の文</p>;",
            CodeLanguage::Tsx
        ),
        ["前の文", "式の中", "後の文"]
    );
}

#[test]
fn html_line_breaks_keep_following_text_and_exact_source_ranges() {
    for tag in ["<br>", "<br/>", "<br />", "<BR>", "<br class=\"line\">"] {
        let src = format!("<p>前の行{tag}次の行&amp;<span>続き</span>です。</p>");
        let d = doc(&src, CodeLanguage::Html);
        assert_eq!(d.blocks.len(), 1, "{tag}: {:?}", d.blocks);
        let b = &d.blocks[0];
        assert_eq!(b.text, "前の行\n次の行&続きです。", "{tag}");
        let at = "前の行".len();
        assert_eq!(b.sentence_breaks, [at]);
        assert_eq!(d.slice(b.to_source(at..at + 1)), tag);
        let at = b.text.find('&').unwrap();
        assert_eq!(d.slice(b.to_source(at..at + 1)), "&amp;");
        exact(&d);
    }
}

#[test]
fn consecutive_html_line_breaks_keep_each_following_line() {
    let src = "<p>前の行<br>次の行<br><br>最後の行</p>";
    let d = doc(src, CodeLanguage::Html);
    assert_eq!(d.blocks.len(), 1);
    let b = &d.blocks[0];
    assert_eq!(b.text, "前の行\n次の行\n\n最後の行");
    let breaks: Vec<_> = b.text.match_indices('\n').map(|(at, _)| at).collect();
    assert_eq!(b.sentence_breaks, breaks);
    for (at, (source_at, tag)) in breaks.iter().zip(src.match_indices("<br>")) {
        assert_eq!(
            b.to_source(*at..*at + 1),
            Span::new(source_at, source_at + tag.len())
        );
    }
    exact(&d);
}

#[test]
fn html_line_breaks_preserve_whitespace_like_self_closing_tags() {
    for src in [
        "<p>前の行<br>\n  次の行</p>",
        "<p>前の行<br>\n<br>\n最後の行</p>",
        "<p><span>前の行<br>次の行 </span>続き</p>",
    ] {
        let d = doc(src, CodeLanguage::Html);
        let other = doc(&src.replace("<br>", "<br/>"), CodeLanguage::Html);
        assert_eq!(d.blocks.len(), other.blocks.len(), "{src}");
        for (b, other) in d.blocks.iter().zip(&other.blocks) {
            assert_eq!(b.text, other.text, "{src}");
            assert_eq!(b.sentence_breaks, other.sentence_breaks, "{src}");
        }
        exact(&d);
    }
}

#[test]
fn html_line_breaks_keep_following_text_when_the_break_is_hidden() {
    for tag in ["<br hidden>", "<br hidden/>"] {
        let src = format!("<p>前の行{tag}次の行</p>");
        assert_eq!(texts(&src, CodeLanguage::Html), ["前の行", "次の行"]);
        exact(&doc(&src, CodeLanguage::Html));
    }
}

#[test]
fn html_line_breaks_keep_prose_directly_under_the_document() {
    for tag in ["<br>", "<br/>", "<br />"] {
        let src = format!("前の行{tag}次の行 <span>続き</span>");
        let d = doc(&src, CodeLanguage::Html);
        assert_eq!(d.blocks.len(), 1, "{tag}");
        assert_eq!(d.blocks[0].text, "前の行\n次の行 続き", "{tag}");
        exact(&d);
    }
    assert_eq!(
        texts("<img alt=\"画像の説明\"/>", CodeLanguage::Html),
        ["画像の説明"]
    );
}

#[test]
fn html_line_breaks_read_display_attributes_once() {
    for tag in ["<br title=\"改行の説明\">", "<br title=\"改行の説明\"/>"] {
        let src = format!("<p>前の行{tag}次の行</p>");
        // 属性は本文とは別の断片として、一度だけ取り出す。
        assert_eq!(
            texts(&src, CodeLanguage::Html),
            ["前の行\n次の行", "改行の説明"]
        );
        exact(&doc(&src, CodeLanguage::Html));
    }
}

#[test]
fn html_line_breaks_keep_markdown_element_context() {
    use crate::document::BlockKind;
    for (src, kind, quote) in [
        (
            "<blockquote><p>前の行<br>次の行</p></blockquote>",
            BlockKind::Paragraph,
            true,
        ),
        (
            "<ul><li>前の行<br>次の行</li></ul>",
            BlockKind::ListItem,
            false,
        ),
        (
            "<table><tr><td>前の行<br>次の行</td></tr></table>",
            BlockKind::TableCell,
            false,
        ),
    ] {
        let d = Document::markdown(src);
        assert_eq!(d.blocks.len(), 1, "{src}");
        assert_eq!(d.blocks[0].text, "前の行\n次の行", "{src}");
        assert_eq!(d.blocks[0].kind, kind, "{src}");
        assert_eq!(d.blocks[0].in_quote, quote, "{src}");
        exact(&d);
    }
}

#[test]
fn excluded_attributes_and_subtrees_do_not_leak_expression_strings() {
    use CodeLanguage::*;
    let jsx = r#"const x = <div className={"除外する値"} href="除外するURL" onClick={() => "除外する式"} title={"表示する属性"}><p hidden={true}>隠れた文{"隠れた式"}</p><p hidden={false}>見える文</p><p aria-hidden="true">視覚的な文</p><script>{"除外するコード"}</script><pre>除外する例</pre><p>残る文</p></div>;"#;
    assert_eq!(
        texts(jsx, Tsx),
        ["表示する属性", "見える文", "視覚的な文", "残る文"]
    );
    let html = r#"<html><head><meta content="除外する値"><title>ページの題</title></head><body><p hidden="false" title="隠れた属性">隠れた文</p><input placeholder="入力の案内"><img alt="画像の説明"><template>除外する文</template><style>p { content: "除外するCSS"; }</style><p>残る文</p></body></html>"#;
    assert_eq!(
        texts(html, Html),
        ["ページの題", "入力の案内", "画像の説明", "残る文"]
    );
}

#[test]
fn identifiers_bytes_regex_and_url_only_values_are_excluded() {
    use CodeLanguage::*;
    assert_eq!(
        texts(
            r#"const x = {"識別用のキー": "表示する値"}; const r = /除外する式/; const u = "https://example.com/日本語"; const p = "https://example.com/日本語 を開きます。";"#,
            JavaScript
        ),
        ["表示する値", "https://example.com/日本語 を開きます。"]
    );
    assert_eq!(
        texts("日本語のキー: 表示する値\n\"別のキー\": \"別の値\"\n", Yaml),
        ["表示する値", "別の値"]
    );
    assert_eq!(
        texts("\"日本語のキー\" = \"表示する値\"\n", Toml),
        ["表示する値"]
    );
    assert_eq!(
        texts(r#"x = b"除外するバイト列"; y = "表示する値""#, Python),
        ["表示する値"]
    );
    assert_eq!(
        texts(
            r#"let x = b"除外するバイト列"; let y = "表示する値";"#,
            Rust
        ),
        ["表示する値"]
    );
    assert_eq!(
        texts(
            "p { font-family: \"識別用の名前\"; content: \"表示する値\"; }",
            Css
        ),
        ["表示する値"]
    );
}

#[test]
fn python_docstrings_are_owned_once_and_literals_do_not_supply_directives() {
    let src =
        "\"\"\"説明の文です。\"\"\"\nx = \"noslop-disable P01 日本語\"\ny = \"別の文です。\"\n";
    let d = doc(src, CodeLanguage::Python);
    assert_eq!(d.blocks.len(), 3);
    assert!(d.directives.is_empty());
    assert_eq!(d.blocks[0].text, "説明の文です。");
}

#[test]
fn bom_crlf_multibyte_and_broken_syntax_keep_valid_source_boundaries() {
    let d = doc(
        "\u{FEFF}const x = \"日本語\r\n次の行\";\r\nconst y = <p>保存します。</p>;",
        CodeLanguage::Tsx,
    );
    exact(&d);
    assert_eq!(d.bom_len, 3);
    for b in &d.blocks {
        assert!(d.source.is_char_boundary(b.span.start));
        assert!(d.source.is_char_boundary(b.span.end));
    }
    for lang in CodeLanguage::ALL {
        for src in [
            "\"日本語",
            "<p title=\"日本語",
            "日本語 \"\\uD800\"",
            "[[[[日本語",
            "\"日本語\\",
        ] {
            let _ = doc(src, lang);
        }
    }
}

#[test]
fn independent_literals_never_form_a_document_or_a_sentence() {
    let d = doc(
        "const x = \"前半\" + \"後半\"; const y = \"別の文。\\n\\n次の段落。\";",
        CodeLanguage::JavaScript,
    );
    assert_eq!(d.kind, DocumentKind::Fragments);
    assert_eq!(d.blocks.len(), 3);
    assert_eq!(d.blocks[0].text, "前半");
    assert_eq!(d.blocks[1].text, "後半");
    let engine = crate::engine::Engine::new(crate::engine::EngineOptions {
        experimental: true,
        ..Default::default()
    })
    .unwrap();
    let report = engine.lint(d);
    assert!(
        report
            .diagnostics
            .iter()
            .all(|diagnostic| !diagnostic.rule_id.starts_with('R')
                && !diagnostic.rule_id.starts_with('S'))
    );
}

#[test]
fn jsx_formatting_and_html_whitespace_follow_distinct_display_rules() {
    let src = "<p>重要な\n  <strong>ポイント</strong>です。</p>";
    let jsx = doc(src, CodeLanguage::Tsx);
    assert_eq!(jsx.blocks[0].text, "重要なポイントです。");
    exact(&jsx);
    let html = doc(src, CodeLanguage::Html);
    assert_eq!(html.blocks[0].text, "重要な ポイントです。");
    assert_eq!(html.slice(html.blocks[0].to_source(9..10)), "\n  ");
    assert_eq!(
        texts("<p>\n  一行目\n  二行目\n</p>", CodeLanguage::Tsx),
        ["一行目 二行目"]
    );
}

#[test]
fn heredocs_and_single_quoted_yaml_keep_their_static_boundaries() {
    use CodeLanguage::*;
    assert_eq!(
        texts("cat <<'END'\n前の文${value}後の文\nEND\n", Bash),
        ["前の文${value}後の文"]
    );
    assert_eq!(
        texts("cat <<END\n前の文${value}後の文\nEND\n", Bash),
        ["前の文", "後の文"]
    );
    assert_eq!(
        texts("x = <<END\n前の文#{\"式の中\"}後の文\nEND\n", Ruby),
        ["前の文", "式の中", "後の文"]
    );
    assert_eq!(
        texts("<?php $x = <<<END\n前の文{$value}後の文\nEND;", Php),
        ["前の文", "後の文"]
    );
    assert_eq!(texts("<?php $x = <<<'END'\n日本語\nEND;", Php), ["日本語"]);
    assert_eq!(
        texts("message: '日本語''の文\\u65e5'\n", Yaml),
        ["日本語'の文\\u65e5"]
    );
}

#[test]
fn quoted_ruby_heredocs_and_lua_initial_newlines_follow_raw_string_rules() {
    assert!(texts("x = <<'END'\n\\u65e5\\u672c\nEND\n", CodeLanguage::Ruby).is_empty());
    assert_eq!(
        texts("x = <<END\n\\u65e5\\u672c\nEND\n", CodeLanguage::Ruby),
        ["日本"]
    );
    let d = doc("local x = [=[\r\n日本語]=]", CodeLanguage::Lua);
    assert_eq!(d.blocks[0].text, "日本語");
    exact(&d);
}

#[test]
fn html_case_insensitivity_does_not_treat_jsx_components_as_intrinsic_tags() {
    assert_eq!(
        texts("<P>前の文<SPAN>続き</SPAN>です。</P>", CodeLanguage::Html),
        ["前の文続きです。"]
    );
    assert_eq!(
        texts(
            "const x = <p>前の文<Script>独立した文</Script>後の文</p>;",
            CodeLanguage::Tsx
        ),
        ["前の文", "独立した文", "後の文"]
    );
}

#[test]
fn rust_raw_strings_keep_literal_quotes_inside_the_raw_delimiter() {
    let src = r##"let x = r#"""保存します。"""#;"##;
    let d = doc(src, CodeLanguage::Rust);
    assert_eq!(d.blocks[0].text, "\"\"保存します。\"\"");
    exact(&d);
    assert_eq!(
        d.slice(d.blocks[0].to_source(0..d.blocks[0].text.len())),
        d.blocks[0].text
    );
}

#[test]
fn mixed_ruby_heredocs_and_arbitrary_percent_delimiters_remain_independent() {
    assert_eq!(
        texts(
            "values = [<<ONE, <<'TWO']\n\\u65e5\\u672c\nONE\n生の文\\u65e5\nTWO\n",
            CodeLanguage::Ruby
        ),
        ["日本", "生の文\\u65e5"]
    );
    assert_eq!(
        texts(
            "x = %q^保存します。^; y = %Q:前の文#{\"式の中\"}後の文:;",
            CodeLanguage::Ruby
        ),
        ["保存します。", "前の文", "式の中", "後の文"]
    );
}

#[test]
fn ruby_heredocs_keep_nested_scopes_and_delimiter_quotes_independent() {
    let sources = [
        (
            "def earlier\n  x = <<'INNER'\n内側の文\\u3068\nINNER\nend\nx = <<OUTER\n\\u975e常に重要\nOUTER\n",
            vec!["内側の文\\u3068", "非常に重要"],
        ),
        ("x = <<\"A'B\"\n\\u975e常に重要\nA'B\n", vec!["非常に重要"]),
        ("x = <<~\"A'B\"\n\\u975e常に重要\nA'B\n", vec!["非常に重要"]),
    ];
    for (src, expected) in sources {
        assert_eq!(texts(src, CodeLanguage::Ruby), expected);
        let engine = crate::engine::Engine::new(crate::engine::EngineOptions::default()).unwrap();
        let report = engine.lint(doc(src, CodeLanguage::Ruby));
        let d = report
            .diagnostics
            .iter()
            .find(|d| d.rule_id == "P06")
            .expect("引用形式に応じて復号した後続ヒアドキュメントを検査する");
        assert_eq!(report.doc.slice(d.span), "\\u975e常に重要");
    }
}

#[test]
fn csharp_variable_hex_and_c_wide_hex_preserve_the_complete_escape_ranges() {
    use CodeLanguage::*;
    for (lang, src) in [
        (CSharp, "class A { string x = \"\\x91cd\\x8981\"; }"),
        (C, "const wchar_t *x = L\"\\x91cd\\x8981\";"),
        (Cpp, "auto x = u\"\\x91cd\\x8981\";"),
    ] {
        let d = doc(src, lang);
        assert_eq!(d.blocks[0].text, "重要", "{lang:?}");
        assert_eq!(d.slice(d.blocks[0].to_source(0..3)), "\\x91cd");
        assert_eq!(d.slice(d.blocks[0].to_source(0..6)), "\\x91cd\\x8981");
    }
    assert_eq!(
        texts("class A { string x = \"\\x91cd要\"; }", CSharp),
        ["重要"]
    );
    assert_eq!(
        texts("const char *x = \"前の文\\x91cd後の文\";", C),
        ["前の文", "後の文"]
    );
    let d = doc("class A { string x = \"\\x9日本語\"; }", CSharp);
    assert_eq!(d.blocks[0].text, "\t日本語");
}

#[test]
fn static_hidden_boolean_is_read_from_the_ast_with_whitespace_and_comments() {
    let src = "const x = <div><p hidden={ true }>除外する文</p><p hidden={/* note */true}>除外する式{\"除外する値\"}</p><p hidden={ false }>見える文</p><p hidden={flag}>確定する文</p></div>;";
    assert_eq!(texts(src, CodeLanguage::Tsx), ["見える文", "確定する文"]);
}

#[test]
fn go_numeric_byte_escapes_decode_utf8_and_cut_invalid_byte_sequences() {
    for escaped in [
        "\\xe9\\x87\\x8d\\xe8\\xa6\\x81",
        "\\351\\207\\215\\350\\246\\201",
    ] {
        let src = format!("package a; var x = \"{escaped}\"");
        let d = doc(&src, CodeLanguage::Go);
        assert_eq!(d.blocks[0].text, "重要");
        assert_eq!(d.slice(d.blocks[0].to_source(0..3)), &escaped[..12]);
        assert_eq!(d.slice(d.blocks[0].to_source(0..6)), escaped);
    }
    assert_eq!(
        texts(r#"package a; var x = "前の文\xff後の文""#, CodeLanguage::Go),
        ["前の文", "後の文"]
    );
}

#[test]
fn go_byte_escaped_phrase_reaches_sentence_rules_at_the_original_escape_range() {
    let src = "package a; var x = \"この点は\\xe9\\x9d\\x9e常に重要です。\"";
    let engine = crate::engine::Engine::new(crate::engine::EngineOptions::default()).unwrap();
    let report = engine.lint(doc(src, CodeLanguage::Go));
    let d = report
        .diagnostics
        .iter()
        .find(|d| d.rule_id == "P06")
        .expect("復号した語句を検査する");
    assert_eq!(report.doc.slice(d.span), "\\xe9\\x9d\\x9e常に重要");
}

#[test]
fn escaped_backslashes_in_c_strings_are_not_reinterpreted_as_hex_escapes() {
    for lang in [CodeLanguage::C, CodeLanguage::Cpp] {
        let src = r#"const char *x = "\\x91cd日本語";"#;
        let d = doc(src, lang);
        assert_eq!(d.blocks[0].text, "\\x91cd日本語");
        exact(&d);
        assert_eq!(d.slice(d.blocks[0].to_source(0..1)), "\\\\");
        assert_eq!(
            texts(r#"const char *x = "前の文\351後の文";"#, lang),
            ["前の文", "後の文"]
        );
    }
}
