//! tree-sitter でコードを読み、コメントのノード (と Python の docstring) を取り出す。
//!
//! 言語ごとに、どのノードがコメントか、どの記号で始まるか、文書のコメント (Markdown か XML・HTML
//! で読むもの) かを決める。行ごとに飾りを外すなどの細部は [`super::body`] が扱う。
//!
//! ノードの種類の名前は文法の版で変わりうるので、文法ごとに実物を読ませて確かめたものだけを書く。
//! 言語は [`CodeLanguage`] を全部並べ、当てはまらない言語を黙って飛ばす既定の分岐は置かない。

use tree_sitter::{Node, Parser};

use super::CodeLanguage;
use crate::diagnostic::Span;

/// コメントの本文をどう読むか。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Reading {
    /// 普通のコメント。記号を外した行を、そのまま段落に組む。
    Plain,
    /// 文書のコメントと docstring。Markdown として読む。
    Markdown,
    /// XML・HTML で書く文書のコメント (C# の XML ドキュメントコメント、Javadoc)。タグを外して段落に組む。
    Markup,
}

/// コメントの形。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Shape {
    /// 行のコメント (`//`・`#`・`--`)。隣り合う行のものをまとめて読む。
    Line,
    /// ブロックのコメント (`/* */`・`<!-- -->`・`=begin`〜`=end`・`--[[ ]]`)。1 つで 1 つのまとまり。
    Block,
    /// Python の docstring (引用符の内側)。
    Docstring,
}

/// 取り出したコメント 1 つ。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RawComment {
    /// コメント全体の原文上の範囲 (末尾の改行は含めない)。
    pub span: Span,
    /// コメントの記号を除いた中身の、原文上の範囲。ブロックの行頭にある `*` の飾りや空白は、
    /// まだ含む。
    pub body: Span,
    pub shape: Shape,
    pub reading: Reading,
    /// 行のコメントを 1 つにまとめる条件で比べる記号の種類 (`//`・`///`・`//!`・`#`・`--`)。
    /// 行のコメントでなければ空。
    pub marker: &'static str,
}

/// この言語の文法 (このビルドに入っていなければ `None`)。
fn grammar(language: CodeLanguage) -> Option<tree_sitter::Language> {
    let grammar: tree_sitter::Language = match language {
        #[cfg(feature = "lang-rust")]
        CodeLanguage::Rust => tree_sitter_rust::LANGUAGE.into(),
        #[cfg(feature = "lang-javascript")]
        CodeLanguage::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
        #[cfg(feature = "lang-typescript")]
        CodeLanguage::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        #[cfg(feature = "lang-typescript")]
        CodeLanguage::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
        #[cfg(feature = "lang-python")]
        CodeLanguage::Python => tree_sitter_python::LANGUAGE.into(),
        #[cfg(feature = "lang-go")]
        CodeLanguage::Go => tree_sitter_go::LANGUAGE.into(),
        #[cfg(feature = "lang-java")]
        CodeLanguage::Java => tree_sitter_java::LANGUAGE.into(),
        #[cfg(feature = "lang-c")]
        CodeLanguage::C => tree_sitter_c::LANGUAGE.into(),
        #[cfg(feature = "lang-cpp")]
        CodeLanguage::Cpp => tree_sitter_cpp::LANGUAGE.into(),
        #[cfg(feature = "lang-csharp")]
        CodeLanguage::CSharp => tree_sitter_c_sharp::LANGUAGE.into(),
        #[cfg(feature = "lang-ruby")]
        CodeLanguage::Ruby => tree_sitter_ruby::LANGUAGE.into(),
        #[cfg(feature = "lang-php")]
        CodeLanguage::Php => tree_sitter_php::LANGUAGE_PHP.into(),
        #[cfg(feature = "lang-swift")]
        CodeLanguage::Swift => tree_sitter_swift::LANGUAGE.into(),
        #[cfg(feature = "lang-kotlin")]
        CodeLanguage::Kotlin => tree_sitter_kotlin_ng::LANGUAGE.into(),
        CodeLanguage::Bash => tree_sitter_bash::LANGUAGE.into(),
        #[cfg(feature = "lang-yaml")]
        CodeLanguage::Yaml => tree_sitter_yaml::LANGUAGE.into(),
        #[cfg(feature = "lang-toml")]
        CodeLanguage::Toml => tree_sitter_toml_ng::LANGUAGE.into(),
        #[cfg(feature = "lang-html")]
        CodeLanguage::Html => tree_sitter_html::LANGUAGE.into(),
        #[cfg(feature = "lang-css")]
        CodeLanguage::Css => tree_sitter_css::LANGUAGE.into(),
        #[cfg(feature = "lang-lua")]
        CodeLanguage::Lua => tree_sitter_lua::LANGUAGE.into(),
        // 文法を入れていない言語 (すべての feature を入れたビルドでは当たらない)
        #[allow(unreachable_patterns)]
        _ => return None,
    };
    Some(grammar)
}

/// `source` をこの言語の文法で読み、コメントを原文の順に返す。文法がこのビルドになければ `None`。
///
/// 構文の誤りがあっても、読めたところのコメントは返す (tree-sitter は誤りを含む木も作る)。
pub(super) fn comments(source: &str, language: CodeLanguage) -> Option<Vec<RawComment>> {
    let grammar = grammar(language)?;
    let mut parser = Parser::new();
    parser
        .set_language(&grammar)
        .unwrap_or_else(|e| panic!("{} の文法を読み込めません: {e}", language.name()));
    let tree = parser.parse(source, None)?;
    let mut out = Vec::new();
    // 深く入れ子になったコードでもスタックを使い切らないよう、再帰せずにカーソルでたどる
    let mut cursor = tree.walk();
    'walk: loop {
        let node = cursor.node();
        let comment = classify(language, node, source);
        let is_comment = comment.is_some();
        out.extend(comment);
        if language == CodeLanguage::Python {
            out.extend(docstring(node, source));
        }
        if !is_comment && cursor.goto_first_child() {
            continue;
        }
        while !cursor.goto_next_sibling() {
            if !cursor.goto_parent() {
                break 'walk;
            }
        }
    }
    out.sort_by_key(|c| c.span.start);
    out.dedup_by_key(|c| c.span.start);
    Some(out)
}

/// `//`・`/* */` で書く言語の、文書のコメントの記号と読み方 (`None` はその記号を文書のコメントに
/// しない)。
#[derive(Debug, Clone, Copy, Default)]
struct Docs {
    /// `///` の行のコメント (`////` は普通のコメント)。
    triple: Option<Reading>,
    /// `//!` の行のコメントと `/*!` のブロック。
    bang: Option<Reading>,
    /// `/**` のブロック (`/**/` と `/***` は普通のコメント)。
    block: Option<Reading>,
}

/// ノードがコメントなら、その書き方を返す。
fn classify(language: CodeLanguage, node: Node<'_>, source: &str) -> Option<RawComment> {
    use Reading::{Markdown, Markup};
    let kind = node.kind();
    // どの文法でも、コメントを表すノードの種類名は `comment` で終わる (`line_comment`・
    // `multiline_comment`・`js_comment` など)。ほかのノードは範囲を求める前に飛ばす
    if !kind.ends_with("comment") {
        return None;
    }
    let span = without_newline(source, node.start_byte(), node.end_byte());
    let text = &source[span.range()];
    match language {
        CodeLanguage::Rust => match kind {
            "line_comment" | "block_comment" => Some(rust(node, span, text)),
            _ => None,
        },
        // JSDoc (`/** */`) を Markdown で読む。`///` は TypeScript の参照の指示などで、文書の
        // コメントではない
        CodeLanguage::JavaScript | CodeLanguage::TypeScript | CodeLanguage::Tsx => match kind {
            "comment" => slash(span, text, Docs::block(Markdown)),
            _ => None,
        },
        CodeLanguage::Go => match kind {
            "comment" => slash(span, text, Docs::default()),
            _ => None,
        },
        // Javadoc (`/** */`) は HTML で書く。Java 23 からの `///` は Markdown で書く
        CodeLanguage::Java => match kind {
            "line_comment" | "block_comment" => slash(
                span,
                text,
                Docs {
                    triple: Some(Markdown),
                    bang: None,
                    block: Some(Markup),
                },
            ),
            _ => None,
        },
        // Doxygen の `///`・`//!`・`/**`・`/*!`
        CodeLanguage::C | CodeLanguage::Cpp => match kind {
            "comment" => slash(
                span,
                text,
                Docs {
                    triple: Some(Markdown),
                    bang: Some(Markdown),
                    block: Some(Markdown),
                },
            ),
            _ => None,
        },
        // XML ドキュメントコメント
        CodeLanguage::CSharp => match kind {
            "comment" => slash(
                span,
                text,
                Docs {
                    triple: Some(Markup),
                    bang: None,
                    block: Some(Markup),
                },
            ),
            _ => None,
        },
        CodeLanguage::Swift => match kind {
            "comment" | "multiline_comment" => slash(
                span,
                text,
                Docs {
                    triple: Some(Markdown),
                    bang: None,
                    block: Some(Markdown),
                },
            ),
            _ => None,
        },
        // KDoc (`/** */`)
        CodeLanguage::Kotlin => match kind {
            "line_comment" | "block_comment" => slash(span, text, Docs::block(Markdown)),
            _ => None,
        },
        // PHPDoc (`/** */`)。`#` の行のコメントも書ける (`#[` は属性で、コメントのノードにならない)
        CodeLanguage::Php => match kind {
            "comment" if text.starts_with('#') => hash(span, text),
            "comment" => slash(span, text, Docs::block(Markdown)),
            _ => None,
        },
        // `//` は tree-sitter-css が読める SCSS 風の行のコメント
        CodeLanguage::Css => match kind {
            "comment" | "js_comment" => slash(span, text, Docs::default()),
            _ => None,
        },
        CodeLanguage::Python | CodeLanguage::Bash | CodeLanguage::Yaml | CodeLanguage::Toml => {
            match kind {
                "comment" => hash(span, text),
                _ => None,
            }
        }
        CodeLanguage::Ruby => match kind {
            "comment" => ruby(span, text),
            _ => None,
        },
        CodeLanguage::Html => match kind {
            "comment" => html(span, text),
            _ => None,
        },
        CodeLanguage::Lua => match kind {
            "comment" => lua(span, text),
            _ => None,
        },
    }
}

impl Docs {
    /// `/** */` だけを文書のコメントとして `reading` で読む。
    fn block(reading: Reading) -> Self {
        Self {
            triple: None,
            bang: None,
            block: Some(reading),
        }
    }
}

/// `start..end` から末尾の改行 (`\n`・`\r\n`) を除いた範囲。
fn without_newline(source: &str, start: usize, mut end: usize) -> Span {
    while end > start && matches!(source.as_bytes()[end - 1], b'\n' | b'\r') {
        end -= 1;
    }
    Span::new(start, end)
}

/// 先頭から `c` が続くバイト数。
fn run_of(text: &str, c: u8) -> usize {
    text.bytes().take_while(|&b| b == c).count()
}

/// 行のコメント。先頭の `marker_len` バイトを記号として外す。
fn line(span: Span, marker_len: usize, marker: &'static str, reading: Reading) -> RawComment {
    RawComment {
        span,
        body: Span::new((span.start + marker_len).min(span.end), span.end),
        shape: Shape::Line,
        reading,
        marker,
    }
}

/// ブロックのコメント。先頭の `open` バイトと末尾の `close` バイトを記号として外す。
fn block(span: Span, open: usize, close: usize, reading: Reading) -> RawComment {
    let start = (span.start + open).min(span.end);
    let end = span.end.saturating_sub(close).max(start);
    RawComment {
        span,
        body: Span::new(start, end),
        shape: Shape::Block,
        reading,
        marker: "",
    }
}

/// `//` か `/*` で始まるコメント。
fn slash(span: Span, text: &str, docs: Docs) -> Option<RawComment> {
    if let Some(rest) = text.strip_prefix("//") {
        let run = run_of(text, b'/');
        let comment = if run == 3
            && let Some(reading) = docs.triple
        {
            line(span, 3, "///", reading)
        } else if run == 2
            && rest.starts_with('!')
            && let Some(reading) = docs.bang
        {
            line(span, 3, "//!", reading)
        } else {
            line(span, run, "//", Reading::Plain)
        };
        Some(comment)
    } else if let Some(rest) = text.strip_prefix("/*") {
        let close = close_len(text, "*/", 4);
        let doc = rest.starts_with('*') && !rest[1..].starts_with(['*', '/']);
        let comment = if doc && let Some(reading) = docs.block {
            block(span, 3, close, reading)
        } else if rest.starts_with('!')
            && let Some(reading) = docs.bang
        {
            block(span, 3, close, reading)
        } else {
            // 文書のコメントにしない言語の `/*!` (圧縮で消さないコメントの印) も、`!` を記号として外す
            let open = if rest.starts_with('!') { 3 } else { 2 };
            block(span, open, close, Reading::Plain)
        };
        Some(comment)
    } else {
        None
    }
}

/// 閉じの記号 `close` で終わっていればその長さ (閉じていないコメントは 0)。`min` は開きと閉じが
/// 重ならない最短の長さ (`/**/` なら 4)。
fn close_len(text: &str, close: &str, min: usize) -> usize {
    if text.len() >= min && text.ends_with(close) {
        close.len()
    } else {
        0
    }
}

/// Rust の `//`・`/* */`。文書のコメント (`///`・`//!`・`/** */`・`/*! */`) は文法の印で見分ける
/// (`////` や `/**/` は印が付かない)。
fn rust(node: Node<'_>, span: Span, text: &str) -> RawComment {
    let mut cursor = node.walk();
    let doc = node.children(&mut cursor).find_map(|c| match c.kind() {
        "outer_doc_comment_marker" => Some(false),
        "inner_doc_comment_marker" => Some(true),
        _ => None,
    });
    if node.kind() == "line_comment" {
        match doc {
            Some(inner) => line(
                span,
                3,
                if inner { "//!" } else { "///" },
                Reading::Markdown,
            ),
            None => line(span, run_of(text, b'/'), "//", Reading::Plain),
        }
    } else {
        let close = close_len(text, "*/", 4);
        match doc {
            Some(_) => block(span, 3, close, Reading::Markdown),
            None => block(span, 2, close, Reading::Plain),
        }
    }
}

/// `#` の行のコメント (`##` のように続く `#` もまとめて外す)。
fn hash(span: Span, text: &str) -> Option<RawComment> {
    let run = run_of(text, b'#');
    (run > 0).then(|| line(span, run, "#", Reading::Plain))
}

/// Ruby の `#` と `=begin`〜`=end`。
fn ruby(span: Span, text: &str) -> Option<RawComment> {
    if text.starts_with("=begin") {
        // 最後の行の `=end` から後ろ (同じ行の残りを含む) を閉じとする
        let close = text.rfind("\n=end").map_or(0, |i| text.len() - i);
        Some(block(span, "=begin".len(), close, Reading::Plain))
    } else {
        hash(span, text)
    }
}

/// HTML の `<!-- -->`。
fn html(span: Span, text: &str) -> Option<RawComment> {
    text.starts_with("<!--")
        .then(|| block(span, 4, close_len(text, "-->", 7), Reading::Plain))
}

/// Lua の `--` と `--[[ ]]`・`--[==[ ]==]`。
fn lua(span: Span, text: &str) -> Option<RawComment> {
    let rest = text.strip_prefix("--")?;
    if let Some(after) = rest.strip_prefix('[') {
        let level = run_of(after, b'=');
        if after[level..].starts_with('[') {
            let open = "--[".len() + level + 1;
            let closing = format!("]{}]", "=".repeat(level));
            let close = close_len(text, &closing, open + closing.len());
            return Some(block(span, open, close, Reading::Plain));
        }
    }
    Some(line(span, run_of(text, b'-'), "--", Reading::Plain))
}

/// Python の docstring (モジュール・クラス・関数の本体で、コメントを除いた最初の文にある文字列)。
///
/// f 文字列・テンプレート文字列・バイト列は docstring にならないので外す。
fn docstring(node: Node<'_>, source: &str) -> Option<RawComment> {
    let body = match node.kind() {
        "module" => node,
        "class_definition" | "function_definition" => node.child_by_field_name("body")?,
        _ => return None,
    };
    let mut cursor = body.walk();
    let first = body
        .named_children(&mut cursor)
        .find(|n| n.kind() != "comment")?;
    if first.kind() != "expression_statement" || first.named_child_count() != 1 {
        return None;
    }
    let string = first.named_child(0)?;
    if string.kind() != "string" || string.child_count() < 2 {
        return None;
    }
    let open = string.child(0)?;
    let close = string.child(string.child_count() - 1)?;
    if open.kind() != "string_start" || close.kind() != "string_end" {
        return None;
    }
    let prefix = &source[open.start_byte()..open.end_byte()];
    if prefix.contains(['f', 'F', 't', 'T', 'b', 'B']) {
        return None;
    }
    Some(RawComment {
        span: Span::new(string.start_byte(), string.end_byte()),
        body: Span::new(open.end_byte(), close.start_byte()),
        shape: Shape::Docstring,
        reading: Reading::Markdown,
        marker: "",
    })
}
