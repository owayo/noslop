//! hasami の配布辞書の取得 (`noslop dict download` / `noslop dict list`)。
//!
//! hasami のリリースに添付されたビルド済みの辞書 (`.hsd`) を、hasami の share
//! ディレクトリ (hasami が辞書を探す場所。既定は `~/.local/share/hasami`) に取得する。
//!
//! - 取得元と照合の値は、hasami のリリースの目録 (`dictionaries.json`) を写した `dict/catalog.json` から
//!   build.rs が作る ([`HASAMI_TAG`]・[`DEFAULT_SOURCE`]・[`DICTIONARIES`])。取得した中身は、目録の大きさと
//!   SHA-256 で確かめる。目録は Release のワークフロー (`make dict-catalog`) が hasami の新しいリリースに
//!   合わせて更新する。依存の hasami (`Cargo.toml` のタグ) と同梱の辞書 (`dict/ipadic.hsd`) は目録とは
//!   別で、判定の結果を左右するので人が上げる。目録の辞書の形式が依存の hasami で読めることはテストで確かめる
//! - 取得そのもの (HTTP・zstd の展開・大きさと SHA-256 の照合・辞書として読めることの確認・一時ファイル
//!   からの置き換え) は hasami の `download` (feature `download`) に任せる。noslop は目録の値と取得元を
//!   固定して渡し、誤りを日本語に言い換える ([`download`])。既定では zstd で圧縮した版 (`<名前>.hsd.zst`。
//!   3 分の 1 ほどの大きさ) を取って展開し、圧縮版と展開後の両方を確かめる。圧縮版を置いていない取得元
//!   (ミラー) では、展開前の辞書を取る指定 (`--uncompressed`) を使う
//! - 取得した辞書は、辞書を指定しないとき (`auto`) に使う。share ディレクトリの配布辞書のうち、依存の
//!   hasami の推奨順 ([`preferred_in`]) で最初に見つかったものを、同梱の辞書より先に選ぶ
//!   ([`crate::morph::resolve`])。選ぶ順は目録ではなく依存の hasami で決まるので、目録の自動更新では
//!   変わらない
//! - 決まった辞書を使うときは `--dict share:<名前>` か、設定の `[morphology] dictionary = "share:<名前>"`
//!   で指定する ([`resolve_share`])
//! - 取得は保存先と同じディレクトリの一時ファイルに書き、確かめてから rename で置く。途中で失敗しても、
//!   置き場所にある既存のファイルは消さず、壊さない (hasami の `download` の動き)

use std::io;
use std::path::{Path, PathBuf};

use hasami::download::{
    CompressedFile, DistributedDict, DownloadError as HasamiError, DownloadOptions,
    Outcome as HasamiOutcome, Verification,
};

/// 辞書の指定 (`--dict`・設定の `dictionary`) で、share ディレクトリの辞書を指す接頭辞。
pub const SHARE_PREFIX: &str = "share:";

/// hasami が配布するビルド済みの辞書。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Distributed {
    /// 名前 (ファイル名は `<名前>.hsd`)。
    pub name: &'static str,
    /// 目録の中身の説明 (英語)。表示には [`Distributed::description`] を使う。
    pub summary: &'static str,
    /// 大きさ (バイト)。
    pub size: u64,
    /// SHA-256 (小文字の 16 進)。
    pub sha256: &'static str,
    /// zstd で圧縮した同じ辞書 (目録の `compressed`)。
    pub compressed: Option<Compressed>,
}

/// zstd で圧縮した配布辞書 (`<名前>.hsd.zst`)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Compressed {
    /// ファイル名。
    pub file: &'static str,
    /// 大きさ (バイト)。
    pub size: u64,
    /// SHA-256 (小文字の 16 進)。
    pub sha256: &'static str,
}

impl Distributed {
    /// ファイル名 (`<名前>.hsd`)。
    pub fn file_name(&self) -> String {
        format!("{}.hsd", self.name)
    }

    /// 取得で受け取る大きさ (圧縮版を取るなら、その大きさ)。
    pub fn transfer_size(&self, compressed: bool) -> u64 {
        match self.compressed {
            Some(c) if compressed => c.size,
            _ => self.size,
        }
    }

    /// hasami の取得の API に渡す形。
    fn to_hasami(self) -> DistributedDict {
        let mut dict = DistributedDict::new(self.name, self.size, self.sha256);
        dict.summary = self.summary.to_string();
        dict.compressed = self.compressed.map(|c| CompressedFile {
            file: c.file.to_string(),
            size: c.size,
            sha256: c.sha256.to_string(),
        });
        dict
    }

    /// 一覧やヘルプに出す説明。知っている辞書は日本語で、知らない辞書は目録の説明のまま。
    pub fn description(&self) -> &'static str {
        match self.name {
            "ipadic" => "IPAdic",
            "ipadic-neologd" => "IPAdic + NEologd",
            "ipadic-neologd-sudachi" => {
                "IPAdic + NEologd + SudachiDict (hasami の推奨、最大の語彙)"
            }
            _ => self.summary,
        }
    }
}

// HASAMI_TAG・DEFAULT_SOURCE・CATALOG_FORMAT_VERSION・RECOMMENDED・DICTIONARIES は、
// dict/catalog.json から build.rs が作る。取得元は HASAMI_TAG のリリースの添付ファイルで、hasami は
// 辞書をリポジトリ (Git LFS) から外して既存のタグからも消したので、リポジトリの中のパスは指さない。
// ミラーがあれば `noslop dict download --source` で切り替えられ、どの取得元でも大きさと SHA-256 を確かめる
include!(concat!(env!("OUT_DIR"), "/catalog.rs"));

/// 名前で配布辞書を探す。
pub fn find(name: &str) -> Option<&'static Distributed> {
    let all: &'static [Distributed] = &DICTIONARIES;
    all.iter().find(|d| d.name == name)
}

// ---------------------------------------------------------------------------
// share ディレクトリ
// ---------------------------------------------------------------------------

/// 配布辞書の置き場 (hasami の share ディレクトリ)。
///
/// hasami が辞書を探す場所 (`hasami::analyzer::default_dict_path`) とずれないよう、hasami の
/// [`hasami::analyzer::data_dir`] をそのまま使う。`HASAMI_DATA_DIR`、`<XDG_DATA_HOME>/hasami`、
/// Windows では `<LOCALAPPDATA>/hasami`、`<HOME>/.local/share/hasami` の順で、空の値は設定されて
/// いないとみなす。どれもなければ `None`。
pub fn share_dir() -> Option<PathBuf> {
    hasami::analyzer::data_dir()
}

/// 辞書の指定が `share:<名前>` なら、その名前。
pub fn share_name(spec: &Path) -> Option<&str> {
    spec.to_str()?.strip_prefix(SHARE_PREFIX)
}

/// `share:<名前>` の辞書を解決できない理由。
#[derive(Debug, thiserror::Error)]
pub enum ShareError {
    #[error(
        "share ディレクトリが分かりません (HASAMI_DATA_DIR・XDG_DATA_HOME・HOME のどれも設定されていません。Windows では LOCALAPPDATA も見ます)。辞書のファイルのパスを指定してください"
    )]
    NoShareDir,
    #[error("share: の後に辞書の名前がありません (例: share:{RECOMMENDED})")]
    EmptyName,
    #[error(
        "share:{0} は辞書の名前として使えません (/・\\・: と、「.」「..」は書けません。share ディレクトリの外の辞書はファイルのパスで指定してください)"
    )]
    InvalidName(String),
    #[error("share:{name} の辞書がありません ({path}){hint}", path = .path.display(), hint = download_hint(.name))]
    Missing { name: String, path: PathBuf },
}

/// 辞書がないときの案内 (名前が配布辞書のどれかのときだけ)。
fn download_hint(name: &str) -> String {
    let name = name.strip_suffix(".hsd").unwrap_or(name);
    match find(name) {
        Some(dict) => format!("。`noslop dict download {}` で取得できます", dict.name),
        None => String::new(),
    }
}

/// `share:<名前>` の名前を、share ディレクトリの中のファイル名 (`<名前>.hsd`) に直す。名前が `.hsd` で
/// 終わっていれば付けない。share ディレクトリの外を指せないよう、パスの区切りと `.`・`..` は拒む
/// (`:` も拒む。Windows では `C:x` がドライブの相対パスになり、share ディレクトリの外を指すため)。
fn share_file_name(name: &str) -> Result<String, ShareError> {
    if name.is_empty() {
        return Err(ShareError::EmptyName);
    }
    if matches!(name, "." | "..") || name.contains(['/', '\\', ':']) {
        return Err(ShareError::InvalidName(name.to_string()));
    }
    Ok(if name.ends_with(".hsd") {
        name.to_string()
    } else {
        format!("{name}.hsd")
    })
}

/// `share:<名前>` が指すファイル (`<dir>/<名前>.hsd`)。ファイルがあるかは確かめない。
pub fn share_path_in(dir: &Path, name: &str) -> Result<PathBuf, ShareError> {
    Ok(dir.join(share_file_name(name)?))
}

/// `share:<名前>` の名前を、share ディレクトリ `dir` ([`share_dir`]) にある辞書のファイルに直す。
/// 名前を先に確かめ、share ディレクトリが分からないときとファイルがないときはエラー。
pub fn resolve_share(dir: Option<&Path>, name: &str) -> Result<PathBuf, ShareError> {
    let file_name = share_file_name(name)?;
    let dir = dir.ok_or(ShareError::NoShareDir)?;
    existing(dir.join(file_name), name)
}

/// ディレクトリ `dir` の配布辞書のうち、辞書を指定しないとき (`auto`) に使うもの。依存の hasami の
/// 推奨順 (`hasami::analyzer::DISTRIBUTED_DICTS`) で最初に見つかったファイル。配布辞書でない `*.hsd` は
/// 選ばない (同梱の辞書より良いとは言えないため)。
pub fn preferred_in(dir: &Path) -> Option<PathBuf> {
    hasami::analyzer::DISTRIBUTED_DICTS
        .iter()
        .map(|name| dir.join(format!("{name}.hsd")))
        .find(|path| path.is_file())
}

/// hasami の推奨順を、人に見せる形で (`ipadic-neologd-sudachi → ipadic-neologd → ipadic`)。
pub fn preference_order() -> String {
    hasami::analyzer::DISTRIBUTED_DICTS.join(" → ")
}

fn existing(path: PathBuf, name: &str) -> Result<PathBuf, ShareError> {
    if path.is_file() {
        Ok(path)
    } else {
        Err(ShareError::Missing {
            name: name.to_string(),
            path,
        })
    }
}

// ---------------------------------------------------------------------------
// 検証と一覧
// ---------------------------------------------------------------------------

/// 置き場所のファイルの状態。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Check {
    /// ファイルがない。
    Missing,
    /// 大きさと SHA-256 が配布辞書と一致した。
    Verified,
    /// 中身が違う (hasami の別の版か、壊れている)。
    Differs,
}

/// 置き場所のファイルを、配布辞書の大きさと SHA-256 で確かめる。大きさが違えばハッシュを計算しない
/// (`hasami::download::verify`)。
pub fn check_file(path: &Path, dict: &Distributed) -> io::Result<Check> {
    Ok(match hasami::download::verify(path, &dict.to_hasami())? {
        Verification::Missing => Check::Missing,
        Verification::Verified => Check::Verified,
        Verification::Differs => Check::Differs,
    })
}

/// 配布辞書の一覧の 1 行 ([`list`])。
#[derive(Debug)]
pub struct Listed {
    pub dictionary: &'static Distributed,
    /// 置き場所 (`<dir>/<名前>.hsd`)。
    pub path: PathBuf,
    /// 置き場所のファイルの状態 (読めなければ `Err`)。
    pub check: io::Result<Check>,
}

/// 各配布辞書の、`dir` の中の置き場所とその状態。通信しない。
pub fn list(dir: &Path) -> Vec<Listed> {
    let all: &'static [Distributed] = &DICTIONARIES;
    all.iter()
        .map(|dictionary| {
            let path = dir.join(dictionary.file_name());
            let check = check_file(&path, dictionary);
            Listed {
                dictionary,
                path,
                check,
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// 取得
// ---------------------------------------------------------------------------

/// hasami が、圧縮版を展開したものの誤りの出所 (`<URL>`) の後ろに付ける印。
const DECOMPRESSED: &str = " (decompressed)";

/// 取得の失敗 (hasami の取得の誤りを、利用者向けの日本語に言い換えたもの)。
#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct DownloadError {
    message: String,
}

impl DownloadError {
    /// hasami の誤りを言い換える。`dict` は取ろうとした辞書。
    fn from_hasami(e: HasamiError, dict: &Distributed) -> Self {
        let name = dict.name;
        // 誤りの出所の「〜の」。圧縮版を展開したものの検査の誤りは、hasami が `<URL> (decompressed)` を
        // 出所にするので、「<URL> を展開した中身の」と書く
        let of = |from: &str| match from.strip_suffix(DECOMPRESSED) {
            Some(url) => format!("{url} を展開した中身の"),
            None => format!("{from} の"),
        };
        let decoded = |from: &str| from.ends_with(DECOMPRESSED);
        let message = match e {
            HasamiError::CreateDir { path, source } => {
                format!(
                    "保存先のディレクトリを作れません: {}: {source}",
                    path.display()
                )
            }
            HasamiError::Inspect { path, source } => {
                format!("{} を確かめられません: {source}", path.display())
            }
            HasamiError::TempFile { dir, source } => {
                format!("一時ファイルを作れません: {}: {source}", dir.display())
            }
            HasamiError::Request { url, source } => format!("{url} を取得できません: {source}"),
            HasamiError::Status { url, status } => {
                format!("{url} を取得できません (HTTP {status})")
            }
            HasamiError::ContentLength {
                url,
                expected,
                actual,
            } => {
                let what = if url.ends_with(".zst") {
                    format!("{name} の圧縮版")
                } else {
                    format!("{name} ")
                };
                format!(
                    "{url} の大きさが違います (Content-Length が {actual} バイト、hasami {HASAMI_TAG} の {what}は {expected} バイト)"
                )
            }
            HasamiError::Receive { from, source } if decoded(&from) => {
                format!("{}書き込みに失敗しました: {source}", of(&from))
            }
            HasamiError::Receive { from, source } => {
                format!("{}受信に失敗しました: {source}", of(&from))
            }
            HasamiError::Oversized { from, expected } => format!(
                "{}大きさが違います ({expected} バイトを超えました)",
                of(&from)
            ),
            HasamiError::Truncated {
                from,
                expected,
                received,
            } if decoded(&from) => format!(
                "{}大きさが足りません ({received} / {expected} バイト)",
                of(&from)
            ),
            HasamiError::Truncated {
                from,
                expected,
                received,
            } => format!(
                "{}受信が途中で切れました ({received} / {expected} バイト)",
                of(&from)
            ),
            HasamiError::Checksum {
                from,
                expected,
                actual,
            } => format!(
                "{} SHA-256 が違います (期待 {expected}、実際 {actual})",
                of(&from)
            ),
            HasamiError::Decompress { from, reason } => {
                format!("{from} を展開できません (zstd): {reason}")
            }
            HasamiError::Write { path, source } => {
                format!("一時ファイルに書き込めません: {}: {source}", path.display())
            }
            HasamiError::NotDictionary { from, reason } => {
                format!("取得したファイルを辞書として読めません ({from}): {reason}")
            }
            HasamiError::Persist { path, source } => {
                format!("{} に置けません: {source}", path.display())
            }
            // 目録の取得の誤りなど、ここでは起きないものと、hasami に後から足された誤り
            other => format!("{name} を取得できません: {other}"),
        };
        Self { message }
    }
}

/// 配布辞書を `dir` に取得する (置き場所は `<dir>/<名前>.hsd`)。
///
/// `source` は取得元の URL の接頭辞 (既定は [`DEFAULT_SOURCE`])。`compressed` が真で目録に圧縮版が
/// あれば `<source>/<名前>.hsd.zst` を取って展開し、なければ `<source>/<名前>.hsd` を取る。
/// 圧縮版が HTTP 404 のときも非圧縮版へ切り替える。通信・検証・展開の失敗では切り替えない。
/// 既存のファイルがあっても毎回取得し、検証に成功してから置き換える。失敗時は既存のファイルを保つ。
///
/// `progress(受信したバイト数, 受信する全体のバイト数)` は、通信を始める前に 1 度 (受信 0 で)、その後は
/// 受け取るたびに呼ぶ。圧縮版を取るときの全体は圧縮版の大きさ ([`Distributed::transfer_size`])。
/// 非圧縮版へ切り替えるときは、その大きさと受信 0 で呼び直す。
/// 通信しないときは呼ばない。
pub fn download(
    dict: &Distributed,
    dir: &Path,
    source: &str,
    compressed: bool,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<PathBuf, DownloadError> {
    let options = DownloadOptions {
        // 目録は依存の hasami より新しいことがあるので、取得元は目録の版 (DEFAULT_SOURCE) を渡す
        base_url: Some(source),
        compressed,
        force: true,
        progress: Some(progress),
    };
    match hasami::download::download(&dict.to_hasami(), dir, options) {
        Ok(HasamiOutcome::Present(path) | HasamiOutcome::Downloaded(path)) => Ok(path),
        Err(e) => Err(DownloadError::from_hasami(e, dict)),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use sha2::{Digest, Sha256};

    use super::*;

    // -----------------------------------------------------------------------
    // share ディレクトリ
    // -----------------------------------------------------------------------

    #[test]
    fn share_specs_name_a_file_in_the_share_directory() {
        assert_eq!(share_name(Path::new("share:ipadic")), Some("ipadic"));
        assert_eq!(share_name(Path::new("share:")), Some(""));
        assert_eq!(share_name(Path::new("dict/ipadic.hsd")), None);
        assert_eq!(share_name(Path::new("./share:ipadic")), None);

        let dir = Path::new("share");
        assert_eq!(
            share_path_in(dir, "ipadic").unwrap(),
            dir.join("ipadic.hsd")
        );
        // .hsd が付いていれば重ねない
        assert_eq!(
            share_path_in(dir, "ipadic.hsd").unwrap(),
            dir.join("ipadic.hsd")
        );
        assert!(matches!(share_path_in(dir, ""), Err(ShareError::EmptyName)));
        for name in ["../x", "a/b", "a\\b", ".", "..", "C:x"] {
            let err = share_path_in(dir, name).unwrap_err();
            assert!(matches!(err, ShareError::InvalidName(_)), "{name}: {err}");
            assert!(err.to_string().contains(name), "{err}");
        }
    }

    /// 名前は share ディレクトリより先に確かめる (share ディレクトリが分からなくても、名前の誤りを示す)。
    #[test]
    fn share_names_are_checked_before_the_share_directory() {
        assert!(matches!(
            resolve_share(None, "../x"),
            Err(ShareError::InvalidName(_))
        ));
        assert!(matches!(
            resolve_share(None, "ipadic"),
            Err(ShareError::NoShareDir)
        ));
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(
            resolve_share(Some(dir.path()), "ipadic"),
            Err(ShareError::Missing { .. })
        ));
        let path = dir.path().join("ipadic.hsd");
        fs::write(&path, b"x").unwrap();
        assert_eq!(resolve_share(Some(dir.path()), "ipadic").unwrap(), path);
    }

    /// auto が選ぶのは、配布辞書のうち hasami の推奨順で最初に見つかったもの。
    #[test]
    fn the_preferred_dictionary_follows_the_hasami_order() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(preferred_in(dir.path()), None);
        fs::write(dir.path().join("custom.hsd"), b"x").unwrap();
        assert_eq!(
            preferred_in(dir.path()),
            None,
            "配布辞書でないものは選ばない"
        );
        // 推奨順の低いものから置いていくと、置くたびに今置いたものが選ばれる
        for name in hasami::analyzer::DISTRIBUTED_DICTS.iter().rev() {
            let path = dir.path().join(format!("{name}.hsd"));
            fs::write(&path, b"x").unwrap();
            assert_eq!(preferred_in(dir.path()), Some(path));
        }
    }

    #[test]
    fn a_missing_share_dictionary_points_to_the_download_command() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ipadic.hsd");
        let err = existing(path.clone(), "ipadic").unwrap_err().to_string();
        assert!(
            err.starts_with("share:ipadic の辞書がありません ("),
            "{err}"
        );
        assert!(err.contains(&path.display().to_string()), "{err}");
        assert!(
            err.ends_with("。`noslop dict download ipadic` で取得できます"),
            "{err}"
        );
        let err = existing(path.clone(), "ipadic.hsd")
            .unwrap_err()
            .to_string();
        assert!(err.contains("noslop dict download ipadic`"), "{err}");
        // 配布辞書でない名前には取得の案内を付けない
        let err = existing(dir.path().join("mine.hsd"), "mine")
            .unwrap_err()
            .to_string();
        assert!(!err.contains("noslop dict download"), "{err}");

        fs::write(&path, b"x").unwrap();
        assert_eq!(existing(path.clone(), "ipadic").unwrap(), path);
    }

    // -----------------------------------------------------------------------
    // 表の整合
    // -----------------------------------------------------------------------

    /// 目録 (dict/catalog.json) の辞書を、依存の hasami で読める。目録の形 (名前・ファイル名・大きさ・
    /// SHA-256 の書式) は build.rs が確かめる。
    #[test]
    fn the_catalog_is_readable_by_the_linked_hasami() {
        assert_eq!(
            CATALOG_FORMAT_VERSION,
            hasami::hsd::FORMAT_VERSION,
            "目録 (dict/catalog.json) の辞書の形式を、依存の hasami は読めない。目録を戻すか、依存を上げる"
        );
        assert_eq!(
            DEFAULT_SOURCE,
            format!("https://github.com/owayo/hasami/releases/download/{HASAMI_TAG}"),
            "取得元は HASAMI_TAG のリリースの添付ファイル"
        );

        for (i, dict) in DICTIONARIES.iter().enumerate() {
            assert!(
                DICTIONARIES[..i].iter().all(|d| d.size <= dict.size),
                "小さい順に並ぶ: {}",
                dict.name
            );
            assert!(share_file_name(dict.name).is_ok(), "{}", dict.name);
            assert_eq!(find(dict.name), Some(dict));
            assert!(!dict.description().is_empty(), "{}", dict.name);
        }
        assert!(find(RECOMMENDED).is_some());
        assert!(find("unidic").is_none());
    }

    /// 同梱の辞書 (dict/ipadic.hsd) は、dict/README.md に記した SHA-256 のもの。目録の ipadic とは
    /// 別に上げる (目録は hasami のリリースごとに変わり、同梱の辞書は判定の校正の前提になるため)。
    #[test]
    fn the_bundled_dictionary_matches_dict_readme() {
        let dict_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("dict");
        let readme = fs::read_to_string(dict_dir.join("README.md")).unwrap();
        let row = |label: &str| {
            readme
                .lines()
                .find_map(|line| line.strip_prefix(&format!("| {label} | ")))
                .and_then(|rest| rest.strip_suffix(" |"))
                .unwrap_or_else(|| panic!("dict/README.md に {label} の行がない"))
                .to_string()
        };
        let sha256 = row("SHA-256").trim_matches('`').to_string();
        let bundled = dict_dir.join("ipadic.hsd");
        assert_eq!(hasami::download::sha256_file(&bundled).unwrap(), sha256);
    }

    // -----------------------------------------------------------------------
    // 検証
    // -----------------------------------------------------------------------

    /// 中身 `bytes` の大きさと SHA-256 を持つ、テスト用の配布辞書 (圧縮版なし)。
    fn distributed(name: &'static str, bytes: &[u8]) -> Distributed {
        let sha256: String = Sha256::digest(bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        Distributed {
            name,
            summary: "テスト用",
            size: bytes.len() as u64,
            sha256: sha256.leak(),
            compressed: None,
        }
    }

    #[test]
    fn files_are_checked_by_size_and_hash() {
        let dir = tempfile::tempdir().unwrap();
        let dict = distributed("t", b"abcdef");
        let path = dir.path().join("t.hsd");
        assert_eq!(check_file(&path, &dict).unwrap(), Check::Missing);
        fs::write(&path, b"abcdef").unwrap();
        assert_eq!(check_file(&path, &dict).unwrap(), Check::Verified);
        // 大きさが違う
        fs::write(&path, b"abcdefg").unwrap();
        assert_eq!(check_file(&path, &dict).unwrap(), Check::Differs);
        // 大きさは同じで中身が違う
        fs::write(&path, b"abcdeg").unwrap();
        assert_eq!(check_file(&path, &dict).unwrap(), Check::Differs);
        // ディレクトリは辞書ではない
        fs::create_dir(dir.path().join("d.hsd")).unwrap();
        assert_eq!(
            check_file(&dir.path().join("d.hsd"), &dict).unwrap(),
            Check::Differs
        );
    }

    #[test]
    fn list_reports_each_distributed_dictionary() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("ipadic.hsd"), b"not the real one").unwrap();
        let listed = list(dir.path());
        let names: Vec<&str> = listed.iter().map(|l| l.dictionary.name).collect();
        assert_eq!(names, DICTIONARIES.map(|d| d.name));
        assert_eq!(listed[0].path, dir.path().join("ipadic.hsd"));
        assert_eq!(*listed[0].check.as_ref().unwrap(), Check::Differs);
        assert_eq!(*listed[1].check.as_ref().unwrap(), Check::Missing);
    }

    // -----------------------------------------------------------------------
    // 取得 (取得そのものは hasami の download。ここでは目録の渡し方と言い換えを確かめる)
    // -----------------------------------------------------------------------

    /// 目録の圧縮版の情報を落とさずに hasami に渡し、hasami の検証にも通る。受け取る大きさは圧縮版を
    /// 取るかで変わる。
    #[test]
    fn catalog_entries_are_passed_to_hasami_with_the_compressed_file() {
        for dict in &DICTIONARIES {
            let converted = dict.to_hasami();
            converted
                .validate()
                .unwrap_or_else(|e| panic!("{}: {e}", dict.name));
            assert_eq!(converted.name, dict.name);
            assert_eq!(converted.file, dict.file_name());
            assert_eq!(converted.size, dict.size);
            assert_eq!(converted.sha256, dict.sha256);
            let c = dict
                .compressed
                .unwrap_or_else(|| panic!("{} に圧縮版がない", dict.name));
            assert_eq!(c.file, format!("{}.zst", dict.file_name()));
            assert!(c.size < dict.size, "{}", dict.name);
            let hc = converted.compressed.unwrap();
            assert_eq!(
                (hc.file.as_str(), hc.size, hc.sha256.as_str()),
                (c.file, c.size, c.sha256)
            );
            assert_eq!(dict.transfer_size(true), c.size);
            assert_eq!(dict.transfer_size(false), dict.size);
        }
        let plain = distributed("t", b"x");
        assert_eq!(
            plain.transfer_size(true),
            1,
            "圧縮版がなければ展開前の大きさ"
        );
        assert!(plain.to_hasami().compressed.is_none());
    }

    /// 既存の中身にかかわらず取得を試み、接続できなければ既存のファイルを保つ。
    #[test]
    fn failed_downloads_preserve_existing_files() {
        let dir = tempfile::tempdir().unwrap();
        let dict = distributed("t", b"abcdef");
        let path = dir.path().join("t.hsd");
        // 接続できない取得元。接続しようとしたら誤りになる
        let unreachable = "http://127.0.0.1:9";
        for old in [b"abcdef", b"abcdeg"] {
            fs::write(&path, old).unwrap();
            let err = download(&dict, dir.path(), unreachable, true, &mut |_, _| {})
                .unwrap_err()
                .to_string();
            assert!(err.contains("を取得できません"), "{err}");
            assert!(!err.contains("--force"), "{err}");
            assert_eq!(fs::read(&path).unwrap(), old, "既存のファイルを保つ");
        }
    }

    /// hasami の取得の誤りを、どこで何が起きたかが分かる日本語に言い換える。
    #[test]
    fn download_errors_are_explained_in_japanese() {
        let dict = find(RECOMMENDED).unwrap();
        let zst = format!("{DEFAULT_SOURCE}/{}.zst", dict.file_name());
        let hsd = format!("{DEFAULT_SOURCE}/{}", dict.file_name());
        let say = |e: HasamiError| DownloadError::from_hasami(e, dict).to_string();

        // HTTP の誤りは、実際に取得に失敗した URL とステータスを伝える
        let missing = say(HasamiError::Status {
            url: zst.clone(),
            status: 404,
        });
        assert!(
            missing.starts_with(&format!("{zst} を取得できません (HTTP 404)")),
            "{missing}"
        );
        assert!(!missing.contains("--uncompressed"), "{missing}");
        let missing = say(HasamiError::Status {
            url: hsd.clone(),
            status: 404,
        });
        assert_eq!(missing, format!("{hsd} を取得できません (HTTP 404)"));

        // 圧縮版の大きさの違いは、圧縮版と分かるように書く
        let length = say(HasamiError::ContentLength {
            url: zst.clone(),
            expected: 10,
            actual: 11,
        });
        assert!(length.contains("大きさが違います"), "{length}");
        assert!(
            length.contains(&format!("{} の圧縮版は 10 バイト", dict.name)),
            "{length}"
        );

        // 展開したものの誤りは「展開した中身」と書き、受信の誤りと分ける
        let decoded = format!("{zst}{DECOMPRESSED}");
        let checksum = say(HasamiError::Checksum {
            from: decoded.clone(),
            expected: "a".into(),
            actual: "b".into(),
        });
        assert_eq!(
            checksum,
            format!("{zst} を展開した中身の SHA-256 が違います (期待 a、実際 b)")
        );
        let short = say(HasamiError::Truncated {
            from: decoded,
            expected: 10,
            received: 3,
        });
        assert!(
            short.contains("を展開した中身の大きさが足りません (3 / 10 バイト)"),
            "{short}"
        );
        let cut = say(HasamiError::Truncated {
            from: zst.clone(),
            expected: 10,
            received: 3,
        });
        assert_eq!(
            cut,
            format!("{zst} の受信が途中で切れました (3 / 10 バイト)")
        );
        let broken = say(HasamiError::Decompress {
            from: zst.clone(),
            reason: "bad frame".into(),
        });
        assert_eq!(broken, format!("{zst} を展開できません (zstd): bad frame"));
        // 取得では起きない誤りも、hasami の説明を添えて返す
        let other = say(HasamiError::InvalidDict("x".into()));
        assert!(
            other.starts_with(&format!("{} を取得できません: ", dict.name)),
            "{other}"
        );
    }

    /// 取得元の添付ファイル (圧縮版) と目録に、HTTPS で届く (`make dict-check`)。TLS の設定 (provider・
    /// root_certs) の誤りは、実際に HTTPS でハンドシェイクするまで分からない。目録の中身が取得元の
    /// 目録と同じことも確かめる。
    #[test]
    #[ignore = "ネットワークが必要"]
    fn the_default_source_serves_the_catalog() {
        let remote = hasami::download::catalog_from(DEFAULT_SOURCE)
            .unwrap_or_else(|e| panic!("{DEFAULT_SOURCE}: {e}"));
        assert_eq!(format!("v{}", remote.hasami_version), HASAMI_TAG);
        for dict in &DICTIONARIES {
            let found = remote
                .find(dict.name)
                .unwrap_or_else(|| panic!("{}", dict.name));
            let ours = dict.to_hasami();
            assert_eq!(
                (&found.file, found.size, &found.sha256, &found.compressed),
                (&ours.file, ours.size, &ours.sha256, &ours.compressed),
                "{}",
                dict.name
            );
        }
    }

    /// 目録の辞書をすべて本番の取得元から取得し (圧縮版を展開する)、大きさ・SHA-256・依存の hasami で
    /// 読めることを確かめる。展開前の辞書を取る経路も、いちばん小さい辞書で確かめる (`make dict-check`。
    /// Release のワークフローが目録を更新したときに回す)。
    #[test]
    #[ignore = "ネットワークが必要 (目録の辞書をすべて圧縮版で取得し、非圧縮版の ipadic も取得する)"]
    fn every_catalog_dictionary_can_be_downloaded_and_read() {
        let dir = tempfile::tempdir().unwrap();
        let smallest = &DICTIONARIES[0];
        let cases = DICTIONARIES
            .iter()
            .map(|dict| (dict, true))
            .chain([(smallest, false)]);
        for (dict, compressed) in cases {
            let path = dir.path().join(dict.file_name());
            let mut total = 0;
            let outcome = download(dict, dir.path(), DEFAULT_SOURCE, compressed, &mut |_, t| {
                total = t;
            })
            .unwrap_or_else(|e| panic!("{} (compressed: {compressed}): {e}", dict.name));
            assert_eq!(outcome, path, "{}", dict.name);
            assert_eq!(total, dict.transfer_size(compressed), "{}", dict.name);
            assert_eq!(check_file(&path, dict).unwrap(), Check::Verified);
            // 次の辞書の分のディスクを空ける
            fs::remove_file(&path).unwrap();
        }
    }
}
