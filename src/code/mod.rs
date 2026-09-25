//! コードのファイルのコメントを、文章として検査するためのブロックにする (tree-sitter)。
//!
//! 言語ごとの文法でコメント (と Python の docstring) を取り出し、コメントの記号を外した本文を
//! ブロックにする。位置は原文 (コードのファイル) のバイト位置に戻す。コードのファイルの文書は
//! 互いに独立した断片の集まり ([`DocumentKind::Fragments`](crate::document::DocumentKind)) として
//! 扱い、文ごとに判定するルールだけを当てる。

use crate::document::{Block, Directive};

/// コメントを読める言語。
///
/// 文法は言語ごとの feature (`lang-rust` など) で入れる。Bash だけは gws のコマンドの解析にも
/// 使うので常に入る。このビルドで読めるかは [`CodeLanguage::is_available`] で確かめる。
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
    /// すべての言語 (このビルドで読めないものも含む)。
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

    /// 文法を入れる feature の名前 (読めないときの案内に使う)。
    pub fn feature(self) -> &'static str {
        match self {
            CodeLanguage::Rust => "lang-rust",
            CodeLanguage::JavaScript => "lang-javascript",
            CodeLanguage::TypeScript | CodeLanguage::Tsx => "lang-typescript",
            CodeLanguage::Python => "lang-python",
            CodeLanguage::Go => "lang-go",
            CodeLanguage::Java => "lang-java",
            CodeLanguage::C => "lang-c",
            CodeLanguage::Cpp => "lang-cpp",
            CodeLanguage::CSharp => "lang-csharp",
            CodeLanguage::Ruby => "lang-ruby",
            CodeLanguage::Php => "lang-php",
            CodeLanguage::Swift => "lang-swift",
            CodeLanguage::Kotlin => "lang-kotlin",
            // 常に入る
            CodeLanguage::Bash => "",
            CodeLanguage::Yaml => "lang-yaml",
            CodeLanguage::Toml => "lang-toml",
            CodeLanguage::Html => "lang-html",
            CodeLanguage::Css => "lang-css",
            CodeLanguage::Lua => "lang-lua",
        }
    }

    /// このビルドで読めるか (文法を feature で入れたか)。
    pub fn is_available(self) -> bool {
        match self {
            CodeLanguage::Rust => cfg!(feature = "lang-rust"),
            CodeLanguage::JavaScript => cfg!(feature = "lang-javascript"),
            CodeLanguage::TypeScript | CodeLanguage::Tsx => cfg!(feature = "lang-typescript"),
            CodeLanguage::Python => cfg!(feature = "lang-python"),
            CodeLanguage::Go => cfg!(feature = "lang-go"),
            CodeLanguage::Java => cfg!(feature = "lang-java"),
            CodeLanguage::C => cfg!(feature = "lang-c"),
            CodeLanguage::Cpp => cfg!(feature = "lang-cpp"),
            CodeLanguage::CSharp => cfg!(feature = "lang-csharp"),
            CodeLanguage::Ruby => cfg!(feature = "lang-ruby"),
            CodeLanguage::Php => cfg!(feature = "lang-php"),
            CodeLanguage::Swift => cfg!(feature = "lang-swift"),
            CodeLanguage::Kotlin => cfg!(feature = "lang-kotlin"),
            CodeLanguage::Bash => true,
            CodeLanguage::Yaml => cfg!(feature = "lang-yaml"),
            CodeLanguage::Toml => cfg!(feature = "lang-toml"),
            CodeLanguage::Html => cfg!(feature = "lang-html"),
            CodeLanguage::Css => cfg!(feature = "lang-css"),
            CodeLanguage::Lua => cfg!(feature = "lang-lua"),
        }
    }

    /// 拡張子 (大文字小文字を問わない、ドットなし) から言語を決める。このビルドで読めない言語も返す。
    pub fn from_extension(ext: &str) -> Option<Self> {
        let ext = ext.to_ascii_lowercase();
        Self::ALL
            .into_iter()
            .find(|lang| lang.extensions().contains(&ext.as_str()))
    }

    /// このビルドで読める言語の拡張子 (小文字、ドットなし)。
    pub fn available_extensions() -> impl Iterator<Item = &'static str> {
        Self::ALL
            .into_iter()
            .filter(|lang| lang.is_available())
            .flat_map(|lang| lang.extensions().iter().copied())
    }
}

/// コードのファイルから、コメントの本文のブロックと抑制のコメントを取り出す。
///
/// ブロックの解析用テキストはコメントの記号を外した本文で、[`Block::to_source`] で原文
/// (`source`) のバイト位置に戻る。このビルドで読めない言語は何も返さない。
pub fn parse(_source: &str, _language: CodeLanguage) -> (Vec<Block>, Vec<Directive>) {
    (Vec::new(), Vec::new())
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn bash_is_always_available() {
        assert!(CodeLanguage::Bash.is_available());
        assert!(CodeLanguage::available_extensions().any(|e| e == "sh"));
    }
}
