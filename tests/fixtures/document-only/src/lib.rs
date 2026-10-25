//! 別の tree-sitter を使うアプリケーションへの組み込みを確かめる。

#[test]
fn markdown_and_plain_text_can_be_linted_and_compared() {
    use noslop::engine::{Engine, EngineOptions};
    use noslop::{Document, SourceFormat};

    let _parser = tree_sitter::Parser::new();
    let engine = Engine::new(EngineOptions::default()).unwrap();
    let before = engine.lint(Document::markdown("<p>料金は100円です。</p>"));
    let after = engine.lint(Document::plain_text("料金は200円です。"));
    assert_eq!(before.doc.format, SourceFormat::Markdown);
    assert_eq!(before.doc.blocks[0].text.trim(), "料金は100円です。");
    assert_eq!(after.doc.format, SourceFormat::PlainText);
    let comparison = noslop::diff::compare(&engine, before, after).unwrap();
    assert!(!comparison.facts.removed.is_empty());
    assert!(!comparison.facts.added.is_empty());
}

#[test]
fn code_inputs_report_the_missing_feature_instead_of_becoming_plain_text() {
    use noslop::engine::{Engine, EngineOptions, Input};
    use noslop::language::CodeLanguage;
    use noslop::{Document, ParseError, ParseOptions, SourceFormat};

    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/lib.rs");
    let format = SourceFormat::from_path(&source);
    assert_eq!(format, SourceFormat::Code(CodeLanguage::Rust));
    assert!(matches!(
        Document::try_parse(
            "sample.rs",
            "// 日本語のコメント",
            format,
            &ParseOptions::default()
        ),
        Err(ParseError::CodeUnavailable(CodeLanguage::Rust))
    ));
    let engine = Engine::new(EngineOptions::default()).unwrap();
    let error = engine
        .try_lint_source("sample.rs".into(), "// 日本語のコメント".into(), format)
        .unwrap_err();
    assert!(error.message.contains("feature `code`"));
    let report = engine.run(vec![Input::Path(source)]);
    assert!(report.files.is_empty());
    assert_eq!(report.errors.len(), 1);
    assert!(report.errors[0].message.contains("feature `code`"));
}
