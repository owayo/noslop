//! 文書のジャンル。
//!
//! ジャンルによって、人間の書き手が正当に使う慣習が違う (技術文書は見出しと箇条書きに
//! 寄る、ビジネス文書は太字・定型見出し・段階表現を使う、エッセイは長い一文や体言止めを
//! 呼吸として使う)。ルールはジャンルに応じて閾値を変え、慣習と衝突するものは既定で止める。

use std::fmt;
use std::str::FromStr;

use serde::Deserialize;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Deserialize)]
#[serde(try_from = "String")]
pub enum Genre {
    /// 指定なし。どのジャンルにも偏らない保守的な閾値。
    #[default]
    General,
    /// 技術文書・技術ブログ。
    Tech,
    /// ビジネス文書 (報告書・提案書・議事録)。
    Business,
    /// エッセイ・ブログ。
    Essay,
}

impl Genre {
    pub const ALL: [Genre; 4] = [Genre::General, Genre::Tech, Genre::Business, Genre::Essay];

    pub fn as_str(self) -> &'static str {
        match self {
            Genre::General => "general",
            Genre::Tech => "tech",
            Genre::Business => "business",
            Genre::Essay => "essay",
        }
    }

    /// 別名 (`blog` → essay、`minutes` → business、`default` → general) も受け付ける。
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "general" | "default" | "none" => Some(Genre::General),
            "tech" | "technical" => Some(Genre::Tech),
            "business" | "minutes" | "report" => Some(Genre::Business),
            "essay" | "blog" => Some(Genre::Essay),
            _ => None,
        }
    }
}

impl fmt::Display for Genre {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Genre {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Genre::parse(s).ok_or_else(|| {
            format!(
                "未知のジャンルです: {s} (general / tech / business / essay、別名 blog / minutes)"
            )
        })
    }
}

impl TryFrom<String> for Genre {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_aliases() {
        assert_eq!(Genre::parse("blog"), Some(Genre::Essay));
        assert_eq!(Genre::parse("Minutes"), Some(Genre::Business));
        assert_eq!(Genre::parse("tech"), Some(Genre::Tech));
        assert_eq!(Genre::parse("x"), None);
        assert!("x".parse::<Genre>().is_err());
    }
}
