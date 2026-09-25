//! 設定ファイルの読み込み。
//!
//! 設定は 2 つの層に分ける。
//!
//! - ユーザーの設定 (`~/.config/noslop/config.toml`、[`user_config_path`]): 手元の環境や好みの設定
//!   (辞書の場所など)。どの OS でもホームディレクトリの `.config/noslop` に置く (作者の別のツールと
//!   そろえる。`XDG_CONFIG_HOME` は見ない)。自動では作らない
//! - プロジェクトの設定 (`noslop.toml`): `--config` の指定がなければ、カレントディレクトリから親へ
//!   向かって `noslop.toml` → `.noslop.toml` の順に探し、最初に見つかった 1 つだけを使う (入力ファイル
//!   ごとに探すと、同じファイルでも実行する場所によって結果が変わってしまうため)
//!
//! 2 つは既定値 < ユーザー < プロジェクト < CLI の順に、項目ごとに重ねる ([`ConfigLayers`])。重ね方の
//! 細部 (パスの基準・除外パターン・ルールの別名・独自ルール) は `cli.rs` と `engine.rs` が受け持つ。

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::diagnostic::{Lane, Severity};
use crate::genre::Genre;
use crate::morph::MorphologyMode;
use crate::segment::LineBreakMode;

/// 探索する設定ファイルの名前 (優先順)。
pub const CONFIG_FILE_NAMES: [&str; 2] = ["noslop.toml", ".noslop.toml"];

/// ユーザーの設定ファイルのパス (`<home>/.config/noslop/config.toml`)。
pub fn user_config_path(home: &Path) -> PathBuf {
    home.join(".config").join("noslop").join("config.toml")
}

/// 設定の誤り。終了コード 2 で報告する。
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("設定ファイルを読めません: {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("設定ファイルの書式が正しくありません: {path}: {message}")]
    Parse { path: PathBuf, message: String },
    #[error("{0}")]
    Invalid(String),
}

/// `--fail-on` / `fail_on`: この重大度以上の指摘があれば終了コード 1 にする。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum FailOn {
    /// 指摘があっても終了コードを変えない (既定)。
    #[default]
    Never,
    At(Severity),
}

impl FailOn {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "never" | "none" => Some(FailOn::Never),
            other => Severity::parse(other).map(FailOn::At),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            FailOn::Never => "never",
            FailOn::At(s) => s.as_str(),
        }
    }

    /// この重大度の指摘で失敗にするか。
    pub fn trips(self, severity: Severity) -> bool {
        match self {
            FailOn::Never => false,
            FailOn::At(min) => severity >= min,
        }
    }
}

impl fmt::Display for FailOn {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl TryFrom<String> for FailOn {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        FailOn::parse(&value).ok_or_else(|| {
            format!(
                "fail_on には never / info / warning / error のどれかを指定してください: {value}"
            )
        })
    }
}

/// `[files]` セクション。
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FilesSection {
    /// 検査する拡張子 (ドットなし)。
    pub extensions: Option<Vec<String>>,
    /// 除外するパス (.gitignore と同じ書式。設定ファイルのディレクトリ基準)。
    pub exclude: Option<Vec<String>>,
}

/// `[code]` セクション。コードのファイルのコメントを検査する。
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodeSection {
    /// コメントを検査するコードの拡張子 (ドットなし)。ディレクトリをたどるときとフックで集める。
    /// 直接指定したファイルは、ここに書かなくても拡張子から言語が分かればコメントを検査する。
    pub extensions: Option<Vec<String>>,
}

/// `[scope]` セクション。語句ルールの対象を段落以外にも広げる。
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScopeSection {
    pub lists: Option<bool>,
    pub tables: Option<bool>,
    pub blockquotes: Option<bool>,
}

/// `[morphology]` セクション。形態素解析の辞書の使い方。
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MorphologySection {
    /// `auto` (見つかれば使う、既定)・`required` (必ず使う)・`off` (使わない)。
    pub mode: Option<MorphologyMode>,
    /// 辞書のファイル (hasami の `.hsd`)。相対パスは設定ファイルのディレクトリが基準で、
    /// `~/` はホームディレクトリ。なければ hasami の既定の場所を探す。
    pub dictionary: Option<PathBuf>,
}

/// `[rules.<ID か名前>]` テーブル。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RuleTable {
    pub enabled: Option<bool>,
    pub severity: Option<Severity>,
    /// `enabled` / `severity` 以外のキー (ルールの `configure` に渡す)。
    pub options: toml::Table,
}

/// `[rules]` セクション。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RulesSection {
    pub enable: Vec<String>,
    pub disable: Vec<String>,
    /// キーは設定ファイルに書かれたままのルール ID か名前。
    pub tables: BTreeMap<String, RuleTable>,
}

impl<'de> Deserialize<'de> for RulesSection {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let table = toml::Table::deserialize(deserializer)?;
        RulesSection::from_table(table).map_err(serde::de::Error::custom)
    }
}

impl RulesSection {
    fn from_table(table: toml::Table) -> Result<Self, String> {
        let mut section = RulesSection::default();
        for (key, value) in table {
            match key.as_str() {
                "enable" | "disable" => {
                    let list = string_list(&key, &value)?;
                    if key == "enable" {
                        section.enable = list;
                    } else {
                        section.disable = list;
                    }
                }
                _ => {
                    let toml::Value::Table(t) = value else {
                        return Err(format!(
                            "[rules] の `{key}` はテーブル ([rules.{key}]) で書いてください (使えるキーは enable / disable とルールごとのテーブル)"
                        ));
                    };
                    section.tables.insert(key.clone(), rule_table(&key, t)?);
                }
            }
        }
        Ok(section)
    }
}

fn string_list(key: &str, value: &toml::Value) -> Result<Vec<String>, String> {
    let toml::Value::Array(items) = value else {
        return Err(format!("[rules] の `{key}` は文字列の配列で書いてください"));
    };
    items
        .iter()
        .map(|v| {
            v.as_str()
                .map(str::to_string)
                .ok_or_else(|| format!("[rules] の `{key}` には文字列だけを並べてください"))
        })
        .collect()
}

fn rule_table(rule: &str, table: toml::Table) -> Result<RuleTable, String> {
    let mut out = RuleTable::default();
    for (key, value) in table {
        match key.as_str() {
            "enabled" => {
                out.enabled = Some(value.as_bool().ok_or_else(|| {
                    format!("[rules.{rule}] の enabled には true か false を指定してください")
                })?);
            }
            "severity" => {
                let s = value.as_str().ok_or_else(|| {
                    format!("[rules.{rule}] の severity には文字列を指定してください")
                })?;
                out.severity = Some(Severity::parse(s).ok_or_else(|| {
                    format!(
                        "[rules.{rule}] の severity が正しくありません: {s} (info / warning / error)"
                    )
                })?);
            }
            _ => {
                out.options.insert(key, value);
            }
        }
    }
    Ok(out)
}

/// `[[custom]]`: チーム独自の禁止語などを追加するルール。
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CustomRuleConfig {
    /// ルール ID (組み込みルールと重ならないもの)。
    pub id: String,
    /// ルール名。省略すると ID と同じ。
    pub name: Option<String>,
    /// 探す文字列 (`regex = true` なら正規表現)。
    pub pattern: String,
    #[serde(default)]
    pub regex: bool,
    /// 見つかったときのメッセージ。
    pub message: String,
    pub hint: Option<String>,
    /// info / warning / error (既定 warning)。
    pub severity: Option<String>,
    /// slop / readability / custom (既定 custom)。
    pub lane: Option<String>,
}

impl CustomRuleConfig {
    pub fn severity(&self) -> Result<Severity, ConfigError> {
        match &self.severity {
            None => Ok(Severity::Warning),
            Some(s) => Severity::parse(s).ok_or_else(|| {
                ConfigError::Invalid(format!(
                    "独自ルール {} の severity が正しくありません: {s} (info / warning / error)",
                    self.id
                ))
            }),
        }
    }

    pub fn lane(&self) -> Result<Lane, ConfigError> {
        match self.lane.as_deref().map(|s| s.trim().to_ascii_lowercase()) {
            None => Ok(Lane::Custom),
            Some(s) if s == "custom" => Ok(Lane::Custom),
            Some(s) if s == "slop" => Ok(Lane::Slop),
            Some(s) if s == "readability" => Ok(Lane::Readability),
            Some(s) => Err(ConfigError::Invalid(format!(
                "独自ルール {} の lane が正しくありません: {s} (slop / readability / custom)",
                self.id
            ))),
        }
    }
}

/// 設定ファイルの内容。
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigFile {
    pub genre: Option<Genre>,
    pub experimental: Option<bool>,
    #[serde(default, deserialize_with = "deserialize_fail_on")]
    pub fail_on: Option<FailOn>,
    pub line_breaks: Option<LineBreakMode>,
    #[serde(default)]
    pub files: FilesSection,
    #[serde(default)]
    pub code: CodeSection,
    #[serde(default)]
    pub scope: ScopeSection,
    #[serde(default)]
    pub rules: RulesSection,
    #[serde(default)]
    pub custom: Vec<CustomRuleConfig>,
    #[serde(default)]
    pub morphology: MorphologySection,
}

fn deserialize_fail_on<'de, D>(deserializer: D) -> Result<Option<FailOn>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<String>::deserialize(deserializer)?;
    value
        .map(FailOn::try_from)
        .transpose()
        .map_err(serde::de::Error::custom)
}

/// 読み込んだ設定ファイル。
#[derive(Debug, Clone)]
pub struct LoadedConfig {
    pub path: PathBuf,
    /// 設定ファイルのあるディレクトリ (除外パスの基準)。
    pub base_dir: PathBuf,
    pub file: ConfigFile,
}

/// この実行で使う設定の層。どちらもなければ既定値だけで動く (`--no-config` もこれ)。
#[derive(Debug, Clone, Default)]
pub struct ConfigLayers {
    /// ユーザーの設定 (`~/.config/noslop/config.toml`)。
    pub user: Option<LoadedConfig>,
    /// プロジェクトの設定 (探した `noslop.toml` か `--config`)。
    pub project: Option<LoadedConfig>,
}

impl ConfigLayers {
    /// 項目の値。プロジェクトの設定に書いてあればそれ、なければユーザーの設定。
    pub fn pick<T>(&self, get: impl Fn(&ConfigFile) -> Option<T>) -> Option<T> {
        self.project
            .as_ref()
            .and_then(|c| get(&c.file))
            .or_else(|| self.user.as_ref().and_then(|c| get(&c.file)))
    }

    /// 項目の値と、それを書いた設定ファイル (プロジェクト、ユーザーの順に探す)。
    pub fn pick_with_source<T>(
        &self,
        get: impl Fn(&ConfigFile) -> Option<T>,
    ) -> Option<(T, &LoadedConfig)> {
        [self.project.as_ref(), self.user.as_ref()]
            .into_iter()
            .flatten()
            .find_map(|c| get(&c.file).map(|v| (v, c)))
    }
}

impl ConfigFile {
    /// TOML の文字列を解釈する。
    pub fn parse(text: &str, path: &Path) -> Result<Self, ConfigError> {
        toml::from_str(text).map_err(|e| ConfigError::Parse {
            path: path.to_path_buf(),
            message: e.message().to_string(),
        })
    }
}

/// 設定ファイルを読み込む。
pub fn load(path: &Path) -> Result<LoadedConfig, ConfigError> {
    let text = std::fs::read_to_string(path).map_err(|source| ConfigError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let file = ConfigFile::parse(&text, path)?;
    let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let base_dir = absolute
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    Ok(LoadedConfig {
        path: path.to_path_buf(),
        base_dir,
        file,
    })
}

/// `start` から親へ向かって設定ファイルを探す。
pub fn discover(start: &Path) -> Option<PathBuf> {
    let start = std::path::absolute(start).unwrap_or_else(|_| start.to_path_buf());
    for dir in start.ancestors() {
        for name in CONFIG_FILE_NAMES {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

/// `noslop init` が書き出すひな形。
pub const TEMPLATE: &str = r#"# noslop の設定ファイル
#
# CLI で指定した値はこのファイルより優先されます。
# どの項目も、書かなければユーザーの設定 (~/.config/noslop/config.toml) の値、
# それもなければ既定値が使われます。

# 文書のジャンル: general / tech / business / essay (別名 blog / minutes)
# ジャンルごとに閾値が変わり、慣習と衝突するルールは既定で止まります。
# genre = "general"

# 実験的な (未校正の) ルールと語句も動かすか
# experimental = false

# この重大度以上の指摘があれば終了コード 1 にする: never / info / warning / error
# 検出は「疑いの提示」なので、既定の never では終了コードを変えません。
# fail_on = "never"

# 段落内の改行の扱い: space (文の区切りにしない) / sentence (改行で文を区切る)
# line_breaks = "space"

[files]
# 検査する拡張子
# extensions = ["md", "markdown", "txt"]
# 除外するパス (.gitignore と同じ書式。このファイルのあるディレクトリ基準)
# exclude = ["CHANGELOG.md", "vendor/"]

[code]
# コメントを検査するコードの拡張子 (既定は空で、ディレクトリをたどるときにコードは集めない)。
# noslop check に直接渡したコードのファイルは、ここに書かなくてもコメントを検査します。
# コメントは短い断片なので、1 文ずつ判定するルールだけを当てます
# extensions = ["rs", "ts", "tsx", "py", "go", "sh"]

[scope]
# 語句ルールをリスト・表・引用にも当てるか (既定は地の文の段落だけ)
# lists = false
# tables = false
# blockquotes = false

[morphology]
# 形態素解析の辞書 (hasami の .hsd) の使い方。P15・P16 を品詞で判定する
#   auto (既定。辞書が見つかれば使う) / required (必ず使う) / off (使わない)
# mode = "auto"
# 使う辞書
#   auto (既定): HASAMI_DICT → hasami の share ディレクトリ (既定は ~/.local/share/hasami) の
#                辞書を推奨順 (ipadic-neologd-sudachi → ipadic-neologd → ipadic) → 同梱の IPAdic
#   bundled: 同梱の IPAdic (P15・P16 を校正した辞書。手元の辞書で結果を変えたくないとき)
#   share:<名前>: share ディレクトリの辞書 (noslop dict download <名前> で取得できる)
#   ファイルのパス (相対パスはこのファイルのあるディレクトリ基準)
# dictionary = "auto"

[rules]
# 有効にするルール (実験的なルールも個別に有効にできる)
# enable = ["P04"]
# 無効にするルール
# disable = ["R03"]

# ルールごとの設定 (ID か名前で書く)
# [rules.R01]
# severity = "info"

# チーム独自の禁止語
# [[custom]]
# id = "X01"
# name = "BANNED_TERM"
# pattern = "ユーザー様"
# message = "「ユーザー様」ではなく「利用者」と書きます"
# hint = "用語集の表記に合わせてください"
# severity = "warning"
"#;

/// `noslop init --user` が書き出す、ユーザーの設定 (`~/.config/noslop/config.toml`) のひな形。
/// 書き方はプロジェクトの設定と同じだが、手元のマシンでだけ使う設定に絞って案内する。
pub const USER_TEMPLATE: &str = r#"# noslop のユーザーの設定 (~/.config/noslop/config.toml)
#
# このマシンでは、どのディレクトリで実行してもこのファイルを読みます。
# プロジェクトの設定 (noslop.toml) に書いた項目はそちらが優先され、CLI で指定した値はさらに優先されます。
# 書き方はプロジェクトの設定と同じです。どの項目も、書かなければ既定値が使われます。
# 手元のマシンでだけ使う設定 (辞書の選び方や、手元で止めたいルールなど) を書いてください。

# 文書のジャンル: general / tech / business / essay (別名 blog / minutes)
# genre = "general"

# 実験的な (未校正の) ルールと語句も動かすか
# experimental = false

[morphology]
# 形態素解析の辞書 (P15・P16 を品詞で判定する)
#   auto (既定): HASAMI_DICT → hasami の share ディレクトリ (既定は ~/.local/share/hasami) の辞書を
#                推奨順 (ipadic-neologd-sudachi → ipadic-neologd → ipadic) → 同梱の IPAdic
#   bundled: 同梱の IPAdic (P15・P16 を校正した辞書)
#   share:<名前>: noslop dict download <名前> で取得した辞書
#   ファイルのパス (相対パスはこのファイルのあるディレクトリ基準、~/ はホームディレクトリ)
# dictionary = "auto"

[rules]
# 手元で止めるルール・動かすルール (プロジェクトの設定で触れたルールは、そちらに従います)
# disable = ["R03"]
# enable = ["P04"]

[files]
# 除外するパス (.gitignore と同じ書式)。ユーザーの設定では、noslop check に渡したディレクトリが基準です。
# プロジェクトの設定に exclude があれば、こちらは使いません
# exclude = ["drafts/"]

[code]
# コメントを検査するコードの拡張子 (フックと、ディレクトリをたどるときに集める。既定は空)。
# プロジェクトの設定に extensions があれば、こちらは使いません
# extensions = ["rs", "ts", "tsx", "py", "go", "sh"]

# 手元で使う独自ルール (プロジェクトの設定に同じ ID があれば、そちらで置き換わります)
# [[custom]]
# id = "X01"
# name = "BANNED_TERM"
# pattern = "ユーザー様"
# message = "「ユーザー様」ではなく「利用者」と書きます"
# severity = "warning"
"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Result<ConfigFile, ConfigError> {
        ConfigFile::parse(text, Path::new("noslop.toml"))
    }

    #[test]
    fn parses_full_config() {
        let cfg = parse(
            r#"
genre = "blog"
experimental = true
fail_on = "warning"
line_breaks = "sentence"

[files]
extensions = ["md"]
exclude = ["vendor/"]

[scope]
lists = true

[rules]
enable = ["P04"]
disable = ["R03"]

[rules.R01]
severity = "info"
threshold = -0.4

[[custom]]
id = "X01"
pattern = "ユーザー様"
message = "表記を統一します"
"#,
        )
        .unwrap();
        assert_eq!(cfg.genre, Some(Genre::Essay));
        assert_eq!(cfg.experimental, Some(true));
        assert_eq!(cfg.fail_on, Some(FailOn::At(Severity::Warning)));
        assert_eq!(cfg.line_breaks, Some(LineBreakMode::Sentence));
        assert_eq!(
            cfg.files.extensions.as_deref(),
            Some(&["md".to_string()][..])
        );
        assert_eq!(cfg.scope.lists, Some(true));
        assert_eq!(cfg.rules.enable, vec!["P04"]);
        assert_eq!(cfg.rules.disable, vec!["R03"]);
        let r01 = &cfg.rules.tables["R01"];
        assert_eq!(r01.severity, Some(Severity::Info));
        assert_eq!(
            r01.options.get("threshold").and_then(|v| v.as_float()),
            Some(-0.4)
        );
        assert_eq!(cfg.custom.len(), 1);
        assert_eq!(cfg.custom[0].severity().unwrap(), Severity::Warning);
        assert_eq!(cfg.custom[0].lane().unwrap(), Lane::Custom);
    }

    #[test]
    fn rejects_unknown_keys() {
        assert!(parse("unknown = 1").is_err());
        assert!(parse("[files]\nextension = [\"md\"]").is_err());
        assert!(
            parse("[[custom]]\nid = \"X\"\npattern = \"a\"\nmessage = \"m\"\nfoo = 1").is_err()
        );
    }

    #[test]
    fn rejects_bad_values() {
        let err = parse("[rules.R01]\nseverity = \"loud\"").unwrap_err();
        assert!(err.to_string().contains("severity"), "{err}");
        assert!(parse("fail_on = \"sometimes\"").is_err());
        assert!(parse("genre = \"poetry\"").is_err());
        assert!(parse("[rules]\nR01 = 1").is_err());
        assert!(parse("[rules]\nenable = [1]").is_err());
        let cfg =
            parse("[[custom]]\nid = \"X\"\npattern = \"a\"\nmessage = \"m\"\nlane = \"loud\"")
                .unwrap();
        assert!(cfg.custom[0].lane().is_err());
    }

    #[test]
    fn template_is_valid() {
        // どちらのひな形も、そのまま読めて、既定値から何も変えない
        for template in [TEMPLATE, USER_TEMPLATE] {
            let cfg = parse(template).unwrap();
            assert!(cfg.genre.is_none());
            assert!(cfg.experimental.is_none());
            assert!(cfg.custom.is_empty());
            assert!(cfg.rules.enable.is_empty() && cfg.rules.disable.is_empty());
            assert!(cfg.morphology.dictionary.is_none());
            assert!(cfg.files.exclude.is_none());
        }
    }

    #[test]
    fn fail_on_trips_at_or_above_threshold() {
        assert!(!FailOn::Never.trips(Severity::Error));
        assert!(FailOn::At(Severity::Warning).trips(Severity::Error));
        assert!(FailOn::At(Severity::Warning).trips(Severity::Warning));
        assert!(!FailOn::At(Severity::Warning).trips(Severity::Info));
        assert_eq!(FailOn::parse("none"), Some(FailOn::Never));
    }

    #[test]
    fn discovers_nearest_config_upwards() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("a/b");
        std::fs::create_dir_all(&nested).unwrap();
        assert!(discover(&nested).is_none() || !discover(&nested).unwrap().starts_with(dir.path()));
        std::fs::write(dir.path().join(".noslop.toml"), "").unwrap();
        assert_eq!(
            discover(&nested).unwrap(),
            std::path::absolute(dir.path().join(".noslop.toml")).unwrap()
        );
        std::fs::write(dir.path().join("a/noslop.toml"), "").unwrap();
        assert_eq!(
            discover(&nested).unwrap(),
            std::path::absolute(dir.path().join("a/noslop.toml")).unwrap()
        );
    }
}
