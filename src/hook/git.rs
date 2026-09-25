//! git の差分から、変わった行を求める。
//!
//! - [`git_changed_regions`]: 1 つのファイルの、HEAD との差分で変わった行 (`hook file`)
//! - [`WorkTree`]: 作業ツリーのコミットしていない変更 (HEAD との差分と追跡していないファイル) の
//!   ファイルと変わった行 (Stop の `hook claude-code` と `hook git-diff`)
//!
//! git は手元の設定や言語に左右されにくいようにそろえて呼ぶ ([`git_in`])。パスの一覧は `-z` の
//! 出力で受け取り、空白・引用符・日本語を含むパスも崩さずに扱う。

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Output, Stdio};

use crate::diagnostic::Span;
use crate::document::Document;

/// 1 回の git に渡すパスの合計の上限 (バイト)。Windows のコマンドラインの上限 (32,767 文字) より
/// 十分に短くし、超える分は分けて呼ぶ。
const PATHSPEC_BATCH_BYTES: usize = 8 * 1024;

/// `dir` で git を動かすコマンド。
fn git_in(dir: &Path) -> Command {
    let mut cmd = Command::new("git");
    cmd.arg("-C")
        .arg(dir)
        // 差分を取るついでに index の日時の情報を書き直さない。並んで動くほかのフック (自動の
        // コミットなど) の git と index.lock を取り合わないように
        .args(["-c", "diff.autoRefreshIndex=false"])
        // パスの * や ? をパターンとして読ませない
        .env("GIT_LITERAL_PATHSPECS", "1")
        // 誤りの文言で「リポジトリの外」を見分けるので、英語にそろえる
        .env("LC_ALL", "C")
        .stdin(Stdio::null());
    cmd
}

/// git を実行して出力を受け取る。実行できなければ誤り。
fn run(cmd: &mut Command) -> Result<Output, String> {
    cmd.output()
        .map_err(|e| format!("git を実行できません: {e}"))
}

/// 失敗した git の誤りの文言。
fn failure(what: &str, out: &Output) -> String {
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stderr = stderr.trim();
    if stderr.is_empty() {
        format!("git {what} が失敗しました ({})", out.status)
    } else {
        format!("git {what} が失敗しました: {stderr}")
    }
}

/// git の差分 (HEAD との比較) で変わった行 (原文上の範囲)。git の外・追跡していないファイル・
/// HEAD がない・git を実行できないときは `None` (ファイル全体)。コミットしていない変更がなければ空。
pub(super) fn git_changed_regions(path: &Path, doc: &Document) -> Option<Vec<Span>> {
    let lines = git_changed_lines(path)?;
    Some(line_regions(&lines, doc))
}

/// 行番号 (1 始まり) の集まりを、原文上の範囲にする。
pub(super) fn line_regions(lines: &BTreeSet<usize>, doc: &Document) -> Vec<Span> {
    lines
        .iter()
        .map(|&line| doc.lines.line_span(&doc.source, line))
        .collect()
}

fn git_changed_lines(path: &Path) -> Option<BTreeSet<usize>> {
    // git -C でファイルのディレクトリに移るので、相対パスのままでは指す先がずれる
    let path = std::path::absolute(path).ok()?;
    let dir = path.parent()?;
    let git = |args: &[&str]| {
        git_in(dir)
            .args(args)
            .arg(&path)
            .stderr(Stdio::null())
            .output()
            .ok()
    };
    // 追跡していないファイル (新しく作った・無視している) はファイル全体を見る
    if !git(&["ls-files", "--error-unmatch", "--"])?
        .status
        .success()
    {
        return None;
    }
    let diff = git(&[
        "diff",
        "--no-color",
        "--no-ext-diff",
        "--no-textconv",
        "-U0",
        "--inter-hunk-context=0",
        "HEAD",
        "--",
    ])?;
    if !diff.status.success() {
        return None;
    }
    unified_diff_lines(&String::from_utf8_lossy(&diff.stdout))
}

/// `git diff -U0` の出力から、変わった後のファイルの行番号 (1 始まり) を集める。削除だけの箇所は、
/// つなぎ目の前後の行を入れる。hunk の見出しの形が想定と違えば `None`。
pub(super) fn unified_diff_lines(diff: &str) -> Option<BTreeSet<usize>> {
    let mut lines = BTreeSet::new();
    // -U0 では本文の行は + か - で始まるので、@@ で始まる行は hunk の見出しだけ
    for header in diff.lines().filter(|l| l.starts_with("@@ ")) {
        hunk_lines(header, &mut lines)?;
    }
    Some(lines)
}

/// hunk の見出し (`@@ -<旧の開始>[,<行数>] +<新の開始>[,<行数>] @@`) から、変わった後のファイルの
/// 行番号を `lines` に足す。削除だけの hunk は、つなぎ目の前後の行を入れる。形が違えば `None`。
fn hunk_lines(header: &str, lines: &mut BTreeSet<usize>) -> Option<()> {
    let new = header.split_whitespace().nth(2)?.strip_prefix('+')?;
    let (start, count) = match new.split_once(',') {
        Some((start, count)) => (start.parse::<usize>().ok()?, count.parse::<usize>().ok()?),
        None => (new.parse::<usize>().ok()?, 1),
    };
    if count == 0 {
        // 削除だけ: start 行の後ろが消えた
        lines.insert(start.max(1));
        lines.insert(start + 1);
    } else {
        lines.extend(start.max(1)..start + count);
    }
    Some(())
}

/// コミットしていない変更があるファイル。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ChangedFile {
    /// ファイルのパス (作業ツリーの最上位を付けたもの)。
    pub path: PathBuf,
    /// 変わった行 (1 始まり)。`None` はファイル全体 (追跡していないファイルと、HEAD のない
    /// リポジトリのファイル。hunk の見出しを読めなかったときも)。
    pub lines: Option<BTreeSet<usize>>,
}

/// git の作業ツリー。
#[derive(Debug)]
pub(super) struct WorkTree {
    /// 最上位のディレクトリ。探し始めた場所のパスの形 (シンボリックリンクを解かない形) にそろえ、
    /// 表示名や設定の除外がその形のパスで決まるようにする。
    top: PathBuf,
}

impl WorkTree {
    /// `start` を含む git の作業ツリーを探す。リポジトリの外と、作業ツリーのない場所 (`.git` の中・
    /// bare リポジトリ) は `Ok(None)`。git を実行できないときなど、ほかの失敗は誤り。
    pub(super) fn discover(start: &Path) -> Result<Option<Self>, String> {
        let start = std::path::absolute(start)
            .map_err(|e| format!("{} を絶対パスにできません: {e}", start.display()))?;
        let out = run(git_in(&start).args(["rev-parse", "--show-cdup", "--show-toplevel"]))?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            if stderr.contains("not a git repository")
                || stderr.contains("must be run in a work tree")
            {
                return Ok(None);
            }
            return Err(failure("rev-parse", &out));
        }
        // 1 行目が最上位までの相対パス (`../` の並びか空)、残りが最上位の絶対パス
        let stdout = out.stdout.strip_suffix(b"\n").unwrap_or(&out.stdout);
        let Some(newline) = stdout.iter().position(|&b| b == b'\n') else {
            return Err(format!(
                "git rev-parse の出力を読めません: {}",
                String::from_utf8_lossy(&out.stdout)
            ));
        };
        let (cdup, toplevel) = (&stdout[..newline], &stdout[newline + 1..]);
        // git が返す最上位はシンボリックリンクを解いた形 (macOS の /var → /private/var など) なので、
        // 同じディレクトリを指すなら、探し始めた場所から `..` をたどった形を使う
        let physical = os_path(toplevel);
        let logical = normalize(&start.join(os_path(cdup)));
        let top = if same_dir(&logical, &physical) {
            logical
        } else {
            physical
        };
        Ok(Some(Self { top }))
    }

    /// コミットしていない変更のうち、`select` が選んだファイルと変わった行 (パスの順)。
    ///
    /// - HEAD との差分 (index と作業ツリーの両方): 変わった行。消したファイルは除き、名前を変えた
    ///   ファイルは新しいパスで、名前を変える前との差分を見る。中身の変わっていないファイル (名前・
    ///   モードだけの変更) は入れない
    /// - 追跡していないファイル (`.gitignore` などで無視するものは除く): ファイル全体
    /// - まだコミットがないリポジトリ: index に入れたファイルと追跡していないファイルの全体
    pub(super) fn changes(
        &self,
        select: impl Fn(&Path) -> bool,
    ) -> Result<Vec<ChangedFile>, String> {
        let mut files = Vec::new();
        if self.has_head()? {
            let mut tracked = Vec::new();
            for entry in self.changed_tracked()? {
                let path = self.top.join(os_path(&entry.path));
                if select(&path) {
                    tracked.push((entry, path));
                }
            }
            let entries: Vec<&NameStatus> = tracked.iter().map(|(e, _)| e).collect();
            let mut lines = self.changed_lines(&entries)?;
            for (entry, path) in tracked {
                // 差分に hunk のないファイルは、中身が変わっていない
                if let Some(lines) = lines.remove(&entry.path) {
                    files.push(ChangedFile { path, lines });
                }
            }
            for rel in self.ls_files(&["--others", "--exclude-standard"])? {
                let path = self.top.join(os_path(&rel));
                if select(&path) {
                    files.push(ChangedFile { path, lines: None });
                }
            }
        } else {
            for rel in self.ls_files(&["--cached", "--others", "--exclude-standard"])? {
                let path = self.top.join(os_path(&rel));
                if select(&path) {
                    files.push(ChangedFile { path, lines: None });
                }
            }
        }
        files.sort_by(|a, b| a.path.cmp(&b.path));
        files.dedup_by(|a, b| a.path == b.path);
        Ok(files)
    }

    /// HEAD (コミット) があるか。まだコミットのないリポジトリでは `false`。
    fn has_head(&self) -> Result<bool, String> {
        let out = run(git_in(&self.top).args(["rev-parse", "-q", "--verify", "HEAD^{commit}"]))?;
        match out.status.code() {
            Some(0) => Ok(true),
            Some(1) => Ok(false),
            _ => Err(failure("rev-parse", &out)),
        }
    }

    /// HEAD との差分があるファイル (最上位からの相対パス)。消したファイルは除く。
    fn changed_tracked(&self) -> Result<Vec<NameStatus>, String> {
        let out = run(git_in(&self.top).args([
            "diff",
            "--name-status",
            "-z",
            "-M",
            "--diff-filter=d",
            "--no-color",
            "--no-ext-diff",
            "--no-textconv",
            "--ignore-submodules",
            "HEAD",
            "--",
        ]))?;
        if !out.status.success() {
            return Err(failure("diff", &out));
        }
        parse_name_status(&out.stdout)
            .ok_or_else(|| "git diff --name-status の出力を読めません".to_string())
    }

    /// `entries` の、HEAD との差分で変わった行 (変わった後のパスごと)。hunk のないファイルは入らない。
    fn changed_lines(
        &self,
        entries: &[&NameStatus],
    ) -> Result<BTreeMap<Vec<u8>, Option<BTreeSet<usize>>>, String> {
        let mut lines = BTreeMap::new();
        for batch in batches(entries) {
            let mut cmd = git_in(&self.top);
            // 手元の設定 (diff.noprefix・diff.mnemonicPrefix・diff.interHunkContext・diff.external・
            // 色) に左右されない形の出力にする
            cmd.args([
                "-c",
                "core.quotePath=false",
                "diff",
                "-U0",
                "--inter-hunk-context=0",
                "-M",
                "--no-color",
                "--no-ext-diff",
                "--no-textconv",
                "--ignore-submodules",
                "--src-prefix=a/",
                "--dst-prefix=b/",
                "HEAD",
                "--",
            ]);
            for entry in batch {
                // 名前を変えたファイルは、前のパスも渡さないと新しいファイルに見える
                cmd.arg(os_arg(&entry.path));
                if let Some(old) = &entry.old_path {
                    cmd.arg(os_arg(old));
                }
            }
            let out = run(&mut cmd)?;
            if !out.status.success() {
                return Err(failure("diff", &out));
            }
            lines.extend(patch_lines_by_path(&out.stdout));
        }
        Ok(lines)
    }

    /// `git ls-files -z <args>` のパス (最上位からの相対パス)。
    fn ls_files(&self, args: &[&str]) -> Result<Vec<Vec<u8>>, String> {
        let out = run(git_in(&self.top).args(["ls-files", "-z"]).args(args))?;
        if !out.status.success() {
            return Err(failure("ls-files", &out));
        }
        Ok(out
            .stdout
            .split(|&b| b == 0)
            .filter(|p| !p.is_empty())
            .map(<[u8]>::to_vec)
            .collect())
    }
}

/// `git diff --name-status -z` の 1 件。
#[derive(Debug, Clone, PartialEq, Eq)]
struct NameStatus {
    /// 変わった後のパス (最上位からの相対パス)。
    path: Vec<u8>,
    /// 名前を変える・写す前のパス。
    old_path: Option<Vec<u8>>,
}

/// `git diff --name-status -z` の出力を読む。消したファイルは除く。形が違えば `None`。
fn parse_name_status(out: &[u8]) -> Option<Vec<NameStatus>> {
    let mut fields = out.split(|&b| b == 0);
    // 欄は空にならない。空の欄は最後の NUL の後ろか、途中で切れた出力
    let mut field = || fields.next().filter(|f| !f.is_empty()).map(<[u8]>::to_vec);
    let mut entries = Vec::new();
    while let Some(status) = field() {
        let first = field()?;
        match status[0] {
            // 名前を変えた (R) ・写した (C): 前のパスと後のパス
            b'R' | b'C' => entries.push(NameStatus {
                path: field()?,
                old_path: Some(first),
            }),
            b'D' => {}
            _ => entries.push(NameStatus {
                path: first,
                old_path: None,
            }),
        }
    }
    Some(entries)
}

/// パスの合計が [`PATHSPEC_BATCH_BYTES`] を超えないように分ける (1 件で超えるものは単独で)。
fn batches<'a>(entries: &[&'a NameStatus]) -> Vec<Vec<&'a NameStatus>> {
    let mut batches: Vec<Vec<&NameStatus>> = Vec::new();
    let mut size = 0;
    for &entry in entries {
        let n = entry.path.len() + entry.old_path.as_ref().map_or(0, Vec::len) + 2;
        match batches.last_mut() {
            Some(batch) if size + n <= PATHSPEC_BATCH_BYTES => {
                batch.push(entry);
                size += n;
            }
            _ => {
                batches.push(vec![entry]);
                size = n;
            }
        }
    }
    batches
}

/// `git diff -U0 --src-prefix=a/ --dst-prefix=b/` の出力 (複数のファイル) を、変わった後のパス
/// (最上位からの相対パス) ごとの変わった行に分ける。消したファイル (`+++ /dev/null`) の hunk は
/// 数えず、hunk のないファイルは入れない。hunk の見出しを読めないファイルは `None` (ファイル全体)。
fn patch_lines_by_path(patch: &[u8]) -> BTreeMap<Vec<u8>, Option<BTreeSet<usize>>> {
    let mut files: BTreeMap<Vec<u8>, Option<BTreeSet<usize>>> = BTreeMap::new();
    let mut current: Option<Vec<u8>> = None;
    // ファイルの見出し (`diff --git` から最初の hunk まで) の中か。本文の追加の行が `++ ` で
    // 始まると `+++ ` に見えるので、パスは見出しの中でだけ読む
    let mut in_header = false;
    for line in patch.split(|&b| b == b'\n') {
        if line.starts_with(b"diff --git ") {
            current = None;
            in_header = true;
        } else if in_header && line.starts_with(b"+++ ") {
            current = new_side_path(&line[4..]);
        } else if line.starts_with(b"@@ ") {
            // -U0 では本文の行は + か - (と「\ No newline」) で始まるので、@@ で始まる行は見出しだけ
            in_header = false;
            let Some(path) = &current else {
                continue;
            };
            let entry = files
                .entry(path.clone())
                .or_insert_with(|| Some(BTreeSet::new()));
            if let Some(lines) = entry
                && hunk_lines(&String::from_utf8_lossy(line), lines).is_none()
            {
                *entry = None;
            }
        }
    }
    files
}

/// `+++ ` の後ろ (`b/<パス>`・引用符で囲んだ `"b/<パス>"`・`/dev/null`) から、パスを取り出す。
/// 空白を含むパスの後ろには、git が区切りのタブを付ける。
fn new_side_path(field: &[u8]) -> Option<Vec<u8>> {
    let path = if field.first() == Some(&b'"') {
        unquote(field)?
    } else {
        field.strip_suffix(b"\t").unwrap_or(field).to_vec()
    };
    path.strip_prefix(b"b/").map(<[u8]>::to_vec)
}

/// git が C の文字列の書式で引用したパス (`"docs/a\"b.md"`) を元のバイト列に戻す。閉じ引用符の
/// 後ろは、区切りのタブだけを許す。
fn unquote(quoted: &[u8]) -> Option<Vec<u8>> {
    let mut bytes = quoted.strip_prefix(b"\"")?.iter().copied();
    let mut out = Vec::new();
    loop {
        match bytes.next()? {
            b'"' => break,
            b'\\' => {
                let c = bytes.next()?;
                out.push(match c {
                    b'a' => 0x07,
                    b'b' => 0x08,
                    b't' => b'\t',
                    b'n' => b'\n',
                    b'v' => 0x0b,
                    b'f' => 0x0c,
                    b'r' => b'\r',
                    b'"' | b'\\' => c,
                    // 8 進数 3 桁 (日本語などの UTF-8 のバイト)
                    b'0'..=b'3' => {
                        let mut value = c - b'0';
                        for _ in 0..2 {
                            let d = bytes.next()?;
                            if !(b'0'..=b'7').contains(&d) {
                                return None;
                            }
                            value = value * 8 + (d - b'0');
                        }
                        value
                    }
                    _ => return None,
                });
            }
            b => out.push(b),
        }
    }
    let rest: Vec<u8> = bytes.collect();
    (rest.is_empty() || rest == b"\t").then_some(out)
}

/// git が出力したパス (バイト列) を OS のパスにする。
fn os_path(bytes: &[u8]) -> PathBuf {
    PathBuf::from(os_arg(bytes))
}

/// git が出力したパス (バイト列) を、git に渡す引数にする。Unix ではバイト列のまま、ほかでは
/// UTF-8 として読む (Git for Windows のパスは UTF-8)。
#[cfg(unix)]
fn os_arg(bytes: &[u8]) -> OsString {
    use std::os::unix::ffi::OsStrExt;
    std::ffi::OsStr::from_bytes(bytes).to_os_string()
}

#[cfg(not(unix))]
fn os_arg(bytes: &[u8]) -> OsString {
    OsString::from(String::from_utf8_lossy(bytes).into_owned())
}

/// `..` と `.` を字面のまま解く (シンボリックリンクはたどらない)。
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir
                if matches!(out.components().next_back(), Some(Component::Normal(_))) =>
            {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

/// 2 つのパスが同じディレクトリを指すか (シンボリックリンクや Windows の短い名前を解いて比べる)。
fn same_dir(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unified_diff_lines_follow_the_hunk_headers() {
        let diff = "diff --git a/g.md b/g.md\nindex 1..2 100644\n--- a/g.md\n+++ b/g.md\n\
            @@ -3 +3 @@ 見出し\n-old\n+new\n\
            @@ -5,2 +6,0 @@\n-a\n-b\n\
            @@ -9,0 +10,2 @@\n+c\n+d\n";
        let lines: Vec<usize> = unified_diff_lines(diff).unwrap().into_iter().collect();
        // 3 行目の書き換え、6 行目の後ろの削除 (つなぎ目の 6・7 行目)、10〜11 行目の追加
        assert_eq!(lines, vec![3, 6, 7, 10, 11]);
        // 新しいファイルの全行と、差分なし
        let lines: Vec<usize> = unified_diff_lines("@@ -0,0 +1,3 @@\n+a\n+b\n+c\n")
            .unwrap()
            .into_iter()
            .collect();
        assert_eq!(lines, vec![1, 2, 3]);
        assert_eq!(unified_diff_lines(""), Some(BTreeSet::new()));
        assert_eq!(unified_diff_lines("@@ -1 +x @@\n"), None);
    }

    fn lines(v: &[usize]) -> Option<BTreeSet<usize>> {
        Some(v.iter().copied().collect())
    }

    /// 複数のファイルの差分を、変わった後のパスごとに分ける。空白のあるパスの区切りのタブ、引用符で
    /// 囲んだパス、消したファイル、`+++ ` で始まる本文の行、hunk のない名前の変更を崩さずに読む。
    #[test]
    fn patch_is_split_by_the_new_path() {
        let patch = b"diff --git a/my notes.md b/my notes.md\n\
            index 1..2 100644\n\
            --- a/my notes.md\t\n\
            +++ b/my notes.md\t\n\
            @@ -2 +2 @@ \xe8\xa6\x8b\xe5\x87\xba\xe3\x81\x97\n\
            -old\n\
            +++ looks like a header\n\
            @@ -5,0 +6,2 @@\n\
            +a\n\
            +b\n\
            diff --git \"a/q\\\"uote.md\" \"b/q\\\"uote.md\"\n\
            deleted file mode 100644\n\
            --- \"a/q\\\"uote.md\"\n\
            +++ /dev/null\n\
            @@ -1,3 +0,0 @@\n\
            -x\n\
            diff --git a/old.md b/new.md\n\
            similarity index 100%\n\
            rename from old.md\n\
            rename to new.md\n\
            diff --git \"a/\\346\\227\\245 \\346\\234\\254.md\" \"b/\\346\\227\\245 \\346\\234\\254.md\"\n\
            --- /dev/null\n\
            +++ \"b/\\346\\227\\245 \\346\\234\\254.md\"\t\n\
            @@ -0,0 +1,2 @@\n\
            +c\n\
            +d\n\
            diff --git a/broken.md b/broken.md\n\
            --- a/broken.md\n\
            +++ b/broken.md\n\
            @@ -1 +x @@\n";
        let files = patch_lines_by_path(patch);
        let keys: Vec<String> = files
            .keys()
            .map(|k| String::from_utf8(k.clone()).unwrap())
            .collect();
        assert_eq!(keys, vec!["broken.md", "my notes.md", "日 本.md"]);
        assert_eq!(files[b"my notes.md".as_slice()], lines(&[2, 6, 7]));
        assert_eq!(files["日 本.md".as_bytes()], lines(&[1, 2]));
        assert_eq!(files[b"broken.md".as_slice()], None, "読めない見出しは全体");
    }

    #[test]
    fn quoted_paths_are_unquoted() {
        assert_eq!(
            unquote(b"\"b/tab\\there\\\\\\\"q\\\"\"").unwrap(),
            b"b/tab\there\\\"q\"".to_vec()
        );
        assert_eq!(
            unquote(b"\"\\346\\227\\245.md\"\t").unwrap(),
            "日.md".as_bytes()
        );
        assert_eq!(unquote(b"\"unterminated"), None);
        assert_eq!(unquote(b"\"bad\\q\""), None);
        assert_eq!(unquote(b"\"bad\\38x\""), None);
        assert_eq!(unquote(b"\"a\" trailing"), None);
        assert_eq!(new_side_path(b"/dev/null"), None);
        assert_eq!(new_side_path(b"b/a b.md\t"), Some(b"a b.md".to_vec()));
    }

    #[test]
    fn name_status_keeps_the_new_path_of_renames_and_drops_deletions() {
        let out = b"M\0docs/a b.md\0R087\0old.md\0new name.md\0D\0gone.md\0A\0\xe6\x97\xa5.md\0";
        let entries = parse_name_status(out).unwrap();
        assert_eq!(
            entries,
            vec![
                NameStatus {
                    path: b"docs/a b.md".to_vec(),
                    old_path: None
                },
                NameStatus {
                    path: b"new name.md".to_vec(),
                    old_path: Some(b"old.md".to_vec())
                },
                NameStatus {
                    path: "日.md".as_bytes().to_vec(),
                    old_path: None
                },
            ]
        );
        assert_eq!(parse_name_status(b""), Some(Vec::new()));
        assert_eq!(parse_name_status(b"R100\0only-old.md\0"), None);
    }

    #[test]
    fn pathspecs_are_split_into_batches() {
        let long = NameStatus {
            path: vec![b'a'; PATHSPEC_BATCH_BYTES],
            old_path: None,
        };
        let short = NameStatus {
            path: b"b.md".to_vec(),
            old_path: Some(b"c.md".to_vec()),
        };
        let entries = vec![&short, &short, &long, &short];
        let sizes: Vec<usize> = batches(&entries).iter().map(Vec::len).collect();
        assert_eq!(sizes, vec![2, 1, 1]);
        assert!(batches(&[]).is_empty());
    }

    #[test]
    fn parent_components_are_resolved_lexically() {
        let base = std::path::absolute("repo").unwrap();
        assert_eq!(normalize(&base.join("sub/deep/../..")), base);
        assert_eq!(normalize(&base.join("./a/./b")), base.join("a/b"));
    }
}
