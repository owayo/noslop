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
//! - 取得した辞書は、辞書を指定しないとき (`auto`) に使う。share ディレクトリの配布辞書のうち、依存の
//!   hasami の推奨順 ([`preferred_in`]) で最初に見つかったものを、同梱の辞書より先に選ぶ
//!   ([`crate::morph::resolve`])。選ぶ順は目録ではなく依存の hasami で決まるので、目録の自動更新では
//!   変わらない
//! - 決まった辞書を使うときは `--dict share:<名前>` か、設定の `[morphology] dictionary = "share:<名前>"`
//!   で指定する ([`resolve_share`])
//! - 取得は保存先と同じディレクトリの一時ファイルに書き、大きさ・SHA-256・辞書として読めることを
//!   確かめてから rename で置く。途中で失敗しても、置き場所にある既存のファイルは消さず、壊さない

use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use sha2::{Digest, Sha256};
use ureq::config::ConfigBuilder;
use ureq::tls::{RootCerts, TlsConfig, TlsProvider};
use ureq::typestate::AgentScope;

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
}

impl Distributed {
    /// ファイル名 (`<名前>.hsd`)。
    pub fn file_name(&self) -> String {
        format!("{}.hsd", self.name)
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

/// 読み書きのバッファの大きさ。
const BUFFER_BYTES: usize = 256 * 1024;
/// 接続 (TLS のハンドシェイクを含む) の上限。
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
/// 要求を送ってから応答のヘッダーを受け取るまでの上限。
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(60);
/// 取得全体の上限。237MB を遅い回線で受け取る時間を見込む (1 時間で受け取るには約 0.53 Mbps 要る)。
const GLOBAL_TIMEOUT: Duration = Duration::from_secs(60 * 60);
/// これより古い一時ファイルは、前回の強制終了で残ったものとみなして消す。取得の上限
/// ([`GLOBAL_TIMEOUT`]) より十分長くして、ほかのプロセスが書いている途中のものは消さない。
const STALE_PART_AGE: Duration = Duration::from_secs(24 * 60 * 60);
/// 一時ファイルの名前の末尾 (名前は `.<名前>.hsd.<乱数>.part`)。
const PART_SUFFIX: &str = ".part";

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

/// 置き場所のファイルを、配布辞書の大きさと SHA-256 で確かめる。大きさが違えばハッシュを計算しない。
pub fn check_file(path: &Path, dict: &Distributed) -> io::Result<Check> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Check::Missing),
        Err(e) => return Err(e),
    };
    if !metadata.is_file() || metadata.len() != dict.size {
        return Ok(Check::Differs);
    }
    Ok(if sha256_file(path)? == dict.sha256 {
        Check::Verified
    } else {
        Check::Differs
    })
}

/// ファイルの SHA-256 (小文字の 16 進)。流し読みで計算する。
fn sha256_file(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; BUFFER_BYTES];
    loop {
        match file.read(&mut buffer) {
            Ok(0) => break,
            Ok(n) => hasher.update(&buffer[..n]),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(to_hex(&hasher.finalize()))
}

fn to_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut hex = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        hex.push(char::from(DIGITS[usize::from(b >> 4)]));
        hex.push(char::from(DIGITS[usize::from(b & 0x0f)]));
    }
    hex
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

/// 取得の結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// 正しいファイルがすでにあった (通信していない)。
    Present(PathBuf),
    /// 取得して置いた。
    Downloaded(PathBuf),
}

/// 取得の失敗。
#[derive(Debug, thiserror::Error)]
pub enum DownloadError {
    #[error("保存先のディレクトリを作れません: {path}: {source}", path = .path.display())]
    CreateDir {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("{path} を確かめられません: {source}", path = .path.display())]
    Inspect {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error(
        "{path} は hasami {HASAMI_TAG} の {name} と中身が違います (hasami の別の版か、壊れています)。置き換えるには --force を付けてください",
        path = .path.display()
    )]
    Differs { path: PathBuf, name: &'static str },
    #[error("一時ファイルを作れません: {dir}: {source}", dir = .dir.display())]
    TempFile {
        dir: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("{url} を取得できません: {source}")]
    Request {
        url: String,
        #[source]
        source: Box<ureq::Error>,
    },
    #[error("{url} を取得できません (HTTP {status})")]
    Status { url: String, status: u16 },
    #[error(
        "{url} の大きさが違います (Content-Length が {actual} バイト、hasami {HASAMI_TAG} の {name} は {expected} バイト)"
    )]
    ContentLength {
        url: String,
        name: &'static str,
        expected: u64,
        actual: u64,
    },
    #[error("{url} の受信に失敗しました: {source}")]
    Receive {
        url: String,
        #[source]
        source: io::Error,
    },
    #[error(
        "{url} の大きさが違います (hasami {HASAMI_TAG} の {name} の {expected} バイトを超えて受信しました)"
    )]
    Oversized {
        url: String,
        name: &'static str,
        expected: u64,
    },
    #[error("{url} の受信が途中で切れました ({received} / {expected} バイト)")]
    Truncated {
        url: String,
        expected: u64,
        received: u64,
    },
    #[error("{url} の SHA-256 が違います (期待 {expected}、実際 {actual})")]
    Checksum {
        url: String,
        expected: &'static str,
        actual: String,
    },
    #[error("一時ファイルに書き込めません: {path}: {source}", path = .path.display())]
    Write {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("取得したファイルを辞書として読めません ({url}): {message}")]
    NotDictionary { url: String, message: String },
    #[error("{path} に置けません: {source}", path = .path.display())]
    Persist {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

/// 配布辞書を `dir` に取得する (置き場所は `<dir>/<名前>.hsd`)。
///
/// `source` は取得元の URL の接頭辞 (既定は [`DEFAULT_SOURCE`])。`<source>/<名前>.hsd` を取得する。
/// 正しいファイルがすでにあれば、`force` でなければ通信せずに [`Outcome::Present`] を返す。中身の
/// 違うファイルがあれば、`force` でなければエラーにする。
///
/// `progress(受信したバイト数, 全体のバイト数)` は、通信を始める前に 1 度 (受信 0 で)、その後は
/// 受け取るたびに呼ぶ。通信しないときは呼ばない。
pub fn download(
    dict: &Distributed,
    dir: &Path,
    source: &str,
    force: bool,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<Outcome, DownloadError> {
    download_with(&agent(), dict, dir, source, force, progress)
}

/// 取得に使う HTTP の設定。プロキシは ureq の既定 (環境変数 `HTTPS_PROXY`・`NO_PROXY` など) のまま。
fn agent_config() -> ConfigBuilder<AgentScope> {
    ureq::Agent::config_builder()
        .user_agent(concat!("noslop/", env!("CARGO_PKG_VERSION")))
        .timeout_connect(Some(CONNECT_TIMEOUT))
        .timeout_recv_response(Some(RESPONSE_TIMEOUT))
        .timeout_global(Some(GLOBAL_TIMEOUT))
        // 2xx 以外はすべて自分で確かめる (既定では 3xx が成功として返る)
        .http_status_as_error(false)
        // provider と root_certs は明示する。root_certs の既定 (WebPki) は同梱のルート証明書だけを
        // 信頼するので、社内の CA を OS に入れた環境 (TLS を検査するプロキシの下など) で通らない
        .tls_config(
            TlsConfig::builder()
                .provider(TlsProvider::Rustls)
                .root_certs(RootCerts::PlatformVerifier)
                .build(),
        )
}

fn agent() -> ureq::Agent {
    agent_config().build().new_agent()
}

fn download_with(
    agent: &ureq::Agent,
    dict: &Distributed,
    dir: &Path,
    source: &str,
    force: bool,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<Outcome, DownloadError> {
    fs::create_dir_all(dir).map_err(|source| DownloadError::CreateDir {
        path: dir.to_path_buf(),
        source,
    })?;
    let path = dir.join(dict.file_name());
    // force なら中身を問わず取り直すので、既存のファイルのハッシュは計算しない
    if !force {
        let check = check_file(&path, dict).map_err(|source| DownloadError::Inspect {
            path: path.clone(),
            source,
        })?;
        match check {
            Check::Verified => return Ok(Outcome::Present(path)),
            Check::Differs => {
                return Err(DownloadError::Differs {
                    path,
                    name: dict.name,
                });
            }
            Check::Missing => {}
        }
    }
    remove_stale_parts(dir, dict, SystemTime::now());

    // 置き場所と同じディレクトリに書き、確かめてから rename で置き換える。失敗したら (エラーでも
    // パニックでも) 一時ファイルは drop で消え、置き場所にある既存のファイルには触れない
    let prefix = part_prefix(dict);
    let mut builder = tempfile::Builder::new();
    builder.prefix(&prefix).suffix(PART_SUFFIX);
    // 置いた辞書は、普通に作ったファイルと同じく umask に従わせる (tempfile の既定は所有者だけが
    // 読み書きできる 0600 で、共有の場所に置くとほかの利用者が読めない)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        builder.permissions(fs::Permissions::from_mode(0o666));
    }
    let mut part = builder
        .tempfile_in(dir)
        .map_err(|source| DownloadError::TempFile {
            dir: dir.to_path_buf(),
            source,
        })?;
    let url = format!("{}/{}", source.trim_end_matches('/'), dict.file_name());
    progress(0, dict.size);
    receive(agent, &url, dict, part.as_file_mut(), progress).map_err(|e| match e {
        Received::Http(e) => e,
        Received::Write(source) => DownloadError::Write {
            path: part.path().to_path_buf(),
            source,
        },
    })?;
    let written = part
        .as_file_mut()
        .flush()
        .and_then(|()| part.as_file().sync_all());
    written.map_err(|source| DownloadError::Write {
        path: part.path().to_path_buf(),
        source,
    })?;

    // 辞書として読めることを確かめる。読んだ辞書 (ファイルの mmap) は置き換えの前に捨てる
    // (Windows では開いたままのファイルを rename できない)
    if let Err(e) = hasami::Dictionary::load(part.path()) {
        return Err(DownloadError::NotDictionary {
            url,
            message: e.to_string(),
        });
    }

    part.persist(&path).map_err(|e| DownloadError::Persist {
        path: path.clone(),
        source: e.error,
    })?;
    Ok(Outcome::Downloaded(path))
}

/// 受信の失敗 (書き込みの失敗は、一時ファイルのパスを添えて呼び出し側でエラーにする)。
enum Received {
    Http(DownloadError),
    Write(io::Error),
}

/// `url` を GET して `out` に書く。大きさと SHA-256 を、受け取りながら確かめる。
fn receive(
    agent: &ureq::Agent,
    url: &str,
    dict: &Distributed,
    out: &mut File,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<(), Received> {
    let http = Received::Http;
    let mut response = agent.get(url).call().map_err(|source| {
        http(DownloadError::Request {
            url: url.to_string(),
            source: Box::new(source),
        })
    })?;
    let status = response.status();
    if !status.is_success() {
        return Err(http(DownloadError::Status {
            url: url.to_string(),
            status: status.as_u16(),
        }));
    }
    // 大きさが違うと分かっていれば、本体を読まずにやめる。ヘッダーを直に読む (ureq は
    // `Content-Length: 0` を本体なしとみなし、Body::content_length では None を返すため)
    let declared = response
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<u64>().ok());
    if let Some(actual) = declared
        && actual != dict.size
    {
        return Err(http(DownloadError::ContentLength {
            url: url.to_string(),
            name: dict.name,
            expected: dict.size,
            actual,
        }));
    }

    let mut reader = response.body_mut().as_reader();
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; BUFFER_BYTES];
    let mut received: u64 = 0;
    loop {
        let n = match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(source) => {
                return Err(http(DownloadError::Receive {
                    url: url.to_string(),
                    source,
                }));
            }
        };
        received += n as u64;
        if received > dict.size {
            return Err(http(DownloadError::Oversized {
                url: url.to_string(),
                name: dict.name,
                expected: dict.size,
            }));
        }
        let chunk = &buffer[..n];
        hasher.update(chunk);
        out.write_all(chunk).map_err(Received::Write)?;
        progress(received, dict.size);
    }
    if received != dict.size {
        return Err(http(DownloadError::Truncated {
            url: url.to_string(),
            expected: dict.size,
            received,
        }));
    }
    let actual = to_hex(&hasher.finalize());
    if actual != dict.sha256 {
        return Err(http(DownloadError::Checksum {
            url: url.to_string(),
            expected: dict.sha256,
            actual,
        }));
    }
    Ok(())
}

/// 一時ファイルの名前の先頭 (`.<名前>.hsd.`)。
fn part_prefix(dict: &Distributed) -> String {
    format!(".{}.", dict.file_name())
}

/// 前回の強制終了で残った、同じ辞書の一時ファイル ([`STALE_PART_AGE`] より古いもの) を消す。
/// 消せなくても取得は続ける。
fn remove_stale_parts(dir: &Path, dict: &Distributed, now: SystemTime) {
    let prefix = part_prefix(dict);
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if !(name.starts_with(&prefix) && name.ends_with(PART_SUFFIX)) {
            continue;
        }
        // symlink はたどらずに確かめる (ファイルそのものだけを消す)
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        let stale = metadata.is_file()
            && metadata
                .modified()
                .ok()
                .and_then(|modified| now.duration_since(modified).ok())
                .is_some_and(|age| age > STALE_PART_AGE);
        if stale {
            let _ = fs::remove_file(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader};
    use std::net::{Shutdown, TcpListener, TcpStream};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::thread;

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

    /// 同梱の辞書 (dict/ipadic.hsd) は、dict/README.md に記した大きさと SHA-256 のもの。目録の ipadic とは
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
        let size: u64 = row("大きさ")
            .trim_end_matches(" バイト")
            .replace(',', "")
            .parse()
            .unwrap();
        let sha256 = row("SHA-256").trim_matches('`').to_string();
        let bundled = dict_dir.join("ipadic.hsd");
        assert_eq!(fs::metadata(&bundled).unwrap().len(), size);
        assert_eq!(sha256_file(&bundled).unwrap(), sha256);
    }

    // -----------------------------------------------------------------------
    // 検証
    // -----------------------------------------------------------------------

    /// 中身 `bytes` の大きさと SHA-256 を持つ、テスト用の配布辞書。
    fn distributed(name: &'static str, bytes: &[u8]) -> Distributed {
        Distributed {
            name,
            summary: "テスト用",
            size: bytes.len() as u64,
            sha256: to_hex(&Sha256::digest(bytes)).leak(),
        }
    }

    #[test]
    fn to_hex_is_lowercase() {
        assert_eq!(to_hex(&[0x00, 0x0f, 0xa0, 0xff]), "000fa0ff");
        // 空の入力の SHA-256 (よく知られた値)
        assert_eq!(
            to_hex(&Sha256::digest(b"")),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
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
    // 取得 (127.0.0.1 の小さな HTTP サーバーを相手にする。外には出ない)
    // -----------------------------------------------------------------------

    /// テスト用の小さな辞書 (hasami の .hsd) のバイト列。
    fn dictionary_bytes() -> Vec<u8> {
        use hasami::DictEntry;
        use hasami::dict::DictBuilder;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.hsd");
        let mut builder = DictBuilder::new();
        for (surface, pos) in [("猫", "名詞,一般,*,*"), ("です", "助動詞,*,*,*")] {
            builder.add_entry(DictEntry {
                surface: surface.into(),
                left_id: 1,
                right_id: 1,
                cost: 1000,
                pos: pos.into(),
                base_form: surface.into(),
                ..Default::default()
            });
        }
        builder
            .write_hsd(&path, &builder.write_options(), |_, _| {})
            .unwrap();
        fs::read(&path).unwrap()
    }

    /// サーバーの応答。
    #[derive(Clone)]
    enum Reply {
        /// 200 と Content-Length 付きで本体を返す。
        Body(Vec<u8>),
        /// 200 で、Content-Length を付けずに本体を返してから接続を閉じる。
        CloseDelimited(Vec<u8>),
        /// 200 で、Content-Length に本体と違う値を書く。
        WrongLength(Vec<u8>, u64),
        /// 本体を返さずにステータスだけ返す。
        Status(u16, &'static str),
    }

    struct Server {
        url: String,
        requests: Arc<AtomicUsize>,
    }

    impl Server {
        fn requests(&self) -> usize {
            self.requests.load(Ordering::SeqCst)
        }
    }

    /// `path` への GET に `reply` を返すサーバーを立てる (ほかのパスには 404)。
    fn serve(path: &'static str, reply: Reply) -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(AtomicUsize::new(0));
        let count = Arc::clone(&requests);
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                count.fetch_add(1, Ordering::SeqCst);
                // 読み手が途中でやめたときの書き込みの失敗は気にしない
                let _ = respond(stream, path, &reply);
            }
        });
        Server { url, requests }
    }

    fn respond(mut stream: TcpStream, path: &str, reply: &Reply) -> io::Result<()> {
        let mut reader = BufReader::new(stream.try_clone()?);
        let mut request_line = String::new();
        reader.read_line(&mut request_line)?;
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line)? == 0 || line == "\r\n" {
                break;
            }
        }
        let requested = request_line.split_whitespace().nth(1).unwrap_or_default();
        let reply = if request_line.starts_with("GET ") && requested == path {
            reply.clone()
        } else {
            Reply::Status(404, "Not Found")
        };
        let (head, body) = match reply {
            Reply::Body(body) => (format!("200 OK\r\nContent-Length: {}", body.len()), body),
            Reply::CloseDelimited(body) => ("200 OK".to_string(), body),
            Reply::WrongLength(body, length) => {
                (format!("200 OK\r\nContent-Length: {length}"), body)
            }
            Reply::Status(code, reason) => {
                (format!("{code} {reason}\r\nContent-Length: 0"), Vec::new())
            }
        };
        stream.write_all(format!("HTTP/1.1 {head}\r\nConnection: close\r\n\r\n").as_bytes())?;
        stream.write_all(&body)?;
        stream.flush()?;
        close_after_peer(&stream, &mut reader)
    }

    /// 送信側だけを閉じ、相手が読み終えて閉じるまで待つ (待つのは 10 秒まで)。書き終えてすぐに
    /// 閉じると、Windows では相手が本文を受け取っている途中で接続が切られることがある
    /// (統合テストで `Peer disconnected` になった)。
    fn close_after_peer(stream: &TcpStream, reader: &mut BufReader<TcpStream>) -> io::Result<()> {
        stream.shutdown(Shutdown::Write)?;
        reader
            .get_ref()
            .set_read_timeout(Some(Duration::from_secs(10)))?;
        io::copy(reader, &mut io::sink()).map(drop)
    }

    /// プロキシの環境変数に左右されない HTTP の設定 (ほかは本番と同じ)。
    fn test_agent() -> ureq::Agent {
        agent_config().proxy(None).build().new_agent()
    }

    fn fetch(
        dict: &Distributed,
        dir: &Path,
        server: &Server,
        force: bool,
    ) -> Result<Outcome, DownloadError> {
        download_with(&test_agent(), dict, dir, &server.url, force, &mut |_, _| {})
    }

    /// ディレクトリに残った一時ファイル。
    fn parts(dir: &Path) -> Vec<String> {
        fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .filter(|name| name.ends_with(PART_SUFFIX))
            .collect()
    }

    #[test]
    fn a_dictionary_is_downloaded_verified_and_placed() {
        let bytes = dictionary_bytes();
        let dict = distributed("test", &bytes);
        let server = serve("/dict/test.hsd", Reply::Body(bytes.clone()));
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("share").join("hasami");
        let mut calls = Vec::new();
        // 取得元の末尾の / は重ねない。保存先のディレクトリは作る
        let source = format!("{}/dict/", server.url);
        let outcome = download_with(&test_agent(), &dict, &dir, &source, false, &mut |r, t| {
            calls.push((r, t))
        })
        .unwrap();
        let path = dir.join("test.hsd");
        assert_eq!(outcome, Outcome::Downloaded(path.clone()));
        assert_eq!(fs::read(&path).unwrap(), bytes);
        assert!(parts(&dir).is_empty(), "{:?}", parts(&dir));
        assert_eq!(server.requests(), 1);
        assert_eq!(calls.first(), Some(&(0, dict.size)));
        assert_eq!(calls.last(), Some(&(dict.size, dict.size)));
        assert!(hasami::Dictionary::load(&path).is_ok());
        // 権限は普通に作ったファイルと同じ (一時ファイルの 0600 のままにしない)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let reference = dir.join("reference");
            File::create(&reference).unwrap();
            let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(&path), mode(&reference));
        }

        // 取得済みなら通信しない
        let again = download_with(&test_agent(), &dict, &dir, &source, false, &mut |r, t| {
            calls.push((r, t))
        })
        .unwrap();
        assert_eq!(again, Outcome::Present(path));
        assert_eq!(server.requests(), 1);
        assert_eq!(calls.last(), Some(&(dict.size, dict.size)));
    }

    #[test]
    fn a_different_file_is_not_replaced_without_force() {
        let bytes = dictionary_bytes();
        let dict = distributed("test", &bytes);
        let server = serve("/test.hsd", Reply::Body(bytes.clone()));
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.hsd");
        fs::write(&path, b"another version").unwrap();

        let err = fetch(&dict, dir.path(), &server, false).unwrap_err();
        assert!(matches!(err, DownloadError::Differs { .. }), "{err}");
        let message = err.to_string();
        assert!(message.contains("中身が違います"), "{message}");
        assert!(message.contains("--force"), "{message}");
        assert!(message.contains(HASAMI_TAG), "{message}");
        assert_eq!(fs::read(&path).unwrap(), b"another version");
        assert_eq!(server.requests(), 0);

        // force なら置き換える
        let outcome = fetch(&dict, dir.path(), &server, true).unwrap();
        assert_eq!(outcome, Outcome::Downloaded(path.clone()));
        assert_eq!(fs::read(&path).unwrap(), bytes);
        assert!(parts(dir.path()).is_empty());
    }

    /// 取得に失敗したら、置き場所には何も置かず、一時ファイルも残さない。
    fn assert_fails_and_leaves_nothing(
        dict: &Distributed,
        reply: Reply,
        expected: impl Fn(&DownloadError) -> bool,
    ) -> String {
        let server = serve("/test.hsd", reply);
        let dir = tempfile::tempdir().unwrap();
        let err = fetch(dict, dir.path(), &server, false).unwrap_err();
        assert!(expected(&err), "{err:?}");
        assert!(!dir.path().join("test.hsd").exists());
        assert!(parts(dir.path()).is_empty(), "{:?}", parts(dir.path()));
        err.to_string()
    }

    #[test]
    fn a_different_hash_is_rejected() {
        let bytes = dictionary_bytes();
        let dict = distributed("test", &bytes);
        let mut other = bytes.clone();
        *other.last_mut().unwrap() ^= 0xff;
        let message = assert_fails_and_leaves_nothing(&dict, Reply::Body(other.clone()), |e| {
            matches!(e, DownloadError::Checksum { .. })
        });
        assert!(message.contains(dict.sha256), "{message}");
        assert!(
            message.contains(&to_hex(&Sha256::digest(&other))),
            "{message}"
        );
    }

    #[test]
    fn a_truncated_body_is_rejected() {
        let bytes = dictionary_bytes();
        let dict = distributed("test", &bytes);
        let half = bytes[..bytes.len() / 2].to_vec();
        // Content-Length がなく、接続が閉じて終わる
        let message =
            assert_fails_and_leaves_nothing(&dict, Reply::CloseDelimited(half.clone()), |e| {
                matches!(e, DownloadError::Truncated { .. })
            });
        assert!(message.contains("途中で切れました"), "{message}");
        // Content-Length に届かないまま接続が閉じる
        assert_fails_and_leaves_nothing(&dict, Reply::WrongLength(half, dict.size), |e| {
            matches!(e, DownloadError::Receive { .. })
        });
    }

    #[test]
    fn an_oversized_body_is_rejected() {
        let bytes = dictionary_bytes();
        let dict = distributed("test", &bytes);
        let mut longer = bytes.clone();
        longer.extend_from_slice(b"extra");
        assert_fails_and_leaves_nothing(&dict, Reply::CloseDelimited(longer), |e| {
            matches!(e, DownloadError::Oversized { .. })
        });
    }

    #[test]
    fn a_different_content_length_is_rejected_before_the_body() {
        let bytes = dictionary_bytes();
        let dict = distributed("test", &bytes);
        let message = assert_fails_and_leaves_nothing(
            &dict,
            Reply::WrongLength(bytes.clone(), dict.size + 10),
            |e| matches!(e, DownloadError::ContentLength { actual, .. } if *actual == dict.size + 10),
        );
        assert!(message.contains("Content-Length"), "{message}");
        // Content-Length: 0 (ureq は本体なしとみなす) も、大きさの違いとして報告する
        assert_fails_and_leaves_nothing(&dict, Reply::Status(200, "OK"), |e| {
            matches!(e, DownloadError::ContentLength { actual: 0, .. })
        });
    }

    #[test]
    fn http_errors_are_reported() {
        let bytes = dictionary_bytes();
        let dict = distributed("test", &bytes);
        let message =
            assert_fails_and_leaves_nothing(&dict, Reply::Status(404, "Not Found"), |e| {
                matches!(e, DownloadError::Status { status: 404, .. })
            });
        assert!(message.contains("HTTP 404"), "{message}");
        assert!(message.contains("/test.hsd"), "{message}");
        // ureq がたどらない 3xx も失敗にする (既定では成功として返ってくる)
        assert_fails_and_leaves_nothing(&dict, Reply::Status(304, "Not Modified"), |e| {
            matches!(e, DownloadError::Status { status: 304, .. })
        });
    }

    #[test]
    fn a_body_that_is_not_a_dictionary_is_rejected() {
        // 大きさと SHA-256 は合う (その中身で作った表) が、hasami の辞書ではない
        let bytes = b"this is not a hasami dictionary\n".repeat(64);
        let dict = distributed("test", &bytes);
        let message = assert_fails_and_leaves_nothing(&dict, Reply::Body(bytes.clone()), |e| {
            matches!(e, DownloadError::NotDictionary { .. })
        });
        assert!(message.contains("辞書として読めません"), "{message}");
    }

    #[test]
    fn a_failed_forced_download_keeps_the_existing_file() {
        let bytes = dictionary_bytes();
        let dict = distributed("test", &bytes);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.hsd");
        fs::write(&path, &bytes).unwrap();
        for reply in [
            Reply::Status(500, "Internal Server Error"),
            Reply::Body(vec![0; bytes.len()]),
            Reply::CloseDelimited(bytes[..10].to_vec()),
        ] {
            let server = serve("/test.hsd", reply);
            assert!(fetch(&dict, dir.path(), &server, true).is_err());
            assert_eq!(server.requests(), 1, "force なら取得済みでも取り直す");
            assert_eq!(fs::read(&path).unwrap(), bytes);
            assert!(parts(dir.path()).is_empty(), "{:?}", parts(dir.path()));
        }
    }

    #[test]
    fn stale_partial_files_are_removed() {
        let bytes = dictionary_bytes();
        let dict = distributed("test", &bytes);
        let server = serve("/test.hsd", Reply::Body(bytes));
        let dir = tempfile::tempdir().unwrap();
        let aged = |name: &str, age: Duration| {
            let path = dir.path().join(name);
            let file = File::create(&path).unwrap();
            file.set_modified(SystemTime::now() - age).unwrap();
            path
        };
        let day = Duration::from_secs(24 * 60 * 60);
        let old = aged(".test.hsd.abc123.part", day + Duration::from_secs(3600));
        let fresh = aged(".test.hsd.def456.part", Duration::from_secs(60));
        // ほかの辞書の一時ファイルと、名前の形が違うファイルには触れない
        let other = aged(".test2.hsd.abc123.part", day * 2);
        let unrelated = aged(".test.hsd.abc123.keep", day * 2);

        fetch(&dict, dir.path(), &server, false).unwrap();
        assert!(!old.exists());
        assert!(fresh.exists());
        assert!(other.exists());
        assert!(unrelated.exists());
    }

    /// 本番の取得元に疎通できることを確かめる (辞書の本体は受け取らない)。TLS の設定 (provider と
    /// root_certs) の誤りは、実際に HTTPS でハンドシェイクするまで分からない。
    #[test]
    #[ignore = "ネットワークが必要"]
    fn the_default_source_is_reachable() {
        let dict = find("ipadic").unwrap();
        let url = format!("{DEFAULT_SOURCE}/{}", dict.file_name());
        let response = agent()
            .head(&url)
            .call()
            .unwrap_or_else(|e| panic!("{url}: {e}"));
        assert_eq!(response.status().as_u16(), 200, "{url}");
        let length = response
            .headers()
            .get("content-length")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok());
        assert_eq!(length, Some(dict.size), "{url}");
    }

    /// 目録の辞書をすべて本番の取得元から取得し、大きさ・SHA-256・依存の hasami で読めることを確かめる
    /// (`make dict-check`。Release のワークフローが目録を更新したときに回す)。
    #[test]
    #[ignore = "ネットワークが必要 (目録の辞書をすべて、合わせて約 477MB 取得する)"]
    fn every_catalog_dictionary_can_be_downloaded_and_read() {
        let dir = tempfile::tempdir().unwrap();
        for dict in &DICTIONARIES {
            let path = dir.path().join(dict.file_name());
            let outcome = download(dict, dir.path(), DEFAULT_SOURCE, false, &mut |_, _| {})
                .unwrap_or_else(|e| panic!("{}: {e}", dict.name));
            assert_eq!(outcome, Outcome::Downloaded(path.clone()), "{}", dict.name);
            assert_eq!(check_file(&path, dict).unwrap(), Check::Verified);
            // 次の辞書の分のディスクを空ける
            fs::remove_file(&path).unwrap();
        }
    }
}
