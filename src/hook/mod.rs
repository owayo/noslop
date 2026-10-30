//! エージェントのフック (`noslop hook claude-code` / `noslop hook command` / `noslop hook file` /
//! `noslop hook git-diff`)。
//!
//! - `claude-code` ([`claude_code`]): Claude Code のフックの入力 (JSON) を受け、イベントごとに検査する。
//!   PostToolUse (Write / Edit / MultiEdit) は書き換えたファイル、PreToolUse (Bash) は gws で Google
//!   ドキュメント・スプレッドシートに書き込む値、Stop はリポジトリの差分を見る。
//! - `command` ([`command()`]): claw-hooks のコマンドフックの判定器。claw-hooks が解析した gws の呼び出し
//!   1 つの引数を受け取り、書き込む値を検査する (検査は claude-code の PreToolUse と同じ)。
//! - `file` ([`file`]): 編集したファイルのパスだけを渡すフックの仕組み (claw-hooks の extension_hooks
//!   など) から呼ぶ。変わった行は git の差分 (HEAD との比較) から求め、結果をテキストで返す。
//! - `git-diff` ([`git_diff`]): フックの入力を渡せない Stop の仕組み (claw-hooks の stop_hooks など) から
//!   呼ぶ。リポジトリの差分を検査し、指摘があれば終了コード 1 にする。
//!
//! ファイルと差分のフックは、変わった行に重なる指摘だけを短い改稿指示 (brief) で返す (編集のたびに同じ
//! 指摘を繰り返し渡して、残すと決めた箇所まで直させないため)。指摘がないとき・対象外のときは何も
//! 出力しない。
//!
//! どのフックも、重大度が情報で手掛かりが弱い指摘 ([`output::brief::is_weak_info`]) は詳しく返さず、
//! 件数と全件の確認コマンドだけを添える (人の文書にもよく出る軽い指摘まで、編集のたびに直させない
//! ため)。省いた後に返す指摘がなければ、指摘がないときと同じに何も出力しない。`--brief-limit unlimited`
//! (全件) のときは省かない。確認コマンドはどれも `--brief-limit unlimited` を付けるので、省いた指摘も
//! 返す。gws の書き込みを止めるかどうかは、省く前の指摘で決める (止めるのは本文に警告以上があるときで、
//! 省く指摘は情報だけ)。

mod claude_code;
mod command;
mod file;
mod gws;
mod stop;

pub use claude_code::{claude_code, respond};
pub use command::command;
pub use file::file;
pub use stop::git_diff;

use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

use crate::changed::ChangeScope;
use crate::cli::{self, HookArgs};
use crate::diagnostic::Diagnostic;
use crate::document::{Document, ParseOptions, SourceFormat};
use crate::engine::{Engine, FileReport, RunReport};
use crate::output::{self, RenderOptions, RuleCatalog};
use crate::walk::WalkOptions;

/// additionalContext に入れる文字数の上限。Claude Code は 10,000 文字を超えた値を
/// ファイルに逃がして先頭しか見せないので、余裕を見てそれより短く切る。
const CONTEXT_BUDGET_CHARS: usize = 9_000;

/// フックの入力 (標準入力の JSON) の上限。Write の入力には本文がまるごと入る。
const MAX_INPUT_BYTES: u64 = 32 * 1024 * 1024;

/// 検査するファイルの上限。これより大きいファイルは、編集のたびに検査すると遅いので飛ばす。
const MAX_FILE_BYTES: u64 = 8 * 1024 * 1024;

/// 改稿指示の後ろに添える注記 (どちらのフックも)。
const KEEP_NOTE: &str = "このフックは編集のたびに動きます。一度見直して残すと決めた指摘は、再び出ても直す必要はありません。\n";

/// フックの検査の道具一式 (対象の選び方とエンジン)。設定を 1 度だけ読み、1 回のフックの中で
/// 複数のファイルや文書を検査するのに使い回す。
struct Reviewer {
    walk: WalkOptions,
    engine: Engine,
    brief_limit: usize,
    /// 重大度が情報で手掛かりが弱い指摘を、詳しく返さずに件数だけにするか (`--brief-limit unlimited`
    /// でなければ省く)。
    omits_weak_info: bool,
}

impl Reviewer {
    /// 設定ファイルを `cwd` から親へ探して読み、フックの引数を重ねてエンジンを作る。
    /// 読みやすさのレーンは、`--include-readability` がなければ止める。
    fn new(args: &HookArgs, cwd: Option<&Path>, env: &cli::Environment) -> Result<Self, String> {
        let cfg = cli::load_config_from(&args.config, cwd, env).map_err(|e| e.to_string())?;
        let walk = cli::walk_options(&cfg).map_err(|e| e.to_string())?;
        let mut options = cli::config_engine_options(&cfg, env);
        if let Some(genre) = args.genre {
            options.genre = genre;
        }
        options.selection.no_readability = !args.include_readability;
        let engine = Engine::new(options).map_err(|e| e.to_string())?;
        Ok(Self {
            walk,
            engine,
            brief_limit: args.brief_limit,
            // 全件 (unlimited) を求められたら、弱い手掛かりの情報の指摘も省かない。全件の確認コマンドは
            // この値を渡して、省いた指摘を返す
            omits_weak_info: args.brief_limit != usize::MAX,
        })
    }

    /// 改稿指示に詳しく載せる指摘か (抑制していない指摘のうち、省く指摘でないもの)。
    fn shows(&self, d: &Diagnostic) -> bool {
        !d.is_suppressed() && !(self.omits_weak_info && output::brief::is_weak_info(d))
    }

    /// `root` を起点に検査するとき、`path` を検査するか (文書・コードの拡張子、設定の除外、
    /// `.noslopignore`)。
    fn selects(&self, root: &Path, path: &Path) -> bool {
        crate::changed::selects(&self.walk, root, path)
    }

    /// ファイルを読んで文書を組み立てる。消えたファイルと大きすぎるファイルは `None`。
    fn read_document(&self, path: &Path, name: String) -> Result<Option<Document>, String> {
        let Some(source) = read_source(path, &name)? else {
            return Ok(None);
        };
        Ok(Some(Document::parse(
            name,
            source,
            SourceFormat::from_path(path),
            self.parse_options(),
        )))
    }

    /// gws で書き込む値を検査する。宛先の全文を取得しないため、全文専用のルールは動かさない。
    fn lint(&self, doc: Document) -> FileReport {
        self.engine.lint_with(doc, crate::engine::Coverage::Partial)
    }

    /// 文書の読み込み方の設定 (改行の扱い)。値から文書を組み立てるときに使う。
    fn parse_options(&self) -> &ParseOptions {
        &self.engine.options().parse
    }

    /// 検査の結果を短い改稿指示にする。詳しく載せる指摘が 1 件もなければ `None` (弱い手掛かりの情報の
    /// 指摘だけのファイルは、指摘のないファイルと同じに扱う)。`followup` は、省いた件数に添える全件の
    /// 確認コマンド。
    fn brief(
        &self,
        files: Vec<FileReport>,
        followup: Option<&str>,
    ) -> Result<Option<String>, String> {
        let files: Vec<FileReport> = files
            .into_iter()
            .filter(|f| f.diagnostics.iter().any(|d| self.shows(d)))
            .collect();
        if files.is_empty() {
            return Ok(None);
        }
        let opts = RenderOptions {
            genre: self.engine.options().genre,
            brief_limit: Some(self.brief_limit),
            brief_compact: true,
            brief_omit_weak_info: self.omits_weak_info,
            brief_followup: followup.map(str::to_string),
            catalog: RuleCatalog::from_engine(&self.engine),
            ..Default::default()
        };
        let report = RunReport {
            files,
            errors: Vec::new(),
            morphology: self.engine.morphology().clone(),
        };
        let mut buf = Vec::new();
        output::brief::render(&report, &opts, &mut buf).map_err(|e| e.to_string())?;
        String::from_utf8(buf).map(Some).map_err(|e| e.to_string())
    }
}

/// フックの検査の結果。
struct Review {
    /// 短い改稿指示。
    brief: String,
    /// 変わった行に絞ったか (絞れなければファイル全体)。
    limited: bool,
}

/// 編集したファイルを検査し、変更範囲 (`changed` が求める) に重なる指摘の
/// 改稿指示を作る。対象外のファイル (拡張子・除外・なくなった・大きすぎる) と、指摘がないときは `None`。
/// `followup` は、省いた指摘の件数に添える全件の確認コマンド。
fn review(
    path: &Path,
    cwd: Option<&Path>,
    args: &HookArgs,
    env: &cli::Environment,
    followup: &str,
    changed: impl FnOnce(&Document) -> ChangeScope,
) -> Result<Option<Review>, String> {
    let reviewer = Reviewer::new(args, cwd, env)?;
    // ユーザーの設定の除外は、検査の起点 (エージェントの作業ディレクトリ。なければファイルの
    // ディレクトリ) が基準
    let root = cwd.or_else(|| path.parent()).unwrap_or(Path::new("."));
    if !reviewer.selects(root, path) {
        return Ok(None);
    }
    let Some(doc) = reviewer.read_document(path, display_name(path, cwd))? else {
        return Ok(None);
    };
    let scope = changed(&doc);
    let limited = scope.is_limited();
    let file = scope.lint(&reviewer.engine, doc);
    let Some(brief) = reviewer.brief(vec![file], Some(followup))? else {
        return Ok(None);
    };
    Ok(Some(Review { brief, limited }))
}

/// 表示名 (作業ディレクトリの中なら相対パス)。
fn display_name(path: &Path, cwd: Option<&Path>) -> String {
    let relative = cwd
        .and_then(|dir| path.strip_prefix(dir).ok())
        .filter(|p| !p.as_os_str().is_empty());
    crate::walk::display(relative.unwrap_or(path))
}

/// フックの入力 (標準入力) を読む。
fn read_input() -> Result<String, String> {
    let mut input = String::new();
    match io::stdin()
        .take(MAX_INPUT_BYTES + 1)
        .read_to_string(&mut input)
    {
        Ok(n) if n as u64 > MAX_INPUT_BYTES => Err(format!(
            "フックの入力が大きすぎます (上限 {} MiB)",
            MAX_INPUT_BYTES / 1024 / 1024
        )),
        Ok(_) => Ok(input),
        Err(e) => Err(format!("フックの入力を読めません: {e}")),
    }
}

/// 検査するファイルを読む。消えたファイルと大きすぎるファイルは飛ばす (`None`)。
fn read_source(path: &Path, name: &str) -> Result<Option<String>, String> {
    let file = match File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("{name} を読み込めません: {e}")),
    };
    let mut bytes = Vec::new();
    file.take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("{name} を読み込めません: {e}"))?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        // 終了コード 0 の標準エラーは Claude Code のデバッグログにだけ残る
        eprintln!(
            "noslop: {name} は {} MiB を超えるので検査しません",
            MAX_FILE_BYTES / 1024 / 1024
        );
        return Ok(None);
    }
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|_| format!("{name} を UTF-8 として読めません"))
}

/// `budget` 文字に収まるよう、行単位で後ろを省く。1 行が長すぎるときは行の途中で切る。
fn truncate_lines(text: &str, budget: usize, followup: Option<&str>) -> String {
    if text.chars().count() <= budget {
        return text.to_string();
    }
    let lines: Vec<&str> = text.lines().collect();
    let note = |remaining: usize| match followup {
        Some(command) => format!(
            "(長いので残り {remaining} 行を省きました。全件は `{command}` で確認できます)\n"
        ),
        None => format!(
            "(長いので残り {remaining} 行を省きました。検査した文章と表示されたルールを確認してください)\n"
        ),
    };
    // 注記は最も長い行数で予約する。案内自体が入らない上限では、省略の事実だけを残す。
    let longest = note(lines.len());
    if longest.chars().count() > budget {
        return "(長いので残りを省きました)\n"
            .chars()
            .take(budget)
            .collect();
    }
    let available = budget - longest.chars().count();
    let mut out = String::new();
    let mut used = 0;
    for (i, line) in lines.iter().enumerate() {
        let n = line.chars().count() + 1;
        if used + n > available {
            let room = available - used;
            if room > 20 {
                out.extend(line.chars().take(room - 2));
                out.push_str("…\n");
            }
            out.push_str(&note(lines.len() - i));
            return out;
        }
        out.push_str(line);
        out.push('\n');
        used += n;
    }
    out
}

/// 再実行するコマンドの語を、シェルの展開を受けない形で引用する。
fn shell_quote(value: &str) -> String {
    quote_for_shell(value, cfg!(windows))
}

fn quote_for_shell(value: &str, powershell: bool) -> String {
    let escaped = if powershell {
        value.replace('\'', "''")
    } else {
        value.replace('\'', "'\"'\"'")
    };
    format!("'{escaped}'")
}

/// 全件の確認コマンドで検査し直す対象。
#[derive(Debug, Clone, Copy)]
enum Followup<'a> {
    /// `hook file` で検査したファイル。同じフックを上限なしで実行し直す (変わった行の求め方も同じ)。
    File(&'a Path),
    /// Stop と `hook git-diff` の作業ツリー。変わった行を見たなら `check --git-diff`、`--whole-file` なら
    /// 同じフックを上限なしで実行し直す。
    WorkTree,
    /// PostToolUse で書き換えたファイル。ツール呼び出しの差分は再現できないので、`check` でファイルの
    /// 全体を検査する (変わった行に絞った指摘も、その中に含まれる)。
    WholeFile(&'a Path),
}

/// フックのルール選択・設定・実行場所を保った、全件の確認コマンド。どれも `--brief-limit unlimited` を
/// 付けるので、フックが件数だけにした弱い手掛かりの情報の指摘も返す。
fn followup_command(args: &HookArgs, cwd: &Path, target: Followup) -> String {
    let (mut command, check, file) = match target {
        Followup::File(path) => (
            "noslop hook file --max-chars unlimited --brief-limit unlimited".to_string(),
            false,
            Some(path),
        ),
        Followup::WorkTree if args.whole_file => (
            "noslop hook git-diff --max-chars unlimited --brief-limit unlimited".to_string(),
            false,
            None,
        ),
        Followup::WorkTree => (
            "noslop check --git-diff --report brief --brief-limit unlimited".to_string(),
            true,
            None,
        ),
        Followup::WholeFile(path) => (
            "noslop check --report brief --brief-limit unlimited".to_string(),
            true,
            Some(path),
        ),
    };
    if !args.include_readability && check {
        command.push_str(" --no-readability");
    } else if args.include_readability && !check {
        command.push_str(" --include-readability");
    }
    if let Some(genre) = args.genre {
        command.push_str(&format!(" --genre {}", genre.as_str()));
    }
    if args.whole_file && !check {
        command.push_str(" --whole-file");
    }
    if args.config.no_config {
        command.push_str(" --no-config");
    } else if let Some(path) = &args.config.config {
        // 明示した設定はイベントの cwd ではなくプロセスの cwd で読んでいる。
        let path = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
        command.push_str(&format!(
            " --config {}",
            shell_quote(&path.to_string_lossy())
        ));
    }
    if let Some(path) = file {
        command.push_str(&format!(" -- {}", shell_quote(&path.to_string_lossy())));
    }
    let location = shell_quote(&cwd.to_string_lossy());
    if cfg!(windows) {
        format!("& {{ Set-Location -LiteralPath {location} -ErrorAction Stop; {command} }}")
    } else {
        format!("cd {location} && {command}")
    }
}

/// フックのテストで共有する道具。
#[cfg(test)]
mod testing {
    use std::path::Path;
    use std::process::{Command, Stdio};

    use crate::cli;

    /// 独自ルールで「ユーザー様」を指摘する設定 (組み込みルールの増減で壊れないように)。
    pub(super) const CONFIG: &str = r#"
[[custom]]
id = "X01"
name = "TEAM_TERM"
pattern = "ユーザー様"
message = "「ユーザー様」ではなく「利用者」と書きます"
hint = "用語集の表記に合わせてください"
severity = "warning"
"#;

    pub(super) const DOC: &str =
        "# 案内\n\nユーザー様の声を集めました。\n\n新しい段落です。ユーザー様に届けます。\n";

    /// 弱い手掛かりの情報の指摘 (3 行目の P05「様々な」) だけを持つ本文。
    pub(super) const WEAK_DOC: &str = "# 案内\n\n様々な案を集めました。\n";

    /// 弱い手掛かりの情報の指摘 (3 行目の P05「様々な」) と、独自ルールの警告 (5 行目) を持つ本文。
    pub(super) const MIXED_DOC: &str =
        "# 案内\n\n様々な案を集めました。\n\nユーザー様に届けます。\n";

    /// 省いた件数の行から、全件の確認コマンドを取り出す。
    pub(super) fn omitted_followup(text: &str, count: usize) -> &str {
        let head = format!("弱い手掛かりの情報の指摘 {count} 件は省きました。全件は `");
        text.split(head.as_str())
            .nth(1)
            .and_then(|rest| rest.split('`').next())
            .unwrap_or_else(|| panic!("省いた件数の行がありません: {text}"))
    }

    /// `noslop hook claude-code <extra...>` の共通の引数。
    pub(super) fn hook_args(extra: &[&str]) -> cli::HookArgs {
        let mut argv = vec!["claude-code"];
        argv.extend_from_slice(extra);
        match parse(&argv) {
            cli::HookCommand::ClaudeCode(a) => a,
            _ => panic!("hook claude-code"),
        }
    }

    /// `CONFIG` の設定と、本文 `doc` の guide.md を置いた作業ディレクトリ。
    pub(super) fn workspace(doc: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("noslop.toml"), CONFIG).unwrap();
        std::fs::write(dir.path().join("guide.md"), doc).unwrap();
        dir
    }

    /// テスト用の git。手元の git の設定 (署名・フック・既定のブランチ名・改行の変換) に左右されない
    /// ように切る。
    pub(super) fn git(dir: &Path, args: &[&str]) {
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

    pub(super) fn commit(dir: &Path) {
        git(dir, &["add", "-A"]);
        git(dir, &["commit", "-q", "--no-verify", "-m", "c"]);
    }

    /// git を実行できるか (できない環境では、差分を見るテストを飛ばす)。
    pub(super) fn git_available() -> bool {
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

    /// `noslop hook <sub> <extra...>` の引数を読む。
    pub(super) fn parse(argv: &[&str]) -> cli::HookCommand {
        use clap::Parser;
        let mut all = vec!["noslop", "hook"];
        all.extend_from_slice(argv);
        match cli::Cli::try_parse_from(all).unwrap().command {
            cli::Command::Hook(command) => command,
            _ => panic!("hook"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::{MIXED_DOC, WEAK_DOC, hook_args, workspace};
    use super::*;
    use crate::diagnostic::{Severity, Signal};

    fn reviewer(dir: &Path, extra: &[&str]) -> Reviewer {
        Reviewer::new(&hook_args(extra), Some(dir), &cli::Environment::default()).unwrap()
    }

    fn lint(reviewer: &Reviewer, text: &str) -> FileReport {
        reviewer.engine.lint(Document::parse(
            "guide.md",
            text,
            SourceFormat::Markdown,
            reviewer.parse_options(),
        ))
    }

    /// フックのテストは、P05 の「様々な」が弱い手掛かりの情報の指摘になることを前提にする (変わったら、
    /// 弱い手掛かりの情報の指摘になる別の例に差し替える)。
    #[test]
    fn the_weak_info_example_is_a_weak_info_finding() {
        let dir = workspace(WEAK_DOC);
        let r = reviewer(dir.path(), &[]);
        let file = lint(&r, WEAK_DOC);
        let findings: Vec<(&str, Severity, Option<Signal>)> = file
            .visible()
            .map(|d| (d.rule_id.as_str(), d.severity, d.signal))
            .collect();
        assert_eq!(findings, vec![("P05", Severity::Info, Some(Signal::Weak))]);
    }

    /// 弱い手掛かりの情報の指摘は件数だけにし、確認コマンドを添える。それしかないファイルは、指摘が
    /// ないときと同じに何も返さない。`--brief-limit unlimited` (全件) では省かない。
    #[test]
    fn weak_info_findings_are_counted_unless_every_finding_is_requested() {
        let dir = workspace(MIXED_DOC);
        let r = reviewer(dir.path(), &[]);
        assert_eq!(r.brief(vec![lint(&r, WEAK_DOC)], Some("noslop")), Ok(None));
        let text = r
            .brief(vec![lint(&r, MIXED_DOC)], Some("noslop check guide.md"))
            .unwrap()
            .unwrap();
        assert!(
            text.starts_with("noslop が guide.md に 独自ルールの指摘を 1 件見つけました。"),
            "省いた指摘は数えない: {text}"
        );
        assert!(!text.contains("P05"), "{text}");
        assert_eq!(testing::omitted_followup(&text, 1), "noslop check guide.md");

        let all = reviewer(dir.path(), &["--brief-limit", "unlimited"]);
        let text = all
            .brief(vec![lint(&all, WEAK_DOC)], Some("noslop"))
            .unwrap()
            .unwrap();
        assert!(
            text.contains("- P05 ") && text.contains(" (AI 臭さ・情報 1 件・弱い手掛かり)"),
            "{text}"
        );
        assert!(!text.contains("省きました"), "{text}");
    }

    /// PostToolUse の確認コマンドは、フックのルールの選び方を保って、check でファイル全体を見る。
    #[test]
    fn whole_file_followup_checks_the_file_with_the_same_selection() {
        let cwd = std::env::temp_dir();
        let target = Path::new("docs/guide.md");
        let command = followup_command(
            &hook_args(&["--genre", "tech", "--whole-file"]),
            &cwd,
            Followup::WholeFile(target),
        );
        assert!(
            command.contains(
                "noslop check --report brief --brief-limit unlimited --no-readability --genre tech -- 'docs/guide.md'"
            ),
            "{command}"
        );
        assert!(
            !command.contains("--whole-file"),
            "check にはない引数: {command}"
        );
        let command = followup_command(
            &hook_args(&["--include-readability"]),
            &cwd,
            Followup::WholeFile(target),
        );
        assert!(
            !command.contains("readability"),
            "check は既定で読みやすさも見る: {command}"
        );
    }

    #[test]
    fn shell_arguments_are_literal_in_posix_and_powershell() {
        let value = "案内 '別名' $HOME `date` $(command)";
        assert_eq!(
            quote_for_shell(value, false),
            "'案内 '\"'\"'別名'\"'\"' $HOME `date` $(command)'"
        );
        assert_eq!(
            quote_for_shell(value, true),
            "'案内 ''別名'' $HOME `date` $(command)'"
        );
    }

    #[test]
    fn long_context_is_cut_within_the_budget() {
        let text: String = (0..500).map(|i| format!("{i} 行目の文です。\n")).collect();
        let cut = truncate_lines(&text, 1_000, Some("noslop hook git-diff"));
        assert!(cut.chars().count() <= 1_000, "{}", cut.chars().count());
        assert!(cut.ends_with("確認できます)\n"), "{cut}");
        assert_eq!(truncate_lines("短い。\n", 1_000, None), "短い。\n");

        let long = format!("{}\n次の行\n", "長".repeat(5_000));
        let cut = truncate_lines(&long, 1_000, None);
        assert!(cut.starts_with("長長"), "1 行目が長くても先頭は残す: {cut}");
        assert!(cut.chars().count() <= 1_000);
        assert!(cut.contains("検査した文章と表示されたルール"), "{cut}");
        assert!(
            !cut.contains("noslop check") && !cut.contains("再利用"),
            "{cut}"
        );

        for budget in 0..200 {
            for followup in [
                None,
                Some("noslop check --git-diff --report brief --brief-limit unlimited"),
            ] {
                let cut = truncate_lines(&long, budget, followup);
                assert!(cut.chars().count() <= budget, "{budget}: {cut}");
            }
        }
    }
}
