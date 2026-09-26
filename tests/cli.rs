//! CLI の統合テスト。
//!
//! 組み込みルールの増減に左右されないよう、件数を確かめるテストは設定ファイルの
//! 独自ルール (`X01`) と `--only-rules` を使う。

use std::fs;
use std::io::{self, BufRead, BufReader, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::thread;
use std::time::Duration;

use assert_cmd::Command;
use noslop::dictionaries::{DICTIONARIES, RECOMMENDED};
use predicates::prelude::*;
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

const DOC_WITH_TERM: &str =
    "# 利用案内\n\n新しい機能はユーザー様の声から生まれました。\n\n問題のない段落です。\n";
const DOC_CLEAN: &str = "# メモ\n\n今日は晴れた。散歩に出かけた。\n";

#[test]
fn new_editorial_rules_are_opt_in_and_report_their_lanes() {
    let dir = tempfile::tempdir().unwrap();
    let text = "おっしゃる通りです。必要であれば表も作成できます。\n\n専門家は有効だと指摘しています。\n\n申請書は提出前に担当者が記入漏れと添付資料の不足を確認してください。\n\n申請書は提出前に担当者が記入漏れと添付資料の不足を確認してください。\n\n操作は速く、柔軟で、直感的です。導入で効率、品質、成長を支えます。運用で信頼、安心、価値を届けます。\n\n運用には課題が残ります。しかし、今後の普及が期待されます。\n";
    for enabled in [false, true] {
        let mut cmd = noslop();
        cmd.current_dir(dir.path()).args([
            "check",
            "-",
            "--stdin-filename",
            "draft.md",
            "--no-config",
            "--no-dict",
            "--format",
            "json",
        ]);
        if enabled {
            cmd.arg("--experimental");
        }
        let out = cmd
            .write_stdin(text)
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        let report = json(&out);
        for (id, count) in [("P18", 2), ("P21", 1), ("R14", 1), ("R15", 1), ("R16", 3)] {
            assert_eq!(
                rule_count(&report, id),
                if enabled { count } else { 0 },
                "{id}"
            );
        }
        if enabled {
            let diagnostics = report["files"][0]["diagnostics"].as_array().unwrap();
            for d in diagnostics.iter().filter(|d| {
                ["P18", "P21", "R14", "R15", "R16"].contains(&d["ruleId"].as_str().unwrap())
            }) {
                assert_eq!(d["status"], "experimental");
                assert_eq!(d["severity"], "info");
                assert_eq!(
                    d["lane"],
                    if d["ruleId"] == "R14" {
                        "readability"
                    } else {
                        "slop"
                    }
                );
            }
            let duplicate = diagnostics.iter().find(|d| d["ruleId"] == "R14").unwrap();
            assert!(
                duplicate["message"]
                    .as_str()
                    .unwrap()
                    .contains("初出は 5 行 1 列")
            );
            assert_eq!(duplicate["related"][0]["start"]["line"], 5);
        }
    }
    // experimental 全体を有効にせず、個別指定でも動く。
    let out = noslop()
        .current_dir(dir.path())
        .args([
            "check",
            "-",
            "--stdin-filename",
            "draft.md",
            "--no-config",
            "--no-dict",
            "--format",
            "json",
            "--only-rules",
            "P21,R14,R15,R16",
        ])
        .write_stdin(text)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let report = json(&out);
    assert_eq!(rule_count(&report, "P18"), 0);
    assert_eq!(report["summary"]["diagnostics"], 6);
}

/// 手元の設定と辞書から切り離した noslop。ユーザーの設定 (`~/.config/noslop`) を読まないよう
/// HOME と USERPROFILE (Windows のホーム) を、auto が手元の辞書を選ばないよう share ディレクトリ
/// (HASAMI_DATA_DIR) を空のディレクトリにし、HASAMI_DICT を外す。テストはこれらを上書きしてよい。
fn noslop() -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_noslop"));
    let empty = empty_dir();
    cmd.env_remove("NO_COLOR")
        .env_remove("CLICOLOR_FORCE")
        .env("HOME", empty)
        .env("USERPROFILE", empty)
        .env("HASAMI_DATA_DIR", empty)
        .env_remove("HASAMI_DICT");
    cmd
}

/// 中に何も置かないディレクトリ (テストの実行ごとに 1 つ)。
fn empty_dir() -> &'static Path {
    static EMPTY: OnceLock<TempDir> = OnceLock::new();
    EMPTY.get_or_init(|| tempfile::tempdir().unwrap()).path()
}

/// 設定ファイルと 2 つの文書を置いた作業ディレクトリ。
fn workspace() -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("noslop.toml"), CONFIG).unwrap();
    fs::create_dir_all(dir.path().join("docs")).unwrap();
    fs::write(dir.path().join("docs/guide.md"), DOC_WITH_TERM).unwrap();
    fs::write(dir.path().join("docs/memo.md"), DOC_CLEAN).unwrap();
    dir
}

fn json(output: &[u8]) -> serde_json::Value {
    serde_json::from_slice(output).unwrap_or_else(|e| {
        panic!(
            "JSON として読めません: {e}\n{}",
            String::from_utf8_lossy(output)
        )
    })
}

fn x01_count(v: &serde_json::Value) -> usize {
    v["files"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|f| f["diagnostics"].as_array().unwrap().iter())
        .filter(|d| d["ruleId"] == "X01" && d["suppressed"].is_null())
        .count()
}

#[test]
fn prints_version() {
    noslop()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::starts_with(format!(
            "noslop {}",
            env!("CARGO_PKG_VERSION")
        )));
}

#[test]
fn check_reports_but_exits_zero_by_default() {
    let dir = workspace();
    noslop()
        .current_dir(dir.path())
        .args(["check", "--color", "never", "--only-rules", "X01"])
        .assert()
        .code(0)
        .stdout(predicate::str::contains("📄 docs/guide.md"))
        .stdout(predicate::str::contains("  独自ルールの指摘 1 件\n"))
        .stdout(predicate::str::contains(
            "3:7  警告  [独自ルール]  X01 TEAM_TERM",
        ))
        .stdout(predicate::str::contains("^^^^^^^^^^"))
        .stdout(predicate::str::contains(
            "💡 用語集の表記に合わせてください",
        ))
        .stdout(predicate::str::contains("📄 docs/memo.md"))
        .stdout(predicate::str::contains(
            "✖ AI 臭さの指摘 0 件、独自ルールの指摘 1 件 (警告 1) — 2 ファイルを検査",
        ));
}

#[test]
fn lint_alias_and_quiet_mode() {
    let dir = workspace();
    noslop()
        .current_dir(dir.path())
        .args([
            "lint",
            "-q",
            "--color",
            "never",
            "--only-rules",
            "X01",
            "docs",
        ])
        .assert()
        .code(0)
        .stdout(predicate::str::contains("docs/guide.md"))
        .stdout(predicate::str::contains("docs/memo.md").not())
        .stdout(predicate::str::contains("ファイルを検査").not());
}

#[test]
fn fail_on_controls_the_exit_code() {
    let dir = workspace();
    noslop()
        .current_dir(dir.path())
        .args(["check", "--fail-on", "warning", "--only-rules", "X01"])
        .assert()
        .code(1);
    noslop()
        .current_dir(dir.path())
        .args(["check", "--fail-on", "error", "--only-rules", "X01"])
        .assert()
        .code(0);
    // 設定ファイルの fail_on も効く
    fs::write(
        dir.path().join("noslop.toml"),
        format!("fail_on = \"warning\"\n{CONFIG}"),
    )
    .unwrap();
    noslop()
        .current_dir(dir.path())
        .args(["check", "--only-rules", "X01"])
        .assert()
        .code(1);
}

#[test]
fn missing_paths_are_errors_but_other_files_are_still_checked() {
    let dir = workspace();
    noslop()
        .current_dir(dir.path())
        .args([
            "check",
            "--color",
            "never",
            "--only-rules",
            "X01",
            "docs",
            "missing.md",
        ])
        .assert()
        .code(2)
        .stderr(predicate::str::contains(
            "missing.md: ファイルまたはディレクトリが見つかりません",
        ))
        .stdout(predicate::str::contains("X01 TEAM_TERM"));
}

#[test]
fn json_output_has_a_stable_schema() {
    let dir = workspace();
    let out = noslop()
        .current_dir(dir.path())
        .args(["check", "--format", "json", "--only-rules", "X01"])
        .assert()
        .code(0)
        .get_output()
        .stdout
        .clone();
    let v = json(&out);
    assert_eq!(v["schemaVersion"], 2);
    assert_eq!(v["tool"]["name"], "noslop");
    assert_eq!(v["columnUnit"], "unicode-scalar");
    assert_eq!(v["settings"]["genre"], "general");
    let files = v["files"].as_array().unwrap();
    assert_eq!(files.len(), 2);
    assert_eq!(files[0]["path"], "docs/guide.md");
    assert_eq!(files[0]["format"], "markdown");
    let d = &files[0]["diagnostics"][0];
    assert_eq!(d["ruleId"], "X01");
    assert_eq!(d["ruleName"], "TEAM_TERM");
    assert_eq!(d["lane"], "custom");
    assert_eq!(d["range"]["start"]["line"], 3);
    assert_eq!(d["range"]["start"]["column"], 7);
    assert_eq!(d["excerpt"], "新しい機能はユーザー様の声から生まれました。");
    assert_eq!(d["metrics"]["matched"], "ユーザー様");
    assert!(d["fingerprint"].as_str().unwrap().len() == 16);
    assert_eq!(v["summary"]["diagnostics"], 1);
    assert_eq!(v["summary"]["byLane"]["custom"], 1);
    assert_eq!(v["errors"].as_array().unwrap().len(), 0);
}

#[test]
fn github_output_emits_annotations() {
    let dir = workspace();
    noslop()
        .current_dir(dir.path())
        .args(["check", "-f", "github-actions", "--only-rules", "X01"])
        .assert()
        .code(0)
        .stdout(predicate::str::starts_with(
            "::warning file=docs/guide.md,line=3,col=7,endLine=3,endColumn=12,title=[独自ルール] X01 TEAM_TERM::「ユーザー様」ではなく「利用者」と書きます%0A💡 用語集の表記に合わせてください",
        ));
}

#[test]
fn reads_stdin_with_a_filename_for_format_detection() {
    let dir = workspace();
    let out = noslop()
        .current_dir(dir.path())
        .args([
            "check",
            "-",
            "--stdin-filename",
            "memo.txt",
            "--format",
            "json",
            "--only-rules",
            "X01",
        ])
        .write_stdin("　ユーザー様へのお知らせです。\n")
        .assert()
        .code(0)
        .get_output()
        .stdout
        .clone();
    let v = json(&out);
    assert_eq!(v["files"][0]["path"], "memo.txt");
    assert_eq!(v["files"][0]["format"], "text");
    assert_eq!(x01_count(&v), 1);

    noslop()
        .current_dir(dir.path())
        .args(["check", "-", "--color", "never", "--only-rules", "X01"])
        .write_stdin("ユーザー様です。\n")
        .assert()
        .code(0)
        .stdout(predicate::str::contains("📄 <stdin>"));
}

#[test]
fn config_is_discovered_from_parent_directories_and_can_be_skipped() {
    let dir = workspace();
    let nested = dir.path().join("docs");
    let out = noslop()
        .current_dir(&nested)
        .args([
            "check",
            "guide.md",
            "--format",
            "json",
            "--only-rules",
            "X01",
        ])
        .assert()
        .code(0)
        .get_output()
        .stdout
        .clone();
    assert_eq!(x01_count(&json(&out)), 1);

    // --no-config なら独自ルールは存在しない
    noslop()
        .current_dir(&nested)
        .args(["check", "guide.md", "--no-config", "--only-rules", "X01"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("未知のルール"));

    // --config で別の設定を指定できる
    let other = dir.path().join("other.toml");
    fs::write(&other, CONFIG.replace("ユーザー様", "声")).unwrap();
    let out = noslop()
        .current_dir(&nested)
        .args([
            "check",
            "guide.md",
            "--format",
            "json",
            "--only-rules",
            "X01",
            "--config",
        ])
        .arg(&other)
        .assert()
        .code(0)
        .get_output()
        .stdout
        .clone();
    let v = json(&out);
    assert_eq!(v["files"][0]["diagnostics"][0]["metrics"]["matched"], "声");
}

#[test]
fn exclude_and_extensions_in_config() {
    let dir = workspace();
    fs::write(
        dir.path().join("noslop.toml"),
        format!("[files]\nextensions = [\"md\", \"text\"]\nexclude = [\"docs/memo.md\"]\n{CONFIG}"),
    )
    .unwrap();
    fs::write(dir.path().join("notes.text"), "ユーザー様。\n").unwrap();
    fs::write(dir.path().join("skip.txt"), "ユーザー様。\n").unwrap();
    let out = noslop()
        .current_dir(dir.path())
        .args(["check", "--format", "json", "--only-rules", "X01"])
        .assert()
        .code(0)
        .get_output()
        .stdout
        .clone();
    let v = json(&out);
    let paths: Vec<_> = v["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["path"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(paths, vec!["docs/guide.md", "notes.text"]);
}

#[test]
fn suppression_comments_keep_a_record() {
    let dir = workspace();
    fs::write(
        dir.path().join("docs/guide.md"),
        "<!-- noslop-disable-next-line X01 -- 引用のため原文どおり -->\nユーザー様の声を引用する。\n\nユーザー様と書いてしまった。\n",
    )
    .unwrap();
    let out = noslop()
        .current_dir(dir.path())
        .args([
            "check",
            "docs/guide.md",
            "--format",
            "json",
            "--only-rules",
            "X01",
        ])
        .assert()
        .code(0)
        .get_output()
        .stdout
        .clone();
    let v = json(&out);
    let diags = v["files"][0]["diagnostics"].as_array().unwrap();
    assert_eq!(diags.len(), 2);
    assert_eq!(diags[0]["suppressed"]["reason"], "引用のため原文どおり");
    assert_eq!(diags[0]["suppressed"]["line"], 1);
    assert!(diags[1]["suppressed"].is_null());
    assert_eq!(v["summary"]["suppressed"], 1);

    noslop()
        .current_dir(dir.path())
        .args([
            "check",
            "docs/guide.md",
            "--color",
            "never",
            "--only-rules",
            "X01",
            "--show-suppressed",
        ])
        .assert()
        .code(0)
        .stdout(predicate::str::contains(
            "↳ 抑制済み (L1: 引用のため原文どおり)",
        ));
}

#[test]
fn invalid_config_and_unknown_rules_are_errors() {
    let dir = workspace();
    noslop()
        .current_dir(dir.path())
        .args(["check", "--ignore-rules", "NOPE"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("NOPE"));
    fs::write(dir.path().join("noslop.toml"), "unknown_key = 1\n").unwrap();
    noslop()
        .current_dir(dir.path())
        .arg("check")
        .assert()
        .code(2)
        .stderr(predicate::str::contains(
            "設定ファイルの書式が正しくありません",
        ));
    fs::write(
        dir.path().join("noslop.toml"),
        "[rules.X]\nseverity = \"loud\"\n",
    )
    .unwrap();
    noslop()
        .current_dir(dir.path())
        .arg("check")
        .assert()
        .code(2)
        .stderr(predicate::str::contains("severity"));
}

#[test]
fn rules_lists_builtin_and_custom_rules() {
    let dir = workspace();
    noslop()
        .current_dir(dir.path())
        .arg("rules")
        .assert()
        .success()
        .stdout(predicate::str::contains("X01"))
        .stdout(predicate::str::contains("TEAM_TERM"));
    let out = noslop()
        .current_dir(dir.path())
        .args(["rules", "--format", "json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v = json(&out);
    let x01 = v["rules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == "X01")
        .unwrap();
    assert_eq!(x01["builtin"], false);
    assert_eq!(x01["enabled"], true);
    assert_eq!(x01["lane"], "custom");
    noslop()
        .current_dir(dir.path())
        .args(["rules", "--format", "markdown"])
        .assert()
        .success()
        .stdout(predicate::str::starts_with("# noslop のルール"))
        .stdout(predicate::str::contains("## X01"));
}

#[test]
fn explain_shows_rule_details_and_rejects_unknown_rules() {
    let dir = workspace();
    noslop()
        .current_dir(dir.path())
        .args(["explain", "team_term"])
        .assert()
        .success()
        .stdout(predicate::str::starts_with("X01 TEAM_TERM"))
        .stdout(predicate::str::contains("独自ルール"));
    noslop()
        .current_dir(dir.path())
        .args(["explain", "NOPE"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("未知のルールです: NOPE"));
}

#[test]
fn init_writes_a_template_once() {
    let dir = tempfile::tempdir().unwrap();
    noslop()
        .current_dir(dir.path())
        .arg("init")
        .assert()
        .success()
        .stdout(predicate::str::contains("noslop.toml を作成しました"));
    let written = fs::read_to_string(dir.path().join("noslop.toml")).unwrap();
    assert!(written.contains("[[custom]]"));
    noslop()
        .current_dir(dir.path())
        .arg("init")
        .assert()
        .code(2)
        .stderr(predicate::str::contains("--force"));
    noslop()
        .current_dir(dir.path())
        .args(["init", "--force"])
        .assert()
        .success();
    // 生成したひな形はそのまま読める
    noslop()
        .current_dir(dir.path())
        .args(["check", "--color", "never"])
        .assert()
        .code(0);
}

/// `init --user` は、ホームディレクトリの `.config/noslop/config.toml` にユーザーの設定のひな形を
/// 書く (ディレクトリも作る)。プロジェクトの `noslop.toml` は作らない。
#[test]
fn init_user_writes_the_user_config_template() {
    let home = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let init = |extra: &[&str]| {
        let mut cmd = noslop();
        cmd.env("HOME", home.path())
            .env("USERPROFILE", home.path())
            .current_dir(work.path())
            .args(["init", "--user"])
            .args(extra);
        cmd
    };
    init(&[])
        .assert()
        .success()
        .stdout(predicate::str::contains("config.toml を作成しました"));
    let path = home
        .path()
        .join(".config")
        .join("noslop")
        .join("config.toml");
    let written = fs::read_to_string(&path).unwrap();
    assert!(written.contains("ユーザーの設定"), "{written}");
    assert!(written.contains("# dictionary = \"auto\""), "{written}");
    assert!(!work.path().join("noslop.toml").exists());

    // すでにあれば止まり、--force で上書きする
    fs::write(&path, "genre = \"tech\"\n").unwrap();
    init(&[])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("--force"));
    assert_eq!(fs::read_to_string(&path).unwrap(), "genre = \"tech\"\n");
    init(&["--force"]).assert().success();
    assert_eq!(fs::read_to_string(&path).unwrap(), written);

    // 作ったひな形はそのまま読める (ユーザーの設定として読み込んでも、既定値から何も変えない)
    fs::write(work.path().join("doc.md"), DOC_CLEAN).unwrap();
    noslop()
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .current_dir(work.path())
        .args(["check", "--color", "never", "doc.md"])
        .assert()
        .code(0);
}

#[test]
fn unreadable_files_are_reported_with_exit_code_two() {
    let dir = workspace();
    fs::write(dir.path().join("docs/broken.md"), [0xff, 0xfe, 0x00, 0x41]).unwrap();
    let out = noslop()
        .current_dir(dir.path())
        .args(["check", "--format", "json", "--only-rules", "X01"])
        .assert()
        .code(2)
        .get_output()
        .stdout
        .clone();
    let v = json(&out);
    assert_eq!(v["errors"][0]["path"], "docs/broken.md");
    assert_eq!(v["files"].as_array().unwrap().len(), 2);
}

#[test]
fn explicit_files_ignore_extension_filters() {
    let dir = workspace();
    let odd = dir.path().join("draft.text");
    fs::write(&odd, "ユーザー様。\n").unwrap();
    let out = noslop()
        .current_dir(dir.path())
        .args(["check", "--format", "json", "--only-rules", "X01"])
        .arg(Path::new("draft.text"))
        .assert()
        .code(0)
        .get_output()
        .stdout
        .clone();
    assert_eq!(x01_count(&json(&out)), 1);
}

#[test]
fn json_offsets_count_a_leading_bom() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("noslop.toml"), CONFIG).unwrap();
    let src = "\u{FEFF}# 見出し\n\nこれはユーザー様の声です。\n";
    fs::write(dir.path().join("bom.md"), src).unwrap();
    let out = noslop()
        .current_dir(dir.path())
        .args(["check", "--format", "json", "--only-rules", "X01", "bom.md"])
        .output()
        .unwrap();
    let v = json(&out.stdout);
    let start = &v["files"][0]["diagnostics"][0]["range"]["start"];
    // オフセットはファイル上のバイト位置 (BOM を含む)、列は BOM を数えない
    assert_eq!(start["offset"], src.find("ユーザー様").unwrap());
    assert_eq!(start["line"], 3);
    assert_eq!(start["column"], 4);
}

/// 設定ファイル (X01) と Markdown の文書 1 つを置いて検査し、X01 の指摘の
/// (行, 列, ファイル上のオフセット) を返す。
fn x01_positions(src: &str) -> Vec<(u64, u64, u64)> {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("noslop.toml"), CONFIG).unwrap();
    fs::write(dir.path().join("doc.md"), src).unwrap();
    let out = noslop()
        .current_dir(dir.path())
        .args(["check", "--format", "json", "--only-rules", "X01", "doc.md"])
        .output()
        .unwrap();
    json(&out.stdout)["files"][0]["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| {
            let start = &d["range"]["start"];
            let field = |key: &str| start[key].as_u64().unwrap();
            (field("line"), field("column"), field("offset"))
        })
        .collect()
}

#[test]
fn front_matter_at_the_top_is_skipped_and_positions_stay_exact() {
    // front matter の中の「ユーザー様」は数えない (... で閉じる形なので、読んでしまうと
    // 段落になって指摘が出る)。BOM の直後の front matter も先頭のものとして扱う
    let src = "\u{FEFF}---\ntitle: ユーザー様の声\n...\n\n# 見出し\n\nこれはユーザー様の声です。\n";
    let offset = src.rfind("ユーザー様").unwrap() as u64;
    assert_eq!(x01_positions(src), [(7, 4, offset)]);
}

#[test]
fn marker_lines_in_the_middle_are_not_front_matter() {
    // 段落の直後でない --- の行から、次の --- か ... の行までが front matter として
    // 検査から漏れていた。閉じのつもりの --- の直前の行 (1 つ目と 2 つ目の 6 行目) は
    // 下線形式の見出しになるので、語句のルールは見ない
    let cases = [
        (
            "一つ目はユーザー様の声です。\n\n---\n二つ目はユーザー様の声です。\n\n\
             三つ目はユーザー様の声です。\n---\n\n四つ目はユーザー様の声です。\n",
            vec![1, 4, 9],
        ),
        // 1 つ目の --- の後に空行がある
        (
            "一つ目はユーザー様の声です。\n\n---\n\n二つ目はユーザー様の声です。\n\n\
             三つ目はユーザー様の声です。\n---\n\n四つ目はユーザー様の声です。\n",
            vec![1, 5, 10],
        ),
        // 閉じの行がない
        (
            "一つ目はユーザー様の声です。\n\n---\n二つ目はユーザー様の声です。\n\n\
             三つ目はユーザー様の声です。\n\n四つ目はユーザー様の声です。\n",
            vec![1, 4, 6, 8],
        ),
        // ... の行で閉じる形
        (
            "一つ目はユーザー様の声です。\n\n---\n二つ目はユーザー様の声です。\n\n\
             三つ目はユーザー様の声です。\n...\n\n四つ目はユーザー様の声です。\n",
            vec![1, 4, 6, 9],
        ),
    ];
    for (src, lines) in cases {
        let found: Vec<u64> = x01_positions(src).iter().map(|p| p.0).collect();
        assert_eq!(found, lines, "{src:?}");
    }
}

#[test]
fn directive_examples_inside_ordinary_comments_do_not_suppress() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("noslop.toml"), CONFIG).unwrap();
    fs::write(
        dir.path().join("doc.md"),
        "<!-- 例: <!-- noslop-disable-file --> -->\n\nこれはユーザー様の声です。\n",
    )
    .unwrap();
    let out = noslop()
        .current_dir(dir.path())
        .args(["check", "--format", "json", "doc.md"])
        .output()
        .unwrap();
    assert_eq!(x01_count(&json(&out.stdout)), 1);
}

#[test]
fn builtin_rules_separate_the_bundled_examples() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let run = |file: &str| {
        let out = noslop()
            .current_dir(root)
            .args(["check", "--no-config", "--format", "json", file])
            .output()
            .unwrap();
        json(&out.stdout)["files"][0].clone()
    };
    let smelly = run("examples/ai-smelly.md");
    let ids: Vec<&str> = smelly["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["ruleId"].as_str().unwrap())
        .collect();
    for id in ["P01", "P02", "R05"] {
        assert!(ids.contains(&id), "{id} が出ていない: {ids:?}");
    }
    // 文書全体の点数は出さず、AI 臭さの指摘をレーン・重大度ごとに数える
    assert!(smelly.get("score").is_none(), "{smelly}");
    let slop = &smelly["counts"]["slop"];
    let serious = slop["error"].as_u64().unwrap() + slop["warning"].as_u64().unwrap();
    assert!(serious >= 5, "重大と警告が少ない: {slop}");

    let natural = run("examples/natural.md");
    assert_eq!(natural["diagnostics"].as_array().unwrap().len(), 0);
    for lane in ["slop", "readability", "custom"] {
        for severity in ["error", "warning", "info"] {
            assert_eq!(natural["counts"][lane][severity], 0, "{lane} {severity}");
        }
    }
}

#[test]
fn diff_reports_new_findings_and_lost_facts() {
    let dir = workspace();
    fs::write(
        dir.path().join("before.md"),
        "# 案内\n\n2024 年に導入した機能は、利用者の声から生まれました。\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("after.md"),
        "# 案内\n\n新しい機能は、ユーザー様の声から生まれました。\n",
    )
    .unwrap();
    let out = noslop()
        .current_dir(dir.path())
        .args([
            "diff",
            "before.md",
            "after.md",
            "--format",
            "json",
            "--only-rules",
            "X01",
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "確認事項があっても 0 で終わる");
    let v = json(&out.stdout);
    assert_eq!(v["kind"], "diff");
    assert_eq!(v["hasConcerns"], true);
    assert_eq!(v["findings"]["summary"]["new"], 1);
    assert_eq!(v["findings"]["new"][0]["ruleId"], "X01");
    let removed: Vec<&str> = v["facts"]["removed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["text"].as_str().unwrap())
        .collect();
    assert!(
        removed.iter().any(|t| t.contains("2024")),
        "消えた数値が出ていない: {removed:?}"
    );

    // text 形式 (色なし) でも同じ内容を出す
    noslop()
        .current_dir(dir.path())
        .args([
            "diff",
            "before.md",
            "after.md",
            "--color",
            "never",
            "--only-rules",
            "X01",
        ])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("before.md → after.md")
                .and(predicate::str::contains("X01"))
                .and(predicate::str::contains("\u{1b}[").not()),
        );
}

#[test]
fn diff_reads_one_side_from_stdin() {
    let dir = workspace();
    fs::write(
        dir.path().join("after.md"),
        "# 案内\n\n新しい機能は、ユーザー様の声から生まれました。\n",
    )
    .unwrap();
    let out = noslop()
        .current_dir(dir.path())
        .args([
            "diff",
            "-",
            "after.md",
            "--stdin-filename",
            "old.md",
            "--format",
            "json",
            "--only-rules",
            "X01",
        ])
        .write_stdin("# 案内\n\n新しい機能は、ユーザー様の声から生まれました。\n")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let v = json(&out.stdout);
    assert_eq!(v["before"]["path"], "old.md");
    assert_eq!(v["findings"]["summary"]["new"], 0);
    assert_eq!(v["findings"]["summary"]["persisting"], 1);
    assert_eq!(v["hasConcerns"], false);

    noslop()
        .current_dir(dir.path())
        .args(["diff", "-", "-"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("どちらか一方"));
    noslop()
        .current_dir(dir.path())
        .args(["diff", "missing.md", "after.md"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("missing.md"));
}

#[test]
fn calibrate_measures_rates_on_a_corpus() {
    let dir = workspace();
    for (group, text) in [
        ("human", "# 記録\n\n利用者の声を聞いて直した。\n"),
        ("ai", "# 記録\n\nユーザー様の声を聞いて直しました。\n"),
    ] {
        let base = dir.path().join("corpus").join(group);
        fs::create_dir_all(&base).unwrap();
        for i in 0..3 {
            fs::write(base.join(format!("doc{i}.md")), text).unwrap();
        }
    }
    let out = noslop()
        .current_dir(dir.path())
        .args([
            "calibrate",
            "--human",
            "corpus/human",
            "--ai",
            "corpus/ai",
            "--format",
            "json",
        ])
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v = json(&out.stdout);
    let x01 = v["rules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == "X01")
        .expect("独自ルールも測る");
    assert_eq!(x01["human"]["fired"], 0);
    assert_eq!(x01["ai"]["fired"], 3);
    assert!(
        !v["warnings"].as_array().unwrap().is_empty(),
        "件数が少ないことを警告する"
    );

    // markdown 形式は記録用の見出しから始まる
    noslop()
        .current_dir(dir.path())
        .args([
            "calibrate",
            "--human",
            "corpus/human",
            "--ai",
            "corpus/ai",
            "-f",
            "markdown",
        ])
        .assert()
        .success()
        .stdout(predicate::str::starts_with("#"));
}

#[test]
fn calibrate_reports_unreadable_inputs_and_invalid_options() {
    let dir = workspace();
    fs::create_dir_all(dir.path().join("human")).unwrap();
    fs::write(dir.path().join("human/a.md"), DOC_CLEAN).unwrap();
    noslop()
        .current_dir(dir.path())
        .args(["calibrate", "--human", "human", "--ai", "missing"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("missing"));
    noslop()
        .current_dir(dir.path())
        .args([
            "calibrate",
            "--human",
            "human",
            "--ai",
            "human",
            "--target-fp",
            "2",
        ])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("--target-fp"));
}

#[test]
fn directories_without_files_are_warned_about() {
    let dir = workspace();
    fs::create_dir_all(dir.path().join("corpus/human")).unwrap();
    fs::write(dir.path().join("corpus/human/a.md"), DOC_CLEAN).unwrap();
    // `dir/**` はファイルに当たるので、指定したディレクトリの中身まで除外される
    fs::write(dir.path().join(".gitignore"), "/corpus/**\n").unwrap();
    noslop()
        .current_dir(dir.path())
        .args(["check", "corpus/human", "--color", "never"])
        .assert()
        .success()
        .stderr(predicate::str::contains(
            "corpus/human には検査するファイルがありません",
        ));
    // ファイルのあるディレクトリでは何も言わない
    noslop()
        .current_dir(dir.path())
        .args(["check", "docs", "--color", "never"])
        .assert()
        .success()
        .stderr(predicate::str::contains("検査するファイルがありません").not());
}

#[test]
fn brief_report_comes_as_json_or_toon() {
    let dir = workspace();
    let out = noslop()
        .current_dir(dir.path())
        .args([
            "check",
            "docs",
            "--only-rules",
            "X01",
            "--report",
            "brief",
            "--format",
            "json",
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let v = json(&out.stdout);
    assert_eq!(v["kind"], "brief");
    assert_eq!(v["revisionRules"].as_array().unwrap().len(), 6);
    let file = &v["files"][0];
    assert_eq!(file["path"], "docs/guide.md");
    assert_eq!(file["rules"][0]["ruleId"], "X01");
    assert_eq!(file["rules"][0]["hint"], "用語集の表記に合わせてください");
    assert_eq!(file["occurrences"][0]["ruleId"], "X01");
    assert_eq!(file["occurrences"][0]["line"], 3);
    assert_eq!(v["cleanFiles"], serde_json::json!(["docs/memo.md"]));

    let out = noslop()
        .current_dir(dir.path())
        .args([
            "check",
            "docs",
            "--only-rules",
            "X01",
            "--report",
            "brief",
            "--format",
            "toon",
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(
        text.starts_with("schemaVersion: 1\nkind: brief\n"),
        "{text}"
    );
    assert!(
        text.contains("occurrences[1]{ruleId,line,column,message,excerpt}:"),
        "{text}"
    );
    assert!(!text.ends_with('\n'), "TOON は末尾に改行を付けない");
}

#[test]
fn full_report_comes_as_toon_and_bad_combinations_are_errors() {
    let dir = workspace();
    noslop()
        .current_dir(dir.path())
        .args(["check", "docs", "--only-rules", "X01", "--format", "toon"])
        .assert()
        .success()
        .stdout(
            predicate::str::starts_with("schemaVersion: 2\n")
                .and(predicate::str::contains("diagnostics["))
                .and(predicate::str::contains("TEAM_TERM")),
        );
    for args in [
        ["--report", "full", "--format", "markdown"],
        ["--report", "brief", "--format", "github"],
    ] {
        noslop()
            .current_dir(dir.path())
            .arg("check")
            .args(args)
            .assert()
            .code(2)
            .stderr(predicate::str::contains("エラー"));
    }
    // --format brief は --report brief --format markdown の省略形
    noslop()
        .current_dir(dir.path())
        .args(["check", "docs", "--only-rules", "X01", "--format", "brief"])
        .assert()
        .success()
        .stdout(predicate::str::starts_with("# noslop の改稿指示"));
}

#[test]
fn diff_comes_as_toon() {
    let dir = workspace();
    fs::write(
        dir.path().join("before.md"),
        "# 案内\n\n2024 年に導入した。\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("after.md"),
        "# 案内\n\n新しい機能はユーザー様の声から生まれた。\n",
    )
    .unwrap();
    noslop()
        .current_dir(dir.path())
        .args([
            "diff",
            "before.md",
            "after.md",
            "--only-rules",
            "X01",
            "-f",
            "toon",
        ])
        .assert()
        .success()
        .stdout(
            predicate::str::starts_with("schemaVersion: 2\nkind: diff\n")
                .and(predicate::str::contains("hasConcerns: true")),
        );
}

#[test]
fn skill_install_writes_the_skill_for_each_agent() {
    let home = tempfile::tempdir().unwrap();
    for (target, config) in [("claude", ".claude"), ("codex", ".codex")] {
        noslop()
            .env("HOME", home.path())
            .env("USERPROFILE", home.path())
            .args(["skill-install", target])
            .assert()
            .success()
            .stdout(predicate::str::contains("noslop のスキルを"));
        let skill = home
            .path()
            .join(config)
            .join("skills")
            .join("noslop")
            .join("SKILL.md");
        let text = fs::read_to_string(&skill).unwrap();
        assert!(text.starts_with("---\nname: noslop\n"), "{target}");
    }

    // --dir でプロジェクトなどの置き場を指定できる
    let project = tempfile::tempdir().unwrap();
    let skills = project.path().join(".claude").join("skills");
    noslop()
        .args(["skill-install", "claude-code", "--dir"])
        .arg(&skills)
        .assert()
        .success();
    assert!(skills.join("noslop").join("SKILL.md").is_file());

    noslop().args(["skill-install", "cursor"]).assert().code(2);
}

// ---------------------------------------------------------------------------
// 形態素解析の辞書
// ---------------------------------------------------------------------------

/// 「の」の連鎖を品詞で数えるのに要る語だけの辞書 (hasami の .hsd) を書き出す。
fn write_dictionary(path: &Path) {
    use hasami::DictEntry;
    use hasami::dict::DictBuilder;

    let mut builder = DictBuilder::new();
    for (surface, pos) in [
        ("俺", "名詞,代名詞,一般,*"),
        ("の", "助詞,連体化,*,*"),
        ("魂", "名詞,一般,*,*"),
        ("安静", "名詞,形容動詞語幹,*,*"),
        ("ため", "名詞,非自立,副詞可能,*"),
        ("に", "助詞,格助詞,一般,*"),
        ("祈る", "動詞,自立,*,*"),
        ("。", "記号,句点,*,*"),
    ] {
        builder.add_entry(DictEntry {
            surface: surface.into(),
            left_id: 1,
            right_id: 1,
            cost: 1000 - 10 * surface.chars().count() as i16,
            pos: pos.into(),
            base_form: surface.into(),
            ..Default::default()
        });
    }
    builder
        .write_hsd(path, &builder.write_options(), |_, _| {})
        .unwrap();
}

/// 辞書なしでは「ため」で連鎖が切れて拾えず、辞書ありでは拾う文書。
const DOC_NO_CHAIN: &str = "# メモ\n\n俺の魂の安静のために祈る。\n";

fn rule_count(v: &serde_json::Value, id: &str) -> usize {
    v["files"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|f| f["diagnostics"].as_array().unwrap().iter())
        .filter(|d| d["ruleId"] == id)
        .count()
}

/// 辞書を探す場所を空にした noslop (手元に入れた辞書に左右されないように)。
fn noslop_without_installed_dictionary(empty: &Path) -> Command {
    let mut cmd = noslop();
    cmd.env_remove("HASAMI_DICT")
        .env_remove("HASAMI_DATA_DIR")
        .env("XDG_DATA_HOME", empty);
    cmd
}

#[test]
fn a_dictionary_switches_part_of_speech_rules_to_the_precise_method() {
    let dir = tempfile::tempdir().unwrap();
    let dict = dir.path().join("test.hsd");
    write_dictionary(&dict);
    let doc = dir.path().join("doc.md");
    fs::write(&doc, DOC_NO_CHAIN).unwrap();
    let base = [
        "check",
        "--no-config",
        "--only-rules",
        "P16",
        "--format",
        "json",
    ];

    let out = noslop()
        .args(base)
        .arg("--no-dict")
        .arg(&doc)
        .output()
        .unwrap();
    let v = json(&out.stdout);
    assert_eq!(v["settings"]["morphology"]["method"], "surface");
    assert_eq!(v["settings"]["morphology"]["reason"], "disabled");
    assert_eq!(rule_count(&v, "P16"), 0);

    let out = noslop()
        .args(base)
        .arg("--dict")
        .arg(&dict)
        .arg(&doc)
        .output()
        .unwrap();
    let v = json(&out.stdout);
    let morphology = &v["settings"]["morphology"];
    assert_eq!(morphology["requested"], "required");
    assert_eq!(morphology["method"], "dictionary");
    assert_eq!(morphology["dictionary"]["source"], "file");
    assert!(
        morphology["dictionary"]["path"]
            .as_str()
            .unwrap()
            .ends_with("test.hsd")
    );
    assert_eq!(rule_count(&v, "P16"), 1);

    // 改稿指示には方式と辞書の名前だけを載せ、手元のパスは渡さない
    let out = noslop()
        .args([
            "check",
            "--no-config",
            "--only-rules",
            "P16",
            "--report",
            "brief",
        ])
        .args(["--format", "json", "--dict"])
        .arg(&dict)
        .arg(&doc)
        .output()
        .unwrap();
    let v = json(&out.stdout);
    assert_eq!(v["settings"]["method"], "dictionary");
    assert!(!v["settings"].to_string().contains("test.hsd"));

    // text は集計の行に方式を添える
    noslop()
        .args(["check", "--no-config", "--only-rules", "P16", "--dict"])
        .arg(&dict)
        .arg(&doc)
        .assert()
        .success()
        .stdout(predicate::str::contains("辞書あり"));
}

#[test]
fn dictionary_settings_are_checked_and_auto_falls_back() {
    let dir = tempfile::tempdir().unwrap();
    let empty = tempfile::tempdir().unwrap();
    let doc = dir.path().join("doc.md");
    fs::write(&doc, DOC_NO_CHAIN).unwrap();

    // 指定した辞書が読めなければ設定の誤り
    noslop()
        .args(["check", "--no-config", "--dict", "missing.hsd"])
        .arg(&doc)
        .assert()
        .code(2)
        .stderr(predicate::str::contains("辞書"));
    // --dict と --no-dict は同時に使えない
    noslop()
        .args(["check", "--no-config", "--dict", "x.hsd", "--no-dict"])
        .arg(&doc)
        .assert()
        .code(2);

    // HASAMI_DICT で指定した辞書が読めなければ、同梱の辞書に黙って切り替えず設定の誤り
    noslop()
        .env("HASAMI_DICT", dir.path().join("missing.hsd"))
        .args(["check", "--no-config", "--only-rules", "P16"])
        .arg(&doc)
        .assert()
        .code(2)
        .stderr(predicate::str::contains("missing.hsd"));

    // 指定がなく share ディレクトリに辞書もなければ、同梱の辞書を使う (同梱しないビルドでは辞書なしの
    // 近似に戻る)。share ディレクトリの辞書を選ぶ場合は auto_uses_the_best_dictionary_in_the_share_directory
    let out = noslop_without_installed_dictionary(empty.path())
        .args([
            "check",
            "--no-config",
            "--only-rules",
            "P16",
            "--format",
            "json",
        ])
        .arg(&doc)
        .output()
        .unwrap();
    let v = json(&out.stdout);
    let morphology = &v["settings"]["morphology"];
    assert_eq!(morphology["requested"], "auto");
    if cfg!(feature = "bundled-dict") {
        assert_eq!(morphology["method"], "dictionary");
        assert_eq!(morphology["dictionary"]["name"], "ipadic");
        assert_eq!(morphology["dictionary"]["source"], "bundled");
        assert!(morphology["dictionary"]["path"].is_null());
        // 辞書で数えるので、ひらがなの語で終わる連鎖も拾う
        assert_eq!(rule_count(&v, "P16"), 1);
    } else {
        assert_eq!(morphology["reason"], "not-found");
    }

    // required で見つからなければ設定の誤り (同梱の辞書があれば、それで満たされる)
    fs::write(
        dir.path().join("noslop.toml"),
        "[morphology]\nmode = \"required\"\n",
    )
    .unwrap();
    let required = noslop_without_installed_dictionary(empty.path())
        .current_dir(dir.path())
        .args(["check", "--only-rules", "P16", "doc.md"])
        .assert();
    if cfg!(feature = "bundled-dict") {
        required.success();
    } else {
        required
            .code(2)
            .stderr(predicate::str::contains("辞書が見つかりません"));
    }

    // 辞書を使うルールが動かなければ、required でも辞書を探さない
    noslop_without_installed_dictionary(empty.path())
        .current_dir(dir.path())
        .args(["check", "--only-rules", "P01", "doc.md"])
        .assert()
        .success();

    // 設定ファイルの相対パスは、設定ファイルのディレクトリが基準
    write_dictionary(&dir.path().join("test.hsd"));
    fs::write(
        dir.path().join("noslop.toml"),
        "[morphology]\ndictionary = \"test.hsd\"\n",
    )
    .unwrap();
    fs::create_dir_all(dir.path().join("sub")).unwrap();
    let out = noslop_without_installed_dictionary(empty.path())
        .current_dir(dir.path().join("sub"))
        .args([
            "check",
            "--only-rules",
            "P16",
            "--format",
            "json",
            "../doc.md",
        ])
        .output()
        .unwrap();
    let v = json(&out.stdout);
    assert_eq!(v["settings"]["morphology"]["method"], "dictionary");
    assert_eq!(rule_count(&v, "P16"), 1);
}

// ---------------------------------------------------------------------------
// 配布辞書の取得 (noslop dict)
// ---------------------------------------------------------------------------

/// リポジトリに同梱した辞書 (hasami の配布辞書 ipadic と同じもの)。
fn bundled_dictionary() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("dict")
        .join("ipadic.hsd")
}

/// プロキシの環境変数を外した noslop (通信は 127.0.0.1 のテスト用のサーバーとだけ行う)。
fn noslop_offline() -> Command {
    let mut cmd = noslop();
    for var in [
        "ALL_PROXY",
        "all_proxy",
        "HTTPS_PROXY",
        "https_proxy",
        "HTTP_PROXY",
        "http_proxy",
    ] {
        cmd.env_remove(var);
    }
    cmd
}

/// `/<name>.hsd` への GET に `body` を返す (ほかは 404) テスト用の HTTP サーバー。URL と、受けた
/// 要求の数を返す。
fn serve_file(file: &str, body: Vec<u8>) -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let requests = Arc::new(AtomicUsize::new(0));
    let count = Arc::clone(&requests);
    let served = format!("GET /{file} ");
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            count.fetch_add(1, Ordering::SeqCst);
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut request_line = String::new();
            let _ = reader.read_line(&mut request_line);
            loop {
                let mut line = String::new();
                match reader.read_line(&mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(_) if line == "\r\n" => break,
                    Ok(_) => {}
                }
            }
            let (status, body) = if request_line.starts_with(&served) {
                ("200 OK", &body[..])
            } else {
                ("404 Not Found", &[][..])
            };
            let head = format!(
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream
                .write_all(head.as_bytes())
                .and_then(|()| stream.write_all(body))
                .and_then(|()| close_after_peer(&stream, &mut reader));
        }
    });
    (url, requests)
}

/// 送信側だけを閉じ、相手が読み終えて閉じるまで待つ (待つのは 10 秒まで)。書き終えてすぐに
/// 閉じると、Windows では相手が本文を受け取っている途中で接続が切られることがある
/// (CI の Windows で `Peer disconnected` になった)。
fn close_after_peer(stream: &TcpStream, reader: &mut BufReader<TcpStream>) -> io::Result<()> {
    stream.shutdown(Shutdown::Write)?;
    reader
        .get_ref()
        .set_read_timeout(Some(Duration::from_secs(10)))?;
    io::copy(reader, &mut io::sink()).map(drop)
}

/// `dict list` の表で、名前が `name` の行。
fn dictionary_row<'a>(stdout: &'a str, name: &str) -> &'a str {
    stdout
        .lines()
        .find(|line| line.split_whitespace().next() == Some(name))
        .unwrap_or_else(|| panic!("{name} の行がない:\n{stdout}"))
}

/// ディレクトリに残った一時ファイル。
fn partial_files(dir: &Path) -> Vec<String> {
    fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .filter(|name| name.ends_with(".part"))
        .collect()
}

#[test]
fn dict_list_shows_whether_each_dictionary_is_downloaded() {
    let dir = tempfile::tempdir().unwrap();
    let list = || {
        let out = noslop()
            .args(["dict", "list", "--dir"])
            .arg(dir.path())
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(0));
        String::from_utf8(out.stdout).unwrap()
    };
    let stdout = list();
    for dict in &DICTIONARIES {
        assert!(
            dictionary_row(&stdout, dict.name).contains("未取得"),
            "{stdout}"
        );
    }

    // 目録と中身の違うファイルは「中身が違う」。同梱の辞書も、目録の版と違えばこうなる
    // (目録と同じ中身で「取得済み」になる表示は cli.rs の単体テストで確かめる)
    let [first, second] = [&DICTIONARIES[0], &DICTIONARIES[1]];
    fs::write(
        dir.path().join(first.file_name()),
        b"not the catalog dictionary",
    )
    .unwrap();
    let stdout = list();
    assert!(
        dictionary_row(&stdout, first.name).contains("中身が違う"),
        "{stdout}"
    );
    assert!(dictionary_row(&stdout, second.name).contains("未取得"));
}

/// 保存先は --dir、省くと share ディレクトリ (HASAMI_DATA_DIR、なければ $XDG_DATA_HOME/hasami)。
/// 取得の前に保存先を作るので、目録と合わない中身を返すサーバーでも、どこに置こうとしたかが分かる。
/// 既定では圧縮版 (<名前>.hsd.zst) を取る。取得できる場合は目録の実物が要るので、単体テスト
/// (dictionaries・cli) と make dict-check で確かめる
#[test]
fn dict_download_puts_the_dictionary_in_the_chosen_directory() {
    let dict = &DICTIONARIES[0];
    let compressed = dict.compressed.expect("目録の辞書には圧縮版がある");
    // 目録と大きさの違う中身を返す (受信の前に Content-Length で断る)
    let (url, requests) = serve_file(compressed.file, Vec::new());
    let root = tempfile::tempdir().unwrap();

    let explicit = root.path().join("explicit");
    noslop_offline()
        .args(["dict", "download", dict.name, "--source", &url, "--dir"])
        .arg(&explicit)
        .assert()
        .code(2)
        .stdout(predicate::str::contains(format!("{} (圧縮版 ", dict.name)))
        .stdout(predicate::str::contains("を取得しています: "))
        .stderr(predicate::str::contains(format!(
            "{} の大きさが違います",
            compressed.file
        )))
        .stderr(predicate::str::contains("の圧縮版は"));
    assert!(explicit.is_dir());
    assert!(!explicit.join(dict.file_name()).exists());
    assert!(partial_files(&explicit).is_empty());

    // HASAMI_DATA_DIR は XDG_DATA_HOME より先 (hasami の data_dir と同じ規則)。下に hasami を足さない
    let preferred = root.path().join("preferred");
    let xdg = root.path().join("xdg");
    noslop_offline()
        .env("HASAMI_DATA_DIR", &preferred)
        .env("XDG_DATA_HOME", &xdg)
        .args(["dict", "download", dict.name, "--source", &url])
        .assert()
        .code(2);
    assert!(preferred.is_dir());
    assert!(!xdg.exists());

    noslop_offline()
        .env_remove("HASAMI_DATA_DIR")
        .env("XDG_DATA_HOME", &xdg)
        .args(["dict", "download", dict.name, "--source", &url])
        .assert()
        .code(2);
    assert!(xdg.join("hasami").is_dir());
    assert_eq!(requests.load(Ordering::SeqCst), 3);
}

#[test]
fn dict_download_rejects_unknown_names_and_failed_downloads() {
    noslop()
        .args(["dict", "download", "unidic"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains(RECOMMENDED));

    // 圧縮版が 404 なら非圧縮版も試し、どちらもなければ非圧縮版の URL で誤りを伝える。
    // サーバーは目録の先頭の辞書の、展開前のファイルだけを配る。
    let [first, second] = [&DICTIONARIES[0], &DICTIONARIES[1]];
    let (url, requests) = serve_file(&first.file_name(), Vec::new());
    let dir = tempfile::tempdir().unwrap();
    noslop_offline()
        .args(["dict", "download", second.name, "--dir"])
        .arg(dir.path())
        .args(["--source", &url])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("HTTP 404"))
        .stderr(predicate::str::contains(format!(
            "{url}/{} を取得できません",
            second.file_name()
        )))
        .stderr(predicate::str::contains("--uncompressed").not());
    assert_eq!(requests.load(Ordering::SeqCst), 2);
    // --uncompressed なら展開前の辞書を取る。中身の違うものは置かない (大きさが合わない)
    noslop_offline()
        .args(["dict", "download", first.name, "--uncompressed", "--dir"])
        .arg(dir.path())
        .args(["--source", &url])
        .assert()
        .code(2)
        .stdout(predicate::str::contains(format!(
            "{} ({} MB)",
            first.name,
            megabytes(first.size)
        )))
        .stderr(predicate::str::contains(format!(
            "{} の大きさが違います",
            first.file_name()
        )))
        .stderr(predicate::str::contains("--uncompressed").not());
    assert_eq!(requests.load(Ordering::SeqCst), 3);
    assert!(!dir.path().join(second.file_name()).exists());
    assert!(!dir.path().join(first.file_name()).exists());
    assert!(partial_files(dir.path()).is_empty());
}

/// 同じ辞書も古い辞書も取り直し、検証に失敗したときは既存の中身を保つ。
/// 目録の版に依存しないよう、テスト用の小さな辞書を配布する。
#[test]
fn dictionary_download_always_replaces_after_verification() {
    // ライブラリは親プロセスのプロキシ設定を読む。ほかの並列テストの環境を変えずに、
    // このテストだけプロキシを外した子プロセスで実行する。
    const CHILD: &str = "NOSLOP_TEST_DOWNLOAD_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let mut child = std::process::Command::new(std::env::current_exe().unwrap());
        child
            .args([
                "--exact",
                "dictionary_download_always_replaces_after_verification",
                "--nocapture",
            ])
            .env(CHILD, "1");
        for var in [
            "ALL_PROXY",
            "all_proxy",
            "HTTPS_PROXY",
            "https_proxy",
            "HTTP_PROXY",
            "http_proxy",
        ] {
            child.env_remove(var);
        }
        let output = child.output().unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let source = tempfile::tempdir().unwrap();
    let source_path = source.path().join("test.hsd");
    write_dictionary(&source_path);
    let body = fs::read(&source_path).unwrap();
    let dict = noslop::dictionaries::Distributed {
        name: "test",
        summary: "テスト用",
        size: body.len() as u64,
        sha256: hasami::download::sha256_file(&source_path).unwrap().leak(),
        compressed: None,
    };
    let (url, requests) = serve_file(&dict.file_name(), body.clone());
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(dict.file_name());
    for (i, old) in [b"old dictionary".as_slice(), body.as_slice()]
        .into_iter()
        .enumerate()
    {
        fs::write(&path, old).unwrap();
        let mut received = 0;
        let placed =
            noslop::dictionaries::download(&dict, dir.path(), &url, true, &mut |r, _| received = r)
                .unwrap();
        assert_eq!(placed, path);
        assert_eq!(fs::read(&path).unwrap(), body);
        assert_eq!(received, dict.size);
        assert_eq!(requests.load(Ordering::SeqCst), i + 1);
    }

    // 大きさが同じでも SHA-256 が違う中身は置かない。
    let mut corrupt = body.clone();
    corrupt[0] ^= 1;
    let (url, requests) = serve_file(&dict.file_name(), corrupt);
    let err = noslop::dictionaries::download(&dict, dir.path(), &url, true, &mut |_, _| {})
        .unwrap_err()
        .to_string();
    assert!(err.contains("SHA-256 が違います"), "{err}");
    assert_eq!(requests.load(Ordering::SeqCst), 1);
    assert_eq!(fs::read(&path).unwrap(), body);
    assert!(partial_files(dir.path()).is_empty());
}

/// `dict download` の表示と同じ、10 進の MB で小数 1 桁。
fn megabytes(bytes: u64) -> String {
    format!("{:.1}", bytes as f64 / 1_000_000.0)
}

#[test]
fn share_specs_point_to_the_share_directory() {
    let data = tempfile::tempdir().unwrap();
    let doc = data.path().join("doc.md");
    fs::write(&doc, DOC_NO_CHAIN).unwrap();
    let check = || {
        let mut cmd = noslop();
        cmd.env_remove("HASAMI_DICT")
            .env_remove("HASAMI_DATA_DIR")
            .env("XDG_DATA_HOME", data.path())
            .args([
                "check",
                "--no-config",
                "--only-rules",
                "P16",
                "--format",
                "json",
                "--dict",
                "share:ipadic",
            ])
            .arg(&doc);
        cmd
    };

    // まだ取得していなければ、取得のコマンドを案内する
    check()
        .assert()
        .code(2)
        .stderr(predicate::str::contains("share:ipadic の辞書がありません"))
        .stderr(predicate::str::contains("`noslop dict download ipadic`"));

    let placed = data.path().join("hasami").join("ipadic.hsd");
    fs::create_dir_all(placed.parent().unwrap()).unwrap();
    fs::copy(bundled_dictionary(), &placed).unwrap();
    let out = check().output().unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v = json(&out.stdout);
    let morphology = &v["settings"]["morphology"];
    assert_eq!(morphology["method"], "dictionary");
    assert_eq!(morphology["dictionary"]["name"], "ipadic");
    assert_eq!(morphology["dictionary"]["source"], "file");
    assert_eq!(
        morphology["dictionary"]["path"].as_str(),
        placed.to_str(),
        "share:<名前> は share ディレクトリのファイルに直す"
    );
    assert_eq!(rule_count(&v, "P16"), 1);

    // 設定ファイルの share:<名前> も、設定ファイルのディレクトリ基準のパスにしない
    let project = tempfile::tempdir().unwrap();
    fs::write(
        project.path().join("noslop.toml"),
        "[morphology]\ndictionary = \"share:ipadic\"\n",
    )
    .unwrap();
    let out = noslop()
        .env_remove("HASAMI_DICT")
        .env_remove("HASAMI_DATA_DIR")
        .env("XDG_DATA_HOME", data.path())
        .current_dir(project.path())
        .args(["check", "--only-rules", "P16", "--format", "json"])
        .arg(&doc)
        .output()
        .unwrap();
    let v = json(&out.stdout);
    assert_eq!(
        v["settings"]["morphology"]["dictionary"]["path"].as_str(),
        placed.to_str()
    );

    // HASAMI_DATA_DIR があれば XDG_DATA_HOME より先に見る (hasami の data_dir と同じ規則)。
    // HASAMI_DATA_DIR は置き場そのもので、下に hasami を足さない
    let preferred = tempfile::tempdir().unwrap();
    let placed_in_preferred = preferred.path().join("ipadic.hsd");
    fs::copy(bundled_dictionary(), &placed_in_preferred).unwrap();
    let out = check()
        .env("HASAMI_DATA_DIR", preferred.path())
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v = json(&out.stdout);
    assert_eq!(
        v["settings"]["morphology"]["dictionary"]["path"].as_str(),
        placed_in_preferred.to_str()
    );
}

/// 辞書を指定しないとき (auto) は、share ディレクトリの配布辞書を hasami の推奨順で選ぶ。
/// bundled なら share ディレクトリに辞書があっても同梱の辞書。選んだ辞書が読めなければ設定の誤り。
#[test]
fn auto_uses_the_best_dictionary_in_the_share_directory() {
    let share = tempfile::tempdir().unwrap();
    let doc = share.path().join("doc.md");
    fs::write(&doc, DOC_NO_CHAIN).unwrap();
    for name in ["ipadic", "ipadic-neologd-sudachi"] {
        write_dictionary(&share.path().join(format!("{name}.hsd")));
    }
    let best = share.path().join("ipadic-neologd-sudachi.hsd");
    let check = |extra: &[&str]| {
        let mut cmd = noslop();
        cmd.env("HASAMI_DATA_DIR", share.path())
            .args([
                "check",
                "--no-config",
                "--only-rules",
                "P16",
                "--format",
                "json",
            ])
            .args(extra)
            .arg(&doc);
        cmd.output().unwrap()
    };

    let out = check(&[]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v = json(&out.stdout);
    let morphology = &v["settings"]["morphology"];
    assert_eq!(morphology["requested"], "auto");
    assert_eq!(morphology["dictionary"]["source"], "file");
    assert_eq!(morphology["dictionary"]["path"].as_str(), best.to_str());
    assert_eq!(rule_count(&v, "P16"), 1);

    // dictionary = "auto" と --dict auto も同じ辞書を選ぶ (--dict は辞書を必須にする)
    let out = check(&["--dict", "auto"]);
    let v = json(&out.stdout);
    assert_eq!(v["settings"]["morphology"]["requested"], "required");
    assert_eq!(
        v["settings"]["morphology"]["dictionary"]["path"].as_str(),
        best.to_str()
    );

    // bundled は share ディレクトリの辞書を見ない
    let out = check(&["--dict", "bundled"]);
    if cfg!(feature = "bundled-dict") {
        let v = json(&out.stdout);
        assert_eq!(
            v["settings"]["morphology"]["dictionary"]["source"],
            "bundled"
        );
    } else {
        assert_eq!(out.status.code(), Some(2));
    }

    // dict list は、辞書を指定しないときに今使う辞書を示す
    noslop()
        .env("HASAMI_DATA_DIR", share.path())
        .args(["dict", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("今は ipadic-neologd-sudachi ("));

    // 選んだ辞書が読めなければ、下の候補 (ipadic) に切り替えずに設定の誤り
    fs::write(&best, b"not a dictionary").unwrap();
    let out = check(&[]);
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("自動で選んだ辞書"), "{stderr}");
    assert!(
        stderr.contains("`noslop dict download ipadic-neologd-sudachi`"),
        "{stderr}"
    );
}

/// ユーザーの設定 (~/.config/noslop/config.toml) の上に、プロジェクトの設定を項目ごとに重ねる。
#[test]
fn the_user_config_is_layered_under_the_project_config() {
    let home = tempfile::tempdir().unwrap();
    let user_config = home
        .path()
        .join(".config")
        .join("noslop")
        .join("config.toml");
    fs::create_dir_all(user_config.parent().unwrap()).unwrap();
    // 独自ルール X01 と、share ディレクトリの辞書の指定 (相対パスでない share: はそのまま)
    fs::write(
        &user_config,
        format!("{CONFIG}\n[morphology]\ndictionary = \"share:ipadic\"\n[files]\nexclude = [\"/drafts/\"]\n"),
    )
    .unwrap();
    // auto なら推奨順の先頭 (ipadic-neologd-sudachi) を選ぶので、ユーザーの設定 (share:ipadic) が
    // 効いているかを見分けられる
    let share = tempfile::tempdir().unwrap();
    let [ipadic, best] =
        ["ipadic", "ipadic-neologd-sudachi"].map(|name| share.path().join(format!("{name}.hsd")));
    write_dictionary(&ipadic);
    write_dictionary(&best);
    let dictionary_path = |v: &serde_json::Value| {
        PathBuf::from(
            v["settings"]["morphology"]["dictionary"]["path"]
                .as_str()
                .unwrap(),
        )
    };

    let dir = workspace();
    fs::remove_file(dir.path().join("noslop.toml")).unwrap();
    fs::create_dir_all(dir.path().join("drafts")).unwrap();
    fs::write(dir.path().join("drafts/wip.md"), DOC_WITH_TERM).unwrap();
    let check = |extra: &[&str]| {
        let mut cmd = noslop();
        cmd.env("HOME", home.path())
            .env("USERPROFILE", home.path())
            .env("HASAMI_DATA_DIR", share.path())
            .current_dir(dir.path())
            .args(["check", "--format", "json"])
            .args(extra);
        let out = cmd.output().unwrap();
        assert_eq!(
            out.status.code(),
            Some(0),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        json(&out.stdout)
    };

    // プロジェクトの設定がなくても、ユーザーの設定が効く。除外は検査の起点 (.) が基準
    let v = check(&[]);
    assert_eq!(x01_count(&v), 1, "docs/guide.md だけ (drafts/ は除外)");
    assert_eq!(dictionary_path(&v), ipadic);

    // プロジェクトの設定の同じ ID の独自ルールは、ユーザーの定義を置き換える
    fs::write(
        dir.path().join("noslop.toml"),
        "[[custom]]\nid = \"x01\"\npattern = \"問題のない\"\nmessage = \"プロジェクトの定義\"\n[files]\nexclude = []\n",
    )
    .unwrap();
    let v = check(&[]);
    let messages: Vec<&str> = v["files"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|f| f["diagnostics"].as_array().unwrap())
        .filter(|d| d["ruleId"] == "x01")
        .map(|d| d["message"].as_str().unwrap())
        .collect();
    assert_eq!(
        messages, ["プロジェクトの定義"; 2],
        "exclude = [] でユーザーの除外も外れる"
    );
    assert_eq!(x01_count(&v), 0);
    assert_eq!(
        dictionary_path(&v),
        ipadic,
        "プロジェクトに書いていない項目はユーザーの設定"
    );

    // --no-config はユーザーの設定も読まない (辞書は auto で選ぶ)
    let v = check(&["--no-config"]);
    assert_eq!(x01_count(&v), 0);
    assert_eq!(dictionary_path(&v), best);
}
