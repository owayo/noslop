//! Stop のフック (`hook git-diff` と、`hook claude-code` の Stop) の統合テスト。
//!
//! 組み込みルールの増減に左右されないよう、指摘は設定ファイルの独自ルール (`X01`) で確かめる。
//! 手元の noslop の設定・辞書と git の設定から切り離して動かす。

use std::fs;
use std::path::Path;
use std::process::{Command as StdCommand, Stdio};

use assert_cmd::Command;
use serde_json::{Value, json};
use tempfile::TempDir;

const CONFIG: &str = r#"
[[custom]]
id = "X01"
name = "TEAM_TERM"
pattern = "ユーザー様"
message = "「ユーザー様」ではなく「利用者」と書きます"
hint = "用語集の表記に合わせてください"
severity = "warning"
"#;

const DOC: &str =
    "# 利用案内\n\n新しい機能はユーザー様の声から生まれました。\n\n問題のない段落です。\n";

/// 中に何も置かないディレクトリ (テストの実行ごとに 1 つ)。
fn empty_dir() -> &'static Path {
    static EMPTY: std::sync::OnceLock<TempDir> = std::sync::OnceLock::new();
    EMPTY.get_or_init(|| tempfile::tempdir().unwrap()).path()
}

/// 手元の git の設定 (システムとユーザーの設定) と、外から渡されたリポジトリの指定を外す。
fn isolate_git(cmd: &mut StdCommand) {
    cmd.env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", empty_dir().join("gitconfig"))
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE");
}

/// 手元の設定と辞書、git の設定から切り離した noslop (tests/cli.rs の同名の関数に git の切り離しを
/// 足したもの)。
fn noslop(dir: &Path) -> Command {
    let mut std = StdCommand::new(env!("CARGO_BIN_EXE_noslop"));
    isolate_git(&mut std);
    let empty = empty_dir();
    std.current_dir(dir)
        .env_remove("NO_COLOR")
        .env_remove("CLICOLOR_FORCE")
        .env_remove("CLAUDE_PROJECT_DIR")
        .env("HOME", empty)
        .env("USERPROFILE", empty)
        .env("HASAMI_DATA_DIR", empty)
        .env_remove("HASAMI_DICT");
    Command::from_std(std)
}

/// テスト用の git (署名・フック・既定のブランチ名・改行の変換に左右されないようにする)。
/// 実行できなければ `false`。
fn git(dir: &Path, args: &[&str]) -> bool {
    let mut cmd = StdCommand::new("git");
    isolate_git(&mut cmd);
    cmd.arg("-C")
        .arg(dir)
        .args([
            "-c",
            "user.name=noslop",
            "-c",
            "user.email=you@example.com",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "init.defaultBranch=main",
            "-c",
            "core.autocrlf=false",
        ])
        .args(args)
        .stdin(Stdio::null())
        .output()
        .is_ok_and(|o| o.status.success())
}

/// 設定と指摘のある文書を置いたディレクトリ (git はまだ使わない)。
fn workspace() -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("noslop.toml"), CONFIG).unwrap();
    fs::create_dir(dir.path().join("docs")).unwrap();
    fs::write(dir.path().join("docs/guide.md"), DOC).unwrap();
    dir
}

/// `workspace` をコミットしたリポジトリ。git を実行できなければ `None`。
fn repository() -> Option<TempDir> {
    let dir = workspace();
    let ok = git(dir.path(), &["init", "-q"])
        && git(dir.path(), &["add", "-A"])
        && git(dir.path(), &["commit", "-q", "--no-verify", "-m", "init"]);
    if !ok {
        eprintln!("git を実行できないので、差分の確かめを飛ばします");
        return None;
    }
    Some(dir)
}

/// 段落を 1 つ足す (7 行目に指摘)。
fn append_paragraph(dir: &Path) {
    let path = dir.join("docs/guide.md");
    let mut doc = fs::read_to_string(&path).unwrap();
    doc.push_str("\n足した段落でもユーザー様と書きました。\n");
    fs::write(&path, doc).unwrap();
}

fn output(cmd: &mut Command) -> (Option<i32>, String, String) {
    let out = cmd.output().unwrap();
    (
        out.status.code(),
        String::from_utf8(out.stdout).unwrap(),
        String::from_utf8(out.stderr).unwrap(),
    )
}

/// `hook git-diff` は、指摘があれば改稿指示をテキストで書いて 1、なければ何も書かずに 0 で終わる。
/// git の外でも 0。設定の誤りは標準エラーに書いて 2。
#[test]
fn git_diff_exit_codes_follow_the_findings() {
    // git の外 (一時ディレクトリがリポジトリの中にある環境では確かめられない)
    let outside = workspace();
    if !git(outside.path(), &["rev-parse", "--git-dir"]) {
        let (code, stdout, stderr) = output(noslop(outside.path()).args(["hook", "git-diff"]));
        assert_eq!(code, Some(0), "{stderr}");
        assert_eq!(stdout, "");
    }

    let Some(dir) = repository() else { return };
    let run = |extra: &[&str]| output(noslop(dir.path()).args(["hook", "git-diff"]).args(extra));

    // コミットした直後は変わった行がない
    let (code, stdout, stderr) = run(&[]);
    assert_eq!(code, Some(0), "{stderr}");
    assert_eq!(stdout, "");

    // 足した段落の指摘だけを返し、前からある 3 行目の指摘は返さない
    append_paragraph(dir.path());
    let (code, stdout, stderr) = run(&[]);
    assert_eq!(code, Some(1), "{stderr}");
    assert!(
        stdout.starts_with("noslop が docs/guide.md に 独自ルールの指摘を 1 件見つけました。"),
        "{stdout}"
    );
    assert!(stdout.contains("  - L7: "), "{stdout}");
    assert!(!stdout.contains("L3:"), "{stdout}");
    assert!(
        stdout.contains("コミットしていない変更 (git の HEAD との差分の行と、追跡していないファイルの全体) に重なる指摘だけ"),
        "{stdout}"
    );
    assert!(
        stdout.contains("再び出ても直す必要はありません"),
        "{stdout}"
    );
    assert!(
        serde_json::from_str::<Value>(&stdout).is_err(),
        "JSON ではなくテキスト"
    );
    assert_eq!(stderr, "");

    // --whole-file は変わったファイルの全体、--max-chars は上限に収める
    let (code, stdout, _) = run(&["--whole-file"]);
    assert_eq!(code, Some(1));
    assert!(stdout.contains("L3:") && stdout.contains("L7:"), "{stdout}");
    let (code, stdout, _) = run(&["--max-chars", "150"]);
    assert_eq!(code, Some(1));
    assert!(stdout.chars().count() <= 150, "{stdout}");

    // 追跡していないファイルは全体を見る。.gitignore で無視するファイルは見ない
    fs::write(dir.path().join(".gitignore"), "ignored.md\n").unwrap();
    fs::write(dir.path().join("ignored.md"), "ユーザー様。\n").unwrap();
    fs::write(dir.path().join("new.md"), "ユーザー様。\n\nユーザー様。\n").unwrap();
    let (code, stdout, _) = run(&[]);
    assert_eq!(code, Some(1));
    // guide.md と new.md の 2 ファイル: 件数はファイルごとの行に出る
    assert!(
        stdout.contains("new.md: 独自ルールの指摘 2 件\n"),
        "{stdout}"
    );
    assert!(!stdout.contains("ignored.md"), "{stdout}");

    // 設定の誤りは 2 (git のリポジトリの中でだけ設定を読む)
    fs::write(dir.path().join("noslop.toml"), "[files\n").unwrap();
    let (code, stdout, stderr) = run(&[]);
    assert_eq!(code, Some(2), "{stdout}");
    assert_eq!(stdout, "");
    assert!(
        stderr.starts_with("noslop: 設定ファイルの書式が正しくありません"),
        "{stderr}"
    );
}

fn stop_event(dir: &Path, active: bool) -> String {
    json!({
        "session_id": "s",
        "transcript_path": dir.join("transcript.jsonl").to_string_lossy(),
        "cwd": dir.to_string_lossy(),
        "permission_mode": "default",
        "hook_event_name": "Stop",
        "stop_hook_active": active,
    })
    .to_string()
}

/// `hook claude-code` の Stop は、コミットしていない変更の指摘を `additionalContext` で返す。
/// このフックで続けた後の Stop (`stop_hook_active`) と、変更のないときは何も書かない。
#[test]
fn claude_code_stop_returns_findings_as_additional_context() {
    let Some(dir) = repository() else { return };
    // 作業ディレクトリは入力の cwd で決まる (プロセスのカレントディレクトリではない)
    let elsewhere = empty_dir();
    let run = |active: bool| {
        output(
            noslop(elsewhere)
                .args(["hook", "claude-code"])
                .write_stdin(stop_event(dir.path(), active)),
        )
    };

    let (code, stdout, stderr) = run(false);
    assert_eq!(code, Some(0), "{stderr}");
    assert_eq!(stdout, "", "変更がなければ何も書かない");

    append_paragraph(dir.path());
    let (code, stdout, stderr) = run(false);
    assert_eq!(code, Some(0), "{stderr}");
    let v: Value = serde_json::from_str(&stdout).unwrap();
    let hso = &v["hookSpecificOutput"];
    assert_eq!(hso["hookEventName"], "Stop");
    let context = hso["additionalContext"].as_str().unwrap();
    assert!(
        context.starts_with("noslop が docs/guide.md に 独自ルールの指摘を 1 件見つけました。"),
        "{context}"
    );
    assert!(
        context.contains("  - L7: ") && !context.contains("L3:"),
        "{context}"
    );
    assert!(
        context.contains("再び出ても直す必要はありません"),
        "{context}"
    );
    assert!(v.get("decision").is_none(), "Stop を止めない: {stdout}");

    let (code, stdout, _) = run(true);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "", "続けた後の Stop では何もしない");
}
