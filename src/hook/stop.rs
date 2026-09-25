//! Stop: リポジトリのコミットしていない変更 (HEAD との差分と追跡していないファイル) の、変わった行に
//! 重なる指摘を返す。
//!
//! Claude Code の Stop フック (`hook claude-code`) と、フックの入力を渡せない Stop の仕組み
//! (claw-hooks の stop_hooks など) 向けの `hook git-diff` の 2 つの出口がある。どちらも作業
//! ディレクトリを含む git の作業ツリーを見て、git の外では何もしない。
//!
//! - Claude Code: 指摘があれば `hookSpecificOutput.additionalContext` で Claude に渡す (会話は続き、
//!   Claude が見直してもう一度止まる)。`stop_hook_active` が真 (このフックで続けた後の Stop) なら
//!   何もしない
//! - `hook git-diff`: 指摘があればテキストを標準出力に書いて終了コード 1、なければ何も書かずに 0。
//!   設定の誤りや git の失敗は、標準エラーに書いて 2
//!
//! 同じ指摘を二度と出さないための記録は持たない (続けた後の Stop では動かないので、同じ応答の中で
//! 繰り返さない)。

use std::io::{self, Write};
use std::path::{Path, PathBuf};

use rayon::prelude::*;
use serde_json::{Value, json};

use super::git::{ChangedFile, WorkTree, line_regions};
use super::{CONTEXT_BUDGET_CHARS, Reviewer, display_name, touches, truncate_lines};
use crate::cli::{self, GitDiffHookArgs, HookArgs};
use crate::engine::FileReport;

/// 改稿指示の後ろに添える注記 (どちらの出口も)。
const KEEP_NOTE: &str = "このフックは応答を終えるたびに動きます。一度見直して残すと決めた指摘は、再び出ても直す必要はありません。\n";

/// 変わった行に絞ったときの注記。
const LIMITED_NOTE: &str = "コミットしていない変更 (git の HEAD との差分の行と、追跡していないファイルの全体) に重なる指摘だけを返しています。ファイル全体は `noslop check --format brief <パス>` で確認できます。\n";

/// `--whole-file` のときの注記。
const WHOLE_FILE_NOTE: &str = "コミットしていない変更 (git の HEAD との差分と、追跡していないファイル) のあるファイルの、全体の指摘を返しています。\n";

/// 注記を後ろに残したまま改稿指示を切り詰めるのに要る、改稿指示の最小の文字数。上限がこれより
/// 小さいときは、注記ごと行の単位で切る。
const MIN_BRIEF_CHARS: usize = 300;

/// Stop の入力に対して標準出力に書く JSON。何も書かないなら `None`。
pub(super) fn stop(
    event: &Value,
    args: &HookArgs,
    env: &cli::Environment,
) -> Result<Option<String>, String> {
    // このフックで会話を続けた後の Stop では動かない (見直しを繰り返させない)
    if event.get("stop_hook_active").and_then(Value::as_bool) == Some(true) {
        return Ok(None);
    }
    let cwd = match event.get("cwd").and_then(Value::as_str) {
        Some(dir) => PathBuf::from(dir),
        None => {
            std::env::current_dir().map_err(|e| format!("作業ディレクトリを取得できません: {e}"))?
        }
    };
    let Some(context) = review_repository(&cwd, args, env, CONTEXT_BUDGET_CHARS)? else {
        return Ok(None);
    };
    let output = json!({
        "hookSpecificOutput": {
            "hookEventName": "Stop",
            "additionalContext": context,
        }
    });
    Ok(Some(output.to_string()))
}

/// `noslop hook git-diff` の本体。終了コードを返す。
///
/// 指摘があれば改稿指示をテキストで標準出力に書いて 1、なければ何も書かずに 0 (git の外でも 0)。
/// 設定の誤りや git の失敗は標準エラーに書いて 2。
pub fn git_diff(args: &GitDiffHookArgs) -> u8 {
    let result = std::env::current_dir()
        .map_err(|e| format!("作業ディレクトリを取得できません: {e}"))
        .and_then(|cwd| review_git_diff(args, &cwd, &cli::Environment::from_process()));
    match result {
        Ok(Some(text)) => {
            let mut out = io::stdout().lock();
            match out.write_all(text.as_bytes()).and_then(|()| out.flush()) {
                Ok(()) => 1,
                Err(e) => {
                    eprintln!("noslop: 出力に失敗しました: {e}");
                    2
                }
            }
        }
        Ok(None) => 0,
        Err(message) => {
            eprintln!("noslop: {message}");
            2
        }
    }
}

/// [`git_diff`] が書く改稿指示。何も書かないなら `None`。`cwd` は作業ディレクトリ (git の作業
/// ツリーと設定ファイルを探し始める場所と、表示名の基準)。
fn review_git_diff(
    args: &GitDiffHookArgs,
    cwd: &Path,
    env: &cli::Environment,
) -> Result<Option<String>, String> {
    review_repository(cwd, &args.hook, env, args.max_chars)
}

/// `start` を含む git の作業ツリーの、コミットしていない変更を検査する。変わった行 (追跡していない
/// ファイルは全体) に重なる指摘の改稿指示に注記を添え、`budget` 文字に収めて返す。git の外と、
/// 見せる指摘がないときは `None`。
fn review_repository(
    start: &Path,
    args: &HookArgs,
    env: &cli::Environment,
    budget: usize,
) -> Result<Option<String>, String> {
    let Some(tree) = WorkTree::discover(start)? else {
        return Ok(None);
    };
    // 表示名とユーザーの設定の除外の基準。変わったファイルのパスも、この形 (シンボリックリンクを
    // 解かない形) にそろえてある
    let start = std::path::absolute(start).unwrap_or_else(|_| start.to_path_buf());
    let reviewer = Reviewer::new(args, Some(&start), env)?;
    let files = tree.changes(|path| reviewer.selects(&start, path) && is_regular_file(path))?;
    if files.is_empty() {
        return Ok(None);
    }
    let reports: Vec<FileReport> = files
        .par_iter()
        .filter_map(|file| lint(&reviewer, file, &start, args.whole_file))
        .collect();
    let Some(brief) = reviewer.brief(reports)? else {
        return Ok(None);
    };
    let mut notes = String::from(KEEP_NOTE);
    notes.push_str(if args.whole_file {
        WHOLE_FILE_NOTE
    } else {
        LIMITED_NOTE
    });
    Ok(Some(fit(&brief, &notes, budget)))
}

/// 変わったファイルを検査し、変わった行に重なる指摘だけを残す。読めないファイル (UTF-8 でない
/// など) は標準エラーに 1 行書いて飛ばす。消えた・大きすぎるファイルも飛ばす。
fn lint(
    reviewer: &Reviewer,
    file: &ChangedFile,
    start: &Path,
    whole_file: bool,
) -> Option<FileReport> {
    let name = display_name(&file.path, Some(start));
    let mut report = match reviewer.lint_file(&file.path, name) {
        Ok(report) => report?,
        Err(message) => {
            eprintln!("noslop: {message} (このファイルは飛ばします)");
            return None;
        }
    };
    if let (false, Some(lines)) = (whole_file, &file.lines) {
        let regions = line_regions(lines, &report.doc);
        report.diagnostics.retain(|d| touches(d, &regions));
    }
    Some(report)
}

/// 通常のファイルか (シンボリックリンク・ディレクトリ (サブモジュール) は見ない。リンクの差分は
/// リンク先の中身の行ではないため)。
fn is_regular_file(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|m| m.is_file())
}

/// 改稿指示と注記を `budget` 文字に収める。注記は残し、改稿指示の後ろを行の単位で省く。上限が
/// 小さすぎるときは、注記ごと切る。
fn fit(brief: &str, notes: &str, budget: usize) -> String {
    let note_chars = notes.chars().count();
    if budget >= note_chars + MIN_BRIEF_CHARS {
        let mut text = truncate_lines(brief, budget - note_chars);
        text.push_str(notes);
        text
    } else {
        truncate_lines(&format!("{brief}{notes}"), budget)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::process::{Command, Stdio};

    use super::super::testing::{CONFIG, DOC, parse};
    use super::*;

    fn git_diff_args(extra: &[&str]) -> GitDiffHookArgs {
        let mut argv = vec!["git-diff"];
        argv.extend_from_slice(extra);
        match parse(&argv) {
            cli::HookCommand::GitDiff(a) => a,
            _ => panic!("hook git-diff"),
        }
    }

    fn hook_args(extra: &[&str]) -> HookArgs {
        let mut argv = vec!["claude-code"];
        argv.extend_from_slice(extra);
        match parse(&argv) {
            cli::HookCommand::ClaudeCode(a) => a,
            _ => panic!("hook claude-code"),
        }
    }

    /// `hook git-diff` の出力 (手元の設定と辞書に左右されないよう、空の環境で)。
    fn run(dir: &Path, extra: &[&str]) -> Option<String> {
        review_git_diff(&git_diff_args(extra), dir, &cli::Environment::default()).unwrap()
    }

    /// テスト用の git。手元の git の設定 (署名・フック・既定のブランチ名・改行の変換) に左右されない
    /// ように切る。
    fn git(dir: &Path, args: &[&str]) {
        let out = Command::new("git")
            .arg("-C")
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
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .stdin(Stdio::null())
            .output()
            .expect("git を実行できません");
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn commit(dir: &Path) {
        git(dir, &["add", "-A"]);
        git(dir, &["commit", "-q", "--no-verify", "-m", "c"]);
    }

    /// git を実行できるか (できない環境では、差分を見るテストを飛ばす)。
    fn git_available() -> bool {
        let ok = Command::new("git")
            .arg("--version")
            .stdin(Stdio::null())
            .output()
            .is_ok_and(|o| o.status.success());
        if !ok {
            eprintln!("git を実行できないので、差分を見るテストを飛ばします");
        }
        ok
    }

    /// `CONFIG` の設定と、本文 `DOC` の guide.md をコミットしたリポジトリ。git がなければ `None`。
    fn repository() -> Option<tempfile::TempDir> {
        if !git_available() {
            return None;
        }
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("noslop.toml"), CONFIG).unwrap();
        fs::write(dir.path().join("guide.md"), DOC).unwrap();
        git(dir.path(), &["init", "-q"]);
        commit(dir.path());
        Some(dir)
    }

    fn write(dir: &Path, rel: &str, text: &str) {
        let path = dir.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    /// `DOC` の 5 行目を書き換えた本文 (3 行目の指摘はそのまま)。
    fn edited_doc() -> String {
        DOC.replace("新しい段落です。", "書き換えた段落です。")
    }

    /// コミットした直後は何も返さない。書き換えた行の指摘だけを返し、変わっていない行の指摘は
    /// 返さない。`--whole-file` は変わったファイルの全体を返す。
    #[test]
    fn only_findings_on_uncommitted_lines_are_returned() {
        let Some(dir) = repository() else { return };
        assert_eq!(run(dir.path(), &[]), None, "変更がなければ何も返さない");

        write(dir.path(), "guide.md", &edited_doc());
        let text = run(dir.path(), &[]).unwrap();
        assert!(
            text.starts_with("noslop が guide.md に 独自ルールの指摘を 1 件見つけました。"),
            "{text}"
        );
        assert!(text.contains("  - L5: "), "{text}");
        assert!(!text.contains("L3:"), "変わっていない行は返さない: {text}");
        assert!(text.contains(KEEP_NOTE), "{text}");
        assert!(text.contains(LIMITED_NOTE), "{text}");
        assert!(!text.trim_start().starts_with('{'), "JSON ではなくテキスト");

        let whole = run(dir.path(), &["--whole-file"]).unwrap();
        assert!(whole.contains("L3:") && whole.contains("L5:"), "{whole}");
        assert!(whole.contains(WHOLE_FILE_NOTE), "{whole}");
        assert!(!whole.contains(LIMITED_NOTE), "{whole}");
    }

    /// index に入れた変更・作業ツリーの変更・追跡していないファイルをまとめて見る。追跡していない
    /// ファイルは全体を見る。
    #[test]
    fn staged_unstaged_and_untracked_changes_are_reviewed() {
        let Some(dir) = repository() else { return };
        write(dir.path(), "guide.md", &edited_doc());
        write(
            dir.path(),
            "docs/staged.md",
            "# 手順\n\nユーザー様の手順です。\n",
        );
        git(dir.path(), &["add", "docs/staged.md"]);
        write(
            dir.path(),
            "notes/new.txt",
            "ユーザー様への連絡です。\n\nもう一度ユーザー様に書きます。\n",
        );
        let text = run(dir.path(), &[]).unwrap();
        assert!(
            text.starts_with("noslop が 3 ファイルに 独自ルールの指摘を 4 件見つけました。"),
            "{text}"
        );
        assert!(
            text.contains("docs/staged.md: 独自ルールの指摘 1 件\n"),
            "{text}"
        );
        assert!(text.contains("guide.md: 独自ルールの指摘 1 件\n"), "{text}");
        assert!(
            text.contains("notes/new.txt: 独自ルールの指摘 2 件\n"),
            "追跡していないファイルは全体: {text}"
        );
        // ファイルはパスの順
        let order: Vec<usize> = ["docs/staged.md:", "\nguide.md:", "notes/new.txt:"]
            .iter()
            .map(|name| text.find(name).unwrap())
            .collect();
        assert!(order.windows(2).all(|w| w[0] < w[1]), "{text}");

        // index に入れた後で作業ツリーを書き換えても、HEAD との差分で見る
        write(
            dir.path(),
            "docs/staged.md",
            "# 手順\n\nユーザー様の手順です。\n\nユーザー様に追記しました。\n",
        );
        let text = run(dir.path(), &[]).unwrap();
        assert!(
            text.contains("docs/staged.md: 独自ルールの指摘 2 件\n"),
            "{text}"
        );
    }

    /// 消したファイルは見ない。行を消したときは、つなぎ目の前後の行の指摘を返す。
    #[test]
    fn deletions_report_only_the_lines_around_the_joint() {
        let Some(dir) = repository() else { return };
        write(
            dir.path(),
            "other.md",
            "# 別\n\nユーザー様の話です。\n\n消す段落です。\n\n残りの段落もユーザー様の話です。\n",
        );
        commit(dir.path());

        fs::remove_file(dir.path().join("guide.md")).unwrap();
        assert_eq!(run(dir.path(), &[]), None, "消しただけなら何も返さない");
        git(dir.path(), &["rm", "-q", "--cached", "guide.md"]);
        assert_eq!(run(dir.path(), &[]), None, "index から消しても同じ");

        write(
            dir.path(),
            "other.md",
            "# 別\n\nユーザー様の話です。\n\n残りの段落もユーザー様の話です。\n",
        );
        let text = run(dir.path(), &[]).unwrap();
        assert!(text.contains("noslop が other.md に"), "{text}");
        assert!(text.contains("  - L5: ") && !text.contains("L3:"), "{text}");
    }

    /// 名前を変えたファイルは、新しいパスで、名前を変える前との差分の行だけを見る。
    #[test]
    fn renamed_files_report_only_the_edited_lines() {
        let Some(dir) = repository() else { return };
        // 名前の変更と見なされるよう、同じ行を多めに置く
        let body: String = (1..=8)
            .map(|i| format!("{i} 番目の段落はユーザー様の話です。\n\n"))
            .collect();
        write(dir.path(), "long.md", &body);
        commit(dir.path());

        git(dir.path(), &["mv", "long.md", "renamed.md"]);
        assert_eq!(run(dir.path(), &[]), None, "名前だけの変更は何も返さない");

        write(
            dir.path(),
            "renamed.md",
            &body.replace("3 番目の段落は", "三つ目の段落も"),
        );
        let text = run(dir.path(), &[]).unwrap();
        assert!(
            text.contains("noslop が renamed.md に 独自ルールの指摘を 1 件"),
            "{text}"
        );
        assert!(text.contains("  - L5: "), "{text}");
    }

    /// .gitignore で無視するファイル、設定の除外、検査しない拡張子は見ない。
    #[test]
    fn ignored_excluded_and_unselected_files_are_skipped() {
        let Some(dir) = repository() else { return };
        write(
            dir.path(),
            "noslop.toml",
            &format!("{CONFIG}\n[files]\nexclude = [\"drafts/\"]\n"),
        );
        write(dir.path(), ".gitignore", "ignored.md\n");
        commit(dir.path());

        let finding = "ユーザー様の話です。\n";
        write(dir.path(), "ignored.md", finding);
        write(dir.path(), "drafts/draft.md", finding);
        write(dir.path(), "main.rs", "// ユーザー様の話です。\n");
        write(dir.path(), "data.csv", finding);
        assert_eq!(run(dir.path(), &[]), None);

        // 設定の除外は、追跡しているファイルの差分にも当てる
        commit(dir.path());
        write(
            dir.path(),
            "drafts/draft.md",
            "ユーザー様。\n\nユーザー様。\n",
        );
        assert_eq!(run(dir.path(), &[]), None);
    }

    /// git の外では、指摘のある文書があっても何も返さない。
    #[test]
    fn outside_git_returns_nothing() {
        if !git_available() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("noslop.toml"), CONFIG).unwrap();
        fs::write(dir.path().join("guide.md"), DOC).unwrap();
        // 一時ディレクトリがどこかのリポジトリの中にある環境では確かめられない
        if WorkTree::discover(dir.path()).unwrap().is_some() {
            eprintln!("一時ディレクトリが git のリポジトリの中なので、確かめを飛ばします");
            return;
        }
        assert_eq!(run(dir.path(), &[]), None);
    }

    /// まだコミットのないリポジトリでは、index に入れたファイルと追跡していないファイルの全体を見る。
    #[test]
    fn repository_without_head_reviews_whole_files() {
        if !git_available() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("noslop.toml"), CONFIG).unwrap();
        fs::write(dir.path().join("guide.md"), DOC).unwrap();
        fs::write(dir.path().join("new.md"), "ユーザー様。\n").unwrap();
        git(dir.path(), &["init", "-q"]);
        git(dir.path(), &["add", "guide.md"]);
        let text = run(dir.path(), &[]).unwrap();
        assert!(text.contains("guide.md: 独自ルールの指摘 2 件\n"), "{text}");
        assert!(text.contains("new.md: 独自ルールの指摘 1 件\n"), "{text}");
    }

    /// 作業ディレクトリがリポジトリの下のディレクトリでも、リポジトリ全体の変更を見る。設定ファイルは
    /// 作業ディレクトリから親へ探す。
    #[test]
    fn a_subdirectory_reviews_the_whole_repository() {
        let Some(dir) = repository() else { return };
        fs::create_dir(dir.path().join("sub")).unwrap();
        write(dir.path(), "guide.md", &edited_doc());
        write(dir.path(), "sub/local.md", "ユーザー様。\n");
        let text = run(&dir.path().join("sub"), &[]).unwrap();
        assert!(
            text.contains("\nlocal.md: "),
            "作業ディレクトリの中は相対パス: {text}"
        );
        assert!(text.contains("guide.md: "), "外のファイルも見る: {text}");
    }

    /// 空白・日本語を含むパスも崩さずに読む。
    #[test]
    fn paths_with_spaces_and_japanese_are_read() {
        let Some(dir) = repository() else { return };
        let names = ["docs/my notes.md", "docs/日本語 メモ.md"];
        for name in names {
            write(dir.path(), name, DOC);
        }
        commit(dir.path());
        for name in names {
            write(dir.path(), name, &edited_doc());
        }
        let text = run(dir.path(), &[]).unwrap();
        for name in names {
            assert!(
                text.contains(&format!("{name}: 独自ルールの指摘 1 件\n")),
                "{name}: {text}"
            );
        }
        assert!(!text.contains("L3:"), "{text}");
    }

    /// 引用符やタブを含むパス (git が引用して出力する) も読む。
    #[cfg(unix)]
    #[test]
    fn quoted_paths_are_read() {
        let Some(dir) = repository() else { return };
        let name = "docs/q\"uote\tand \\ back.md";
        write(dir.path(), name, DOC);
        commit(dir.path());
        write(dir.path(), name, &edited_doc());
        let text = run(dir.path(), &[]).unwrap();
        assert!(text.contains("  - L5: ") && !text.contains("L3:"), "{text}");
    }

    /// UTF-8 として読めないファイルは飛ばし、ほかのファイルは見る。
    #[test]
    fn unreadable_files_are_skipped() {
        let Some(dir) = repository() else { return };
        fs::write(dir.path().join("latin1.txt"), b"caf\xe9\n").unwrap();
        write(dir.path(), "guide.md", &edited_doc());
        let text = run(dir.path(), &[]).unwrap();
        assert!(text.contains("noslop が guide.md に"), "{text}");
        assert!(!text.contains("latin1.txt"), "{text}");
    }

    /// シンボリックリンクは見ない (リンクの差分はリンク先の中身の行ではない)。
    #[cfg(unix)]
    #[test]
    fn symbolic_links_are_skipped() {
        let Some(dir) = repository() else { return };
        std::os::unix::fs::symlink("guide.md", dir.path().join("link.md")).unwrap();
        assert_eq!(run(dir.path(), &[]), None);
    }

    /// 上限を超える分は行の単位で省く。注記は残す。上限が小さすぎるときは注記ごと切る。
    #[test]
    fn long_output_is_cut_within_max_chars() {
        let Some(dir) = repository() else { return };
        for i in 0..40 {
            write(
                dir.path(),
                &format!("docs/{i:02}.md"),
                "ユーザー様の話です。\n\nユーザー様にも届けます。\n",
            );
        }
        let full = run(dir.path(), &[]).unwrap();
        assert!(full.chars().count() > 2_000, "{}", full.chars().count());

        let cut = run(dir.path(), &["--max-chars", "2000"]).unwrap();
        assert!(cut.chars().count() <= 2_000, "{}", cut.chars().count());
        assert!(cut.contains("残り"), "{cut}");
        assert!(cut.ends_with(LIMITED_NOTE), "注記は残す: {cut}");

        let tiny = run(dir.path(), &["--max-chars", "120"]).unwrap();
        assert!(tiny.chars().count() <= 120, "{tiny}");
    }

    fn stop_event(dir: &Path, active: bool) -> Value {
        json!({
            "session_id": "s",
            "transcript_path": dir.join("t.jsonl").to_string_lossy(),
            "cwd": dir.to_string_lossy(),
            "permission_mode": "default",
            "hook_event_name": "Stop",
            "stop_hook_active": active,
        })
    }

    /// Claude Code の Stop には、改稿指示を additionalContext に入れた JSON を返す。このフックで
    /// 続けた後の Stop (`stop_hook_active`) では何もしない。
    #[test]
    fn claude_code_stop_returns_additional_context() {
        let Some(dir) = repository() else { return };
        let respond =
            |event: &Value| stop(event, &hook_args(&[]), &cli::Environment::default()).unwrap();
        assert_eq!(respond(&stop_event(dir.path(), false)), None);

        write(dir.path(), "guide.md", &edited_doc());
        let output = respond(&stop_event(dir.path(), false)).unwrap();
        let v: Value = serde_json::from_str(&output).unwrap();
        let object = v.as_object().unwrap();
        assert_eq!(object.len(), 1, "{output}");
        let hso = v["hookSpecificOutput"].as_object().unwrap();
        assert_eq!(hso.len(), 2, "{output}");
        assert_eq!(hso["hookEventName"], "Stop");
        let context = hso["additionalContext"].as_str().unwrap();
        assert!(
            context.starts_with("noslop が guide.md に 独自ルールの指摘を 1 件"),
            "{context}"
        );
        assert!(context.contains("  - L5: ") && !context.contains("L3:"));
        assert!(context.contains(KEEP_NOTE) && context.contains(LIMITED_NOTE));
        assert!(context.chars().count() <= CONTEXT_BUDGET_CHARS);

        assert_eq!(respond(&stop_event(dir.path(), true)), None);
    }

    /// Stop の入力の cwd が git の外なら何もしない。
    #[test]
    fn claude_code_stop_outside_git_returns_nothing() {
        if !git_available() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("guide.md"), DOC).unwrap();
        if WorkTree::discover(dir.path()).unwrap().is_some() {
            return;
        }
        let out = stop(
            &stop_event(dir.path(), false),
            &hook_args(&[]),
            &cli::Environment::default(),
        )
        .unwrap();
        assert_eq!(out, None);
    }

    /// 設定の誤りは誤りとして返す (git の外では設定を読まない)。
    #[test]
    fn broken_config_is_an_error_inside_a_repository() {
        let Some(dir) = repository() else { return };
        write(dir.path(), "noslop.toml", "[files\n");
        write(dir.path(), "guide.md", &edited_doc());
        let err = review_git_diff(
            &git_diff_args(&[]),
            dir.path(),
            &cli::Environment::default(),
        )
        .unwrap_err();
        assert!(err.contains("noslop.toml"), "{err}");
    }

    #[test]
    fn fit_keeps_the_notes() {
        let brief: String = (0..200)
            .map(|i| format!("{i} 行目の指摘です。\n"))
            .collect();
        let notes = "注記です。\n";
        let text = fit(&brief, notes, 1_000);
        assert!(text.chars().count() <= 1_000, "{}", text.chars().count());
        assert!(text.ends_with(notes) && text.contains("残り"), "{text}");
        assert_eq!(fit("短い。\n", notes, 1_000), "短い。\n注記です。\n");
        let tiny = fit(&brief, notes, 200);
        assert!(tiny.chars().count() <= 200, "{tiny}");
    }
}
