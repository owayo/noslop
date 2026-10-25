use super::*;
use crate::document::Document;
use crate::engine::{Engine, EngineOptions, Selection};

fn exact(doc: &Document) {
    for block in &doc.blocks {
        for (at, c) in block
            .text
            .char_indices()
            .filter(|(_, c)| crate::text::is_japanese(*c))
        {
            let raw = doc.slice(block.to_source(at..at + c.len_utf8()));
            if raw != c.to_string() {
                assert!(raw.starts_with('&'), "{raw}");
                assert_eq!(text::entity(raw).unwrap().1, c.to_string());
            }
        }
    }
}

#[test]
fn html_line_breaks_keep_markdown_element_context() {
    for (source, kind, quote) in [
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
        let doc = Document::markdown(source);
        assert_eq!(doc.blocks.len(), 1);
        assert_eq!(doc.blocks[0].text, "前の行\n次の行");
        assert_eq!(doc.blocks[0].kind, kind);
        assert_eq!(doc.blocks[0].in_quote, quote);
        exact(&doc);
    }
}

#[test]
fn prose_after_line_breaks_is_linted_at_its_original_position() {
    let engine = Engine::new(EngineOptions {
        selection: Selection {
            only: Some(vec!["P06".into()]),
            ..Selection::default()
        },
        ..EngineOptions::default()
    })
    .unwrap();
    let phrase = "非常に重要";
    for tag in ["<br>", "<br/>", "<br />", "<BR>", "<br class=\"line\">"] {
        for template in [
            "<p>前の文。{br}この案は非常に重要です。</p>",
            "<p>前の行{br}この案は非常に重要です。{br}別の案も非常に重要です。</p>",
            "<p>前の行{br}続き<span>この案は非常に重要です。</span></p>",
            "前の行{br}この案は非常に重要です。",
            "<p>前の行{br}\n  この案は非常に重要です。</p>",
            "<p>前の行{br}{br}この案は非常に重要です。</p>",
            "<p>前の行{br}この案は非常に重要です。",
        ] {
            let source = template.replace("{br}", tag);
            let file = engine.lint(Document::markdown(&source));
            let expected: Vec<_> = source.match_indices(phrase).map(|(at, _)| at).collect();
            assert_eq!(file.diagnostics.len(), expected.len(), "{source}");
            for (diagnostic, at) in file.diagnostics.iter().zip(expected) {
                assert_eq!(
                    diagnostic.span,
                    Span::new(at, at + phrase.len()),
                    "{source}"
                );
            }
            exact(&file.doc);
        }
    }
}

#[test]
fn attributes_raw_text_comments_and_optional_end_tags_keep_boundaries() {
    let source = "<div title='> 属性'><head><meta content='除く'><title>題名</title></head><p>本文&amp;後半。<!-- <p hidden> -->続き。</p><script>let x = '<p>除く</p>';</script><textarea>除く</textarea><ul><li>一つ目<li>二つ目</ul><table><tr><td>列一<td>列二</table></div>";
    let doc = Document::markdown(source);
    let texts: Vec<_> = doc.blocks.iter().map(|b| b.text.trim()).collect();
    assert_eq!(
        texts,
        [
            "題名",
            "本文&後半。続き。",
            "一つ目",
            "二つ目",
            "列一",
            "列二"
        ]
    );
    assert_eq!(doc.blocks[2].kind, BlockKind::ListItem);
    assert_eq!(doc.blocks[4].kind, BlockKind::TableCell);
    let at = doc.blocks[1].text.find('&').unwrap();
    assert_eq!(doc.slice(doc.blocks[1].to_source(at..at + 1)), "&amp;");
    exact(&doc);
}

#[test]
fn malformed_tags_and_unicode_never_produce_invalid_source_ranges() {
    for source in [
        "<div>本文<",
        "<div title='未閉じ>本文",
        "<div>本文&#x3042;\r\n後半</div>",
        "<div><!-- 未閉じのコメント",
        "<script>\"</not-script>\"</script><p>本文</p>",
        "<div>本文&unknown;後半</div>",
    ] {
        let doc = Document::markdown(source);
        exact(&doc);
        for block in &doc.blocks {
            assert!(source.get(block.span.range()).is_some(), "{source}");
        }
    }
}

#[test]
fn carried_exclusions_match_the_tags_that_are_carried() {
    let mut body = HtmlBody::default();
    assert!(body.read("<p hidden><span>除く本文").is_empty());
    // p は空行をまたいで持ち越さず、span 自体に hidden はない。
    assert!(!body.skipping());
    let blocks = body.read("残す本文</span>");
    assert_eq!(blocks[0].text, "残す本文");

    let mut body = HtmlBody::default();
    assert!(body.read("<div hidden><span>除く本文").is_empty());
    assert!(body.skipping());
    let blocks = body.read("除く本文</span></div><p>残す本文</p>");
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].text, "残す本文");
    assert!(!body.skipping());
}

#[test]
fn fallback_elements_and_self_closing_raw_text_tags_do_not_consume_later_markup() {
    for inner in [
        "<noscript><p>本文です。</p></noscript>",
        "<iframe><p>本文です。</p></iframe>",
        "<script src='a.js'/><p>本文です。</p>",
        "<style/><p>本文です。</p>",
        "<title/><p>本文です。</p>",
        "<Title /><p>本文です。</p>",
        "<textarea/><p>本文です。</p>",
    ] {
        let doc = Document::markdown(format!("<div>{inner}</div>"));
        assert_eq!(doc.blocks.len(), 1, "{inner}: {:?}", doc.blocks);
        assert_eq!(doc.blocks[0].text, "本文です。", "{inner}");
        exact(&doc);
    }
}

#[test]
fn excluded_elements_cannot_implicitly_close_a_visible_ancestor() {
    for tag in ["template", "code", "pre"] {
        let doc = Document::markdown(format!(
            "<ul><li>最初の項目<{tag}><li>除く項目</{tag}><li>次の項目</ul>"
        ));
        let text: Vec<_> = doc.blocks.iter().map(|b| b.text.trim()).collect();
        assert_eq!(text, ["最初の項目", "次の項目"], "{tag}");
        exact(&doc);
    }
}
