//! AI エージェント (Claude Code・Codex CLI) 向けのスキルを入れる (`noslop skill-install`)。
//!
//! スキルの本文 (`skills/SKILL.md`) はバイナリに埋め込み、エージェントのスキルの置き場へ
//! `noslop/SKILL.md` として書き出す。すでにあれば上書きする (noslop の版に合わせて更新するため)。

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// スキルの本文。
pub const SKILL: &str = include_str!("../skills/SKILL.md");

/// スキルを入れる先の AI エージェント。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// Claude Code (`~/.claude/skills/`)
    Claude,
    /// Codex CLI (`~/.codex/skills/`)
    Codex,
}

impl Target {
    /// ホームディレクトリの下の、エージェントのスキルの置き場。
    pub fn skills_dir(self, home: &Path) -> PathBuf {
        let config = match self {
            Self::Claude => ".claude",
            Self::Codex => ".codex",
        };
        home.join(config).join("skills")
    }

    /// 利用者に見せる名前。
    pub fn name(self) -> &'static str {
        match self {
            Self::Claude => "Claude Code",
            Self::Codex => "Codex CLI",
        }
    }
}

/// スキルの置き場 (`skills_dir`) の下に `noslop/SKILL.md` を書き、そのパスを返す。
pub fn install(skills_dir: &Path) -> io::Result<PathBuf> {
    let dir = skills_dir.join("noslop");
    fs::create_dir_all(&dir)?;
    let path = dir.join("SKILL.md");
    fs::write(&path, SKILL)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skills_dir_depends_on_the_agent() {
        let home = Path::new("home");
        assert_eq!(
            Target::Claude.skills_dir(home),
            home.join(".claude").join("skills")
        );
        assert_eq!(
            Target::Codex.skills_dir(home),
            home.join(".codex").join("skills")
        );
    }

    #[test]
    fn install_writes_and_overwrites_the_skill() {
        let dir = tempfile::tempdir().unwrap();
        let skills = Target::Codex.skills_dir(dir.path());
        let path = install(&skills).unwrap();
        assert_eq!(path, skills.join("noslop").join("SKILL.md"));
        assert_eq!(fs::read_to_string(&path).unwrap(), SKILL);

        // 古い版のスキルは上書きする
        fs::write(&path, "古い版").unwrap();
        install(&skills).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), SKILL);
    }

    #[test]
    fn skill_has_frontmatter_and_names_the_commands() {
        assert!(SKILL.starts_with("---\nname: noslop\n"), "{SKILL}");
        assert!(SKILL.contains("description:"));
        for command in [
            "noslop check --report brief --format toon",
            "noslop diff",
            "noslop explain",
        ] {
            assert!(SKILL.contains(command), "{command}");
        }
        // スキルの本文では引数の置換 ($ の直後の数字や ARGUMENTS) が起きるので書かない
        assert!(!SKILL.contains("$ARGUMENTS"));
        assert!(
            !SKILL
                .as_bytes()
                .windows(2)
                .any(|w| w[0] == b'$' && w[1].is_ascii_digit())
        );
    }
}
