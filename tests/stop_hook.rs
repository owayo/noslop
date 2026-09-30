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

fn changed_report(dir: &Path, paths: &[&str]) -> Value {
    let (code, stdout, stderr) = output(
        noslop(dir)
            .args([
                "check",
                "--git-diff",
                "--no-readability",
                "--format",
                "json",
            ])
            .args(paths),
    );
    assert_eq!(code, Some(0), "{stderr}");
    assert_eq!(stderr, "");
    serde_json::from_str(&stdout).unwrap()
}

/// フックの全件の出力と、check の JSON のファイル・ルール・行の多重集合を比べる。
fn assert_same_hook_findings(dir: &Path, report: &Value) {
    let (_, hook, stderr) = output(noslop(dir).args([
        "hook",
        "git-diff",
        "--max-chars",
        "unlimited",
        "--brief-limit",
        "unlimited",
    ]));
    assert_eq!(stderr, "");
    let files = report["files"].as_array().unwrap();
    let mut expected = Vec::new();
    for file in files {
        for finding in file["diagnostics"].as_array().unwrap() {
            if finding["suppressed"].is_null() {
                expected.push((
                    file["path"].as_str().unwrap().to_string(),
                    finding["ruleId"].as_str().unwrap().to_string(),
                    finding["range"]["start"]["line"].as_u64().unwrap(),
                ));
            }
        }
    }
    let mut actual = Vec::new();
    let mut path = files
        .first()
        .map(|f| f["path"].as_str().unwrap())
        .unwrap_or("");
    let mut rule = "";
    for line in hook.lines() {
        if let Some(file) = files
            .iter()
            .find(|file| line.starts_with(&format!("{}: ", file["path"].as_str().unwrap())))
        {
            path = file["path"].as_str().unwrap();
        } else if let Some(header) = line.strip_prefix("- ") {
            rule = header.split_whitespace().next().unwrap();
        } else if let Some(occurrence) = line.strip_prefix("  - L") {
            let line: u64 = occurrence.split(':').next().unwrap().parse().unwrap();
            actual.push((path.to_string(), rule.to_string(), line));
        }
    }
    expected.sort();
    actual.sort();
    assert_eq!(actual, expected, "{hook}");
}

#[test]
fn check_git_diff_keeps_only_changed_findings_and_matches_the_hook() {
    let Some(dir) = repository() else { return };
    append_paragraph(dir.path());
    let report = changed_report(dir.path(), &[]);
    assert_same_hook_findings(dir.path(), &report);
    assert_eq!(report["summary"]["diagnostics"], 1);
    let finding = &report["files"][0]["diagnostics"][0];
    assert_eq!(finding["ruleId"], "X01");
    assert_eq!(finding["range"]["start"]["line"], 7);

    let (code, hook, stderr) = output(noslop(dir.path()).args([
        "hook",
        "git-diff",
        "--max-chars",
        "unlimited",
        "--brief-limit",
        "unlimited",
    ]));
    assert_eq!(code, Some(1), "{stderr}");
    assert!(hook.contains("指摘を 1 件見つけました"), "{hook}");
    assert!(hook.contains("L7:") && !hook.contains("L3:"), "{hook}");
}

#[test]
fn check_git_diff_keeps_findings_whose_context_crosses_the_changed_line() {
    let Some(dir) = repository() else { return };
    let path = dir.path().join("docs/guide.md");
    fs::write(
        &path,
        "# 案内\n\nユーザー様に届けるため、\n毎月案内を書きます。\n",
    )
    .unwrap();
    assert!(git(dir.path(), &["add", "docs/guide.md"]));
    assert!(git(
        dir.path(),
        &["commit", "-q", "--no-verify", "-m", "paragraph"]
    ));
    fs::write(
        &path,
        "# 案内\n\nユーザー様に届けるため、\n毎月新しい案内を書きます。\n",
    )
    .unwrap();
    let report = changed_report(dir.path(), &[]);
    assert_same_hook_findings(dir.path(), &report);
    assert_eq!(report["summary"]["diagnostics"], 1);
    assert_eq!(
        report["files"][0]["diagnostics"][0]["range"]["start"]["line"],
        3
    );
}

#[test]
fn check_git_diff_distinguishes_omitted_paths_from_the_current_directory() {
    let Some(dir) = repository() else { return };
    append_paragraph(dir.path());
    fs::create_dir(dir.path().join("other")).unwrap();
    fs::write(dir.path().join("other/new.md"), "ユーザー様へ。\n").unwrap();
    let cwd = dir.path().join("docs");
    assert_eq!(changed_report(&cwd, &[])["summary"]["diagnostics"], 2);
    assert_eq!(changed_report(&cwd, &["."])["summary"]["diagnostics"], 1);
    assert_eq!(
        changed_report(&cwd, &["guide.md"])["summary"]["diagnostics"],
        1
    );
}

#[test]
fn check_git_diff_rejects_standard_input_and_non_repositories() {
    let Some(dir) = repository() else { return };
    let (code, _, stderr) = output(noslop(dir.path()).args(["check", "--git-diff", "-"]));
    assert_eq!(code, Some(2));
    assert!(stderr.contains("標準入力"), "{stderr}");
    let outside = workspace();
    if !git(outside.path(), &["rev-parse", "--git-dir"]) {
        let (code, _, stderr) = output(noslop(outside.path()).args(["check", "--git-diff"]));
        assert_eq!(code, Some(2));
        assert!(stderr.contains("作業ツリー"), "{stderr}");
    }
}

#[test]
fn check_git_diff_handles_staged_renamed_deleted_and_untracked_files() {
    let Some(dir) = repository() else { return };
    fs::write(dir.path().join("deleted.md"), "ユーザー様。\n").unwrap();
    assert!(git(dir.path(), &["add", "deleted.md"]));
    assert!(git(
        dir.path(),
        &["commit", "-q", "--no-verify", "-m", "tracked"]
    ));
    fs::remove_file(dir.path().join("deleted.md")).unwrap();
    assert!(git(
        dir.path(),
        &["mv", "docs/guide.md", "docs/名前 空白.md"]
    ));
    let path = dir.path().join("docs/名前 空白.md");
    let mut text = fs::read_to_string(&path).unwrap();
    text.push_str("\nユーザー様へ案内を送ります。\n");
    fs::write(&path, &text).unwrap();
    assert!(git(dir.path(), &["add", "docs/名前 空白.md"]));
    text.push_str("\nユーザー様から返事が届きました。\n");
    fs::write(&path, &text).unwrap();
    fs::write(dir.path().join("new.md"), "ユーザー様へ。\n").unwrap();
    fs::write(dir.path().join(".noslopignore"), "excluded.md\n").unwrap();
    fs::write(dir.path().join("excluded.md"), "ユーザー様。\n").unwrap();
    let report = changed_report(dir.path(), &[]);
    assert_same_hook_findings(dir.path(), &report);
    assert_eq!(report["summary"]["diagnostics"], 3);
    assert_eq!(report["files"][0]["path"], "docs/名前 空白.md");
    assert_eq!(
        report["files"][0]["diagnostics"].as_array().unwrap().len(),
        2
    );
    assert_eq!(report["files"][1]["path"], "new.md");
    assert_eq!(
        changed_report(dir.path(), &["excluded.md"])["summary"]["diagnostics"],
        0
    );
    assert_eq!(
        changed_report(dir.path(), &["deleted.md"])["summary"]["diagnostics"],
        0
    );

    let (code, hook, stderr) = output(noslop(dir.path()).args([
        "hook",
        "git-diff",
        "--max-chars",
        "unlimited",
        "--brief-limit",
        "unlimited",
    ]));
    assert_eq!(code, Some(1), "{stderr}");
    assert_eq!(
        hook.lines()
            .filter(|line| line.starts_with("  - L"))
            .count(),
        3,
        "{hook}"
    );
    assert!(
        !hook.contains("L3:") && !hook.contains("excluded.md"),
        "{hook}"
    );
}

#[test]
fn check_git_diff_before_the_first_commit_and_outside_scopes() {
    let dir = workspace();
    if !git(dir.path(), &["init", "-q"]) {
        return;
    }
    assert!(git(dir.path(), &["add", "docs/guide.md"]));
    assert_same_hook_findings(dir.path(), &changed_report(dir.path(), &[]));
    assert_eq!(changed_report(dir.path(), &[])["summary"]["diagnostics"], 1);
    let other = workspace();
    assert!(git(other.path(), &["init", "-q"]));
    let (code, _, stderr) = output(
        noslop(dir.path())
            .args(["check", "--git-diff"])
            .arg(other.path()),
    );
    assert_eq!(code, Some(2));
    assert!(stderr.contains("同じ git の作業ツリー"), "{stderr}");
}

/// フックが案内したコマンドを、別のディレクトリからそのまま実行する。
fn rerun(command: &str) -> (Option<i32>, String, String) {
    let mut cmd = StdCommand::new(if cfg!(windows) { "pwsh" } else { "sh" });
    if cfg!(windows) {
        // スクリプトブロックの成功と noslop の終了コードは別なので、pwsh のプロセスにも
        // ネイティブコマンドの終了コードを明示的に引き継ぐ。
        let script = format!("{command}; exit $LASTEXITCODE");
        cmd.args(["-NoProfile", "-NonInteractive", "-Command", &script]);
    } else {
        cmd.args(["-c", command]);
    }
    isolate_git(&mut cmd);
    let empty = empty_dir();
    let mut paths = vec![
        Path::new(env!("CARGO_BIN_EXE_noslop"))
            .parent()
            .unwrap()
            .to_path_buf(),
    ];
    paths.extend(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    ));
    cmd.current_dir(empty)
        .env("PATH", std::env::join_paths(paths).unwrap())
        .env("HOME", empty)
        .env("USERPROFILE", empty)
        .env("HASAMI_DATA_DIR", empty)
        .env_remove("HASAMI_DICT");
    output(&mut Command::from_std(cmd))
}

#[test]
fn truncated_stop_guidance_preserves_options_and_returns_every_occurrence() {
    let Some(dir) = repository() else { return };
    let config_name = "設定 '別名'.toml";
    fs::write(dir.path().join(config_name), CONFIG).unwrap();
    let text = "ユーザー様へ。\n\n".repeat(20);
    fs::write(dir.path().join("new.md"), text).unwrap();
    for include_readability in [false, true] {
        let mut args = vec![
            "hook",
            "git-diff",
            "--max-chars",
            "900",
            "--config",
            config_name,
            "--genre",
            "tech",
            "--experimental",
        ];
        if include_readability {
            args.push("--include-readability");
        }
        let (code, cut, stderr) = output(noslop(dir.path()).args(&args));
        assert_eq!(code, Some(1), "{stderr}");
        assert!(cut.chars().count() <= 900, "{cut}");
        let command = cut
            .split("全件は `")
            .nth(1)
            .unwrap()
            .split('`')
            .next()
            .unwrap();
        assert_eq!(command.contains("--no-readability"), !include_readability);
        assert!(
            command.contains("--genre tech") && command.contains("--experimental"),
            "{command}"
        );
        let (code, full, stderr) = rerun(command);
        assert_eq!(code, Some(0), "{stderr}: {command}");
        assert_eq!(
            full.split("X01 TEAM_TERM")
                .nth(1)
                .unwrap()
                .split("\n## ")
                .next()
                .unwrap()
                .lines()
                .filter(|line| line.starts_with("  - L"))
                .count(),
            20,
            "{full}"
        );
        assert!(!full.contains("ほか "), "{full}");
    }

    // Claude Code の Stop は入力の cwd が実行場所。プロセスの cwd は関係しない。
    let (code, _, stderr) = output(
        noslop(empty_dir())
            .args(["hook", "claude-code"])
            .write_stdin(stop_event(dir.path(), false)),
    );
    assert_eq!(code, Some(0), "{stderr}");
    // 既定 9000 文字で省略するだけの指摘を増やす。
    fs::write(dir.path().join("new.md"), "ユーザー様へ。\n\n".repeat(1000)).unwrap();
    let (_, response, _) = output(
        noslop(empty_dir())
            .args(["hook", "claude-code", "--brief-limit", "unlimited"])
            .write_stdin(stop_event(dir.path(), false)),
    );
    let response: Value = serde_json::from_str(&response).unwrap();
    let context = response["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap();
    if context.contains("全件は `") {
        let command = context
            .split("全件は `")
            .nth(1)
            .unwrap()
            .split('`')
            .next()
            .unwrap();
        let (code, full, stderr) = rerun(command);
        assert_eq!(code, Some(0), "{stderr}");
        assert_eq!(
            full.lines()
                .filter(|line| line.contains("ユーザー様へ。"))
                .count(),
            1000
        );
    } else {
        panic!("省略時の案内がありません: {context}");
    }
}

#[test]
fn whole_file_and_file_guidance_reproduce_the_original_hook() {
    let Some(dir) = repository() else { return };
    let path = "docs/案内 '別名'.md";
    assert!(git(dir.path(), &["mv", "docs/guide.md", path]));
    fs::write(
        dir.path().join(path),
        format!("{DOC}\n{}", "ユーザー様へ。\n\n".repeat(20)),
    )
    .unwrap();
    let config = "設定 '別名'.toml";
    fs::write(dir.path().join(config), CONFIG).unwrap();
    for kind in ["git-diff", "file"] {
        let run = |limit: &str| {
            let mut command = noslop(dir.path());
            command.args([
                "hook",
                kind,
                "--max-chars",
                limit,
                "--brief-limit",
                "unlimited",
                "--whole-file",
                "--config",
                config,
                "--include-readability",
                "--genre",
                "tech",
                "--experimental",
            ]);
            if kind == "file" {
                command.arg(path);
            }
            output(&mut command)
        };
        let (_, cut, stderr) = run("900");
        assert_eq!(stderr, "");
        let command = cut
            .split("全件は `")
            .nth(1)
            .unwrap()
            .split('`')
            .next()
            .unwrap();
        assert!(
            command.contains("--whole-file") && command.contains("--include-readability"),
            "{command}"
        );
        if kind == "file" {
            assert!(command.contains(" -- 'docs/"), "{command}");
        }
        let (code, full, stderr) = rerun(command);
        assert_eq!(code, Some(if kind == "file" { 0 } else { 1 }), "{stderr}");
        let (_, expected, _) = run("unlimited");
        assert_eq!(full, expected, "{command}");
    }
}

#[test]
fn stop_guidance_keeps_a_relative_config_from_the_process_directory() {
    let Some(dir) = repository() else { return };
    let config_dir = tempfile::tempdir().unwrap();
    let config = "別の設定.toml";
    fs::write(config_dir.path().join(config), CONFIG).unwrap();
    // イベントの cwd に同名の設定があっても、それは元の検査には使っていない。
    fs::write(dir.path().join(config), "[壊れた設定\n").unwrap();
    fs::write(dir.path().join("new.md"), "ユーザー様へ。\n\n".repeat(1000)).unwrap();
    let (code, response, stderr) = output(
        noslop(config_dir.path())
            .args([
                "hook",
                "claude-code",
                "--config",
                config,
                "--brief-limit",
                "unlimited",
            ])
            .write_stdin(stop_event(dir.path(), false)),
    );
    assert_eq!(code, Some(0), "{stderr}");
    let response: Value = serde_json::from_str(&response).unwrap();
    let context = response["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap();
    let command = context
        .split("全件は `")
        .nth(1)
        .unwrap()
        .split('`')
        .next()
        .unwrap();
    let (code, full, stderr) = rerun(command);
    assert_eq!(code, Some(0), "{stderr}: {command}");
    assert!(full.contains("独自ルール 1000 件"), "{full}");
    assert!(!full.contains("ほか "), "{full}");
}

#[test]
#[cfg(unix)]
fn git_diff_scope_resolves_parent_components_after_symbolic_links() {
    let Some(dir) = repository() else { return };
    fs::create_dir_all(dir.path().join("sub/deep")).unwrap();
    fs::write(dir.path().join("guide.md"), "ユーザー様へ。\n").unwrap();
    fs::write(
        dir.path().join("sub/guide.md"),
        "ユーザー様へ。\n\nユーザー様からの返事。\n",
    )
    .unwrap();
    std::os::unix::fs::symlink("sub/deep", dir.path().join("link")).unwrap();
    let report = changed_report(dir.path(), &["link/../guide.md"]);
    assert_eq!(report["files"][0]["path"], "sub/guide.md");
    assert_eq!(report["summary"]["diagnostics"], 2);
    fs::remove_file(dir.path().join("sub/guide.md")).unwrap();
    assert_eq!(
        changed_report(dir.path(), &["link/../guide.md"])["summary"]["diagnostics"],
        0
    );
}

#[test]
#[cfg(target_os = "linux")]
fn non_utf8_paths_with_the_same_display_name_are_each_checked() {
    use std::os::unix::ffi::OsStringExt;
    let Some(dir) = repository() else { return };
    let first = dir
        .path()
        .join(std::ffi::OsString::from_vec(b"invalid-\xfe.md".to_vec()));
    let second = dir
        .path()
        .join(std::ffi::OsString::from_vec(b"invalid-\xff.md".to_vec()));
    assert_ne!(first, second);
    assert_eq!(first.to_string_lossy(), second.to_string_lossy());
    for path in [&first, &second] {
        fs::write(path, DOC).unwrap();
    }
    assert!(git(dir.path(), &["add", "-A"]));
    assert!(git(
        dir.path(),
        &["commit", "-q", "--no-verify", "-m", "paths"]
    ));
    fs::write(&first, format!("{DOC}\nユーザー様へ。\n")).unwrap();
    fs::write(
        &second,
        format!("{DOC}\nユーザー様へ。\n\nユーザー様からの返事。\n"),
    )
    .unwrap();
    let report = changed_report(dir.path(), &[]);
    assert_eq!(report["summary"]["files"], 2);
    assert_eq!(report["summary"]["diagnostics"], 3);
    let lines: Vec<_> = report["files"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|file| {
            file["diagnostics"]
                .as_array()
                .unwrap()
                .iter()
                .map(|finding| finding["range"]["start"]["line"].as_u64().unwrap())
        })
        .collect();
    assert_eq!(lines, vec![7, 7, 9]);
}
