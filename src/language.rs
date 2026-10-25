//! コードの言語と拡張子の表。文法の依存は持たない。

/// コメントを読める言語。
///
/// 文法による検査は feature `code` で有効にする。
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
