//! コードのファイルのコメントを、文章として検査するためのブロックにする (tree-sitter)。
//!
//! 言語ごとの文法でコメント (と Python の docstring) を取り出し、コメントの記号を外した本文を
//! ブロックにする。位置は原文 (コードのファイル) のバイト位置に戻す。コードのファイルの文書は
//! 互いに独立した断片の集まり ([`DocumentKind::Fragments`](crate::document::DocumentKind)) として
//! 扱い、文ごとに判定するルールだけを当てる。文字列のリテラルは読まない。
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

use crate::document::{Block, Directive};

/// コメントを読める言語。
///
/// 文法はすべてバイナリに入れている (feature で分けない)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum CodeLanguage {
    Rust,
    JavaScript,
    TypeScript,
    Tsx,
    Python,
    Go,
    Java,
    C,
    Cpp,
    CSharp,
    Ruby,
    Php,
    Swift,
    Kotlin,
    Bash,
    Yaml,
    Toml,
    Html,
    Css,
    Lua,
}

impl CodeLanguage {
    /// すべての言語。
    pub const ALL: [CodeLanguage; 20] = [
        CodeLanguage::Rust,
        CodeLanguage::JavaScript,
        CodeLanguage::TypeScript,
        CodeLanguage::Tsx,
        CodeLanguage::Python,
        CodeLanguage::Go,
        CodeLanguage::Java,
        CodeLanguage::C,
        CodeLanguage::Cpp,
        CodeLanguage::CSharp,
        CodeLanguage::Ruby,
        CodeLanguage::Php,
        CodeLanguage::Swift,
        CodeLanguage::Kotlin,
        CodeLanguage::Bash,
        CodeLanguage::Yaml,
        CodeLanguage::Toml,
        CodeLanguage::Html,
        CodeLanguage::Css,
        CodeLanguage::Lua,
    ];

    /// 表示名。
    pub fn name(self) -> &'static str {
        match self {
            CodeLanguage::Rust => "Rust",
            CodeLanguage::JavaScript => "JavaScript",
            CodeLanguage::TypeScript => "TypeScript",
            CodeLanguage::Tsx => "TSX",
            CodeLanguage::Python => "Python",
            CodeLanguage::Go => "Go",
            CodeLanguage::Java => "Java",
            CodeLanguage::C => "C",
            CodeLanguage::Cpp => "C++",
            CodeLanguage::CSharp => "C#",
            CodeLanguage::Ruby => "Ruby",
            CodeLanguage::Php => "PHP",
            CodeLanguage::Swift => "Swift",
            CodeLanguage::Kotlin => "Kotlin",
            CodeLanguage::Bash => "Bash",
            CodeLanguage::Yaml => "YAML",
            CodeLanguage::Toml => "TOML",
            CodeLanguage::Html => "HTML",
            CodeLanguage::Css => "CSS",
            CodeLanguage::Lua => "Lua",
        }
    }

    /// この言語のファイルの拡張子 (小文字、ドットなし)。
    pub fn extensions(self) -> &'static [&'static str] {
        match self {
            CodeLanguage::Rust => &["rs"],
            CodeLanguage::JavaScript => &["js", "mjs", "cjs", "jsx"],
            CodeLanguage::TypeScript => &["ts", "mts", "cts"],
            CodeLanguage::Tsx => &["tsx"],
            CodeLanguage::Python => &["py", "pyi"],
            CodeLanguage::Go => &["go"],
            CodeLanguage::Java => &["java"],
            CodeLanguage::C => &["c", "h"],
            CodeLanguage::Cpp => &["cc", "cpp", "cxx", "hh", "hpp", "hxx"],
            CodeLanguage::CSharp => &["cs"],
            CodeLanguage::Ruby => &["rb"],
            CodeLanguage::Php => &["php"],
            CodeLanguage::Swift => &["swift"],
            CodeLanguage::Kotlin => &["kt", "kts"],
            CodeLanguage::Bash => &["sh", "bash", "zsh"],
            CodeLanguage::Yaml => &["yaml", "yml"],
            CodeLanguage::Toml => &["toml"],
            CodeLanguage::Html => &["html", "htm"],
            CodeLanguage::Css => &["css"],
            CodeLanguage::Lua => &["lua"],
        }
    }

    /// 拡張子 (大文字小文字を問わない、ドットなし) から言語を決める。
    pub fn from_extension(ext: &str) -> Option<Self> {
        let ext = ext.to_ascii_lowercase();
        Self::ALL
            .into_iter()
            .find(|lang| lang.extensions().contains(&ext.as_str()))
    }

    /// コメントを読めるコードの拡張子 (小文字、ドットなし)。
    pub fn known_extensions() -> impl Iterator<Item = &'static str> {
        Self::ALL
            .into_iter()
            .flat_map(|lang| lang.extensions().iter().copied())
    }
}

/// コードのファイルから、コメントの本文のブロックと抑制のコメントを取り出す。
///
/// ブロックの解析用テキストはコメントの記号を外した本文で、[`Block::to_source`] で原文
/// (`source`) のバイト位置に戻る。
pub fn parse(source: &str, language: CodeLanguage) -> (Vec<Block>, Vec<Directive>) {
    let Some(raws) = extract::comments(source, language) else {
        return (Vec::new(), Vec::new());
    };
    let (comments, mut directives) = body::prepare(source, raws);
    let mut blocks = Vec::new();
    for group in body::group(source, comments) {
        build::build(source, &group, &mut blocks, &mut directives);
    }
    blocks.sort_by_key(|b| b.span.start);
    directives.sort_by_key(|d| d.span.start);
    directives.dedup_by_key(|d| d.span.start);
    (blocks, directives)
}

#[cfg(test)]
mod tests;
