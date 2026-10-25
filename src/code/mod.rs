//! コードのコメントと静的な文言を、文章として検査するためのブロックにする (tree-sitter)。
//!
//! 言語ごとの文法でコメント (と Python の docstring) を取り出し、コメントの記号を外した本文を
//! ブロックにする。位置は原文 (コードのファイル) のバイト位置に戻す。コードのファイルの文書は
//! 互いに独立した断片の集まり ([`DocumentKind::Fragments`](crate::document::DocumentKind)) として
//! 扱い、文ごとに判定するルールだけを当てる。静的な文字列と JSX/HTML の文言も独立した断片として読む。
//!
//! 流れは次のとおり。
//!
//! 1. 文法でコメントのノードを取り出し、言語ごとの記号 (`//`・`#`・`/* */`・`<!-- -->` など) と、
//!    文書のコメント (`///`・`/** */`・docstring) かを決める ([`extract`])
//! 2. 記号・飾りを外した本文の行にし、shebang・ツールへの指示 (`eslint-disable` など)・抑制
//!    コメント (中身全体が `noslop-disable-next-line P01` の形のもの) を外す。同じ記号の行の
//!    コメントが隣り合う行に続けば 1 つにまとめ、著作権・ライセンスの表記を外す ([`body`])
//! 3. 日本語を含むまとまりをブロックにする。普通のコメントは段落に、文書のコメントと docstring は
//!    Markdown として、C# の XML ドキュメントコメントと Javadoc はタグを外して読む ([`build`])

mod body;
mod build;
mod extract;
mod static_text;

use crate::document::{Block, Directive, ParseOptions};

pub use crate::language::CodeLanguage;

/// コードからコメントと静的な文言のブロック、抑制のコメントを取り出す。
///
/// ブロックの解析用テキストはコメントの記号を外した本文で、[`Block::to_source`] で原文
/// (`source`) のバイト位置に戻る。
pub fn parse(source: &str, language: CodeLanguage) -> (Vec<Block>, Vec<Directive>) {
    parse_with_options(source, language, &ParseOptions::default())
}

/// 設定を指定して、コメントと静的な文言を取り出す。
pub fn parse_with_options(
    source: &str,
    language: CodeLanguage,
    options: &ParseOptions,
) -> (Vec<Block>, Vec<Directive>) {
    let Some(tree) = extract::tree(source, language) else {
        return (Vec::new(), Vec::new());
    };
    let raws = extract::comments(source, language, &tree);
    let owned: Vec<_> = raws.iter().map(|c| c.span).collect();
    let (comments, mut directives) = body::prepare(source, language, raws);
    let mut blocks = Vec::new();
    for group in body::group(source, comments) {
        build::build(source, &group, &mut blocks, &mut directives);
    }
    if options.static_text {
        blocks.extend(static_text::extract(
            source,
            language,
            tree.root_node(),
            &owned,
        ));
    }
    blocks.sort_by_key(|b| b.span.start);
    directives.sort_by_key(|d| d.span.start);
    directives.dedup_by_key(|d| d.span.start);
    (blocks, directives)
}

#[cfg(test)]
mod tests;
