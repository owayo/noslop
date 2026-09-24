//! コマンドライン。
//!
//! 終了コード:
//!
//! - 0: 完了 (指摘の有無は問わない)
//! - 1: `--fail-on` に指定した重大度以上の、抑制していない指摘がある
//! - 2: 引数・設定・入出力のエラー (読めないファイルがあっても他のファイルは処理して出力する)

use std::io::{self, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use anstream::{AutoStream, ColorChoice};
use clap::{Args, Parser, Subcommand, ValueEnum};
use unicode_width::UnicodeWidthStr;

use crate::config::{self, ConfigError, FailOn, LoadedConfig};
use crate::diagnostic::{Lane, RuleStatus, Severity};
use crate::document::{ParseOptions, SourceFormat};
use crate::engine::{Engine, EngineOptions, Input, RuleEntry, Selection};
use crate::genre::Genre;
use crate::morph::{MorphologyMode, MorphologyOptions};
use crate::output::{self, RenderOptions, RuleCatalog};
use crate::rules::Scope;
use crate::segment::LineBreakMode;
use crate::walk::{self, Exclude, WalkOptions};

#[derive(Debug, Parser)]
#[command(
    name = "noslop",
    version,
    about = "日本語の文章から AI 臭さ (slop) を検出する Linter",
    long_about = "日本語の文章から、LLM が書いた文章に特有の言い回しや単調なリズム (AI 臭さ) を検出します。\n検出は疑いの提示です。直すかどうかは書き手が判断してください。",
    propagate_version = true
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Markdown・テキストを検査する
    #[command(visible_alias = "lint")]
    Check(CheckArgs),
    /// 改稿の前後を比べる (新しく出た指摘・消えた数字や固有名詞・改稿の偏り)
    ///
    /// 同じ設定で両方を検査し、指摘の増減に加えて、改稿で失われた事実と、同じ変換を
    /// 文書全体に当てた形跡を確認事項として出す。確認事項があっても終了コードは 0。
    Diff(DiffArgs),
    /// 人の文書と生成文書のコーパスで、ルールの誤検知率・検出率と閾値を測る
    ///
    /// 閾値や状態は書き換えず、候補を挙げるだけ。手順は docs/calibration.md を参照。
    Calibrate(CalibrateArgs),
    /// ルールの一覧を表示する
    Rules(RulesArgs),
    /// ルールの詳細 (何を見るか・直し方・根拠) を表示する
    Explain(ExplainArgs),
    /// 設定ファイル noslop.toml のひな形を作る
    Init(InitArgs),
    /// MCP サーバーとして標準入出力で待ち受ける (AI エージェントから検査を呼ぶ)
    ///
    /// ツールは check (本文の検査)・diff (改稿の前後の比較)・explain (ルールの説明)・rules (ルールの一覧)。
    /// 設定ファイルは、環境変数 CLAUDE_PROJECT_DIR があればそこから、なければカレントから親へ探す。
    Mcp(McpArgs),
    /// エージェントのフックから呼ぶ (編集したファイルを検査して指摘を返す)
    #[command(subcommand)]
    Hook(HookCommand),
    /// AI エージェント (Claude Code・Codex CLI) に noslop のスキルを入れる
    ///
    /// 日本語の文章を書いた・直した後に、エージェントが noslop で見直すようになる。
    /// 既定の置き場は ~/.claude/skills/noslop/SKILL.md (claude) か
    /// ~/.codex/skills/noslop/SKILL.md (codex)。すでにあれば上書きする。
    SkillInstall(SkillInstallArgs),
}

/// 設定ファイルの指定。
#[derive(Debug, Clone, Args)]
pub struct ConfigArgs {
    /// 設定ファイルを指定する (省略時はカレントから親へ noslop.toml / .noslop.toml を探す)
    #[arg(long, value_name = "PATH")]
    pub config: Option<PathBuf>,
    /// 設定ファイルを読まない
    #[arg(long, conflicts_with = "config")]
    pub no_config: bool,
}

/// ルールの選び方と文書の読み方 (`check` と `diff` で共通)。
#[derive(Debug, Clone, Args)]
pub struct EngineArgs {
    /// 文書のジャンル: general / tech / business / essay (別名 blog / minutes)
    #[arg(long, value_parser = parse_genre, value_name = "GENRE")]
    pub genre: Option<Genre>,
    /// 止めるルール (ID か名前、カンマ区切り)
    #[arg(long, value_delimiter = ',', value_name = "IDS")]
    pub ignore_rules: Vec<String>,
    /// 有効にするルール (実験的なルールも個別に有効にできる)
    #[arg(long, value_delimiter = ',', value_name = "IDS")]
    pub enable_rules: Vec<String>,
    /// 指定したルールだけを動かす (実験的なルールも動く)
    #[arg(long, value_delimiter = ',', value_name = "IDS")]
    pub only_rules: Vec<String>,
    /// 実験的な (未校正の) ルールと語句もすべて動かす
    #[arg(long)]
    pub experimental: bool,
    /// 読みやすさのルールを止める
    #[arg(long)]
    pub no_readability: bool,
    /// 語句ルールをリスト・表・引用にも当てる
    #[arg(long, value_enum, value_delimiter = ',', value_name = "KINDS")]
    pub include: Vec<IncludeArg>,
    /// 段落内の改行の扱い
    #[arg(long, value_enum, value_name = "MODE")]
    pub line_breaks: Option<LineBreaksArg>,
    /// 形態素解析の辞書 (hasami の .hsd)。指定するとこの辞書を必ず使い、品詞で判定する
    #[arg(long, value_name = "PATH", conflicts_with = "no_dict")]
    pub dict: Option<PathBuf>,
    /// 形態素解析の辞書を使わず、辞書なしの近似で判定する
    #[arg(long)]
    pub no_dict: bool,
}

#[derive(Debug, Args)]
pub struct CheckArgs {
    /// 検査するファイルかディレクトリ ("-" で標準入力)
    #[arg(value_name = "PATH", default_value = ".")]
    pub paths: Vec<PathBuf>,
    /// 出力形式 (既定は text。`--report brief` のときは markdown)
    #[arg(short = 'f', long, value_enum)]
    pub format: Option<FormatArg>,
    /// 出力する内容: full (全指摘のレポート) か brief (直す箇所をルールごとにまとめた改稿指示)
    #[arg(long, value_enum, value_name = "KIND")]
    pub report: Option<ReportArg>,
    #[command(flatten)]
    pub engine: EngineArgs,
    #[command(flatten)]
    pub config: ConfigArgs,
    /// この重大度以上の指摘があれば終了コード 1 にする
    #[arg(long, value_enum, value_name = "LEVEL")]
    pub fail_on: Option<FailOnArg>,
    /// 標準入力 ("-") を読むときの表示名 (拡張子で形式を判断する)
    #[arg(long, value_name = "NAME")]
    pub stdin_filename: Option<String>,
    /// 抑制した指摘も表示する (text 形式)
    #[arg(long)]
    pub show_suppressed: bool,
    /// 色付けする
    #[arg(long, value_enum, default_value_t = ColorArg::Auto, value_name = "WHEN")]
    pub color: ColorArg,
    /// 指摘のないファイルとサマリを表示しない
    #[arg(short = 'q', long)]
    pub quiet: bool,
    /// brief 形式で、1 ルールあたりに並べる箇所の上限 (既定 5)
    #[arg(long, value_parser = parse_limit, value_name = "N")]
    pub brief_limit: Option<usize>,
}

#[derive(Debug, Args)]
pub struct DiffArgs {
    /// 改稿前のファイル ("-" で標準入力)
    #[arg(value_name = "BEFORE")]
    pub before: PathBuf,
    /// 改稿後のファイル ("-" で標準入力)
    #[arg(value_name = "AFTER")]
    pub after: PathBuf,
    /// 出力形式
    #[arg(short = 'f', long, value_enum, default_value_t = DiffFormatArg::Text)]
    pub format: DiffFormatArg,
    #[command(flatten)]
    pub engine: EngineArgs,
    #[command(flatten)]
    pub config: ConfigArgs,
    /// 標準入力 ("-") を読むときの表示名 (拡張子で形式を判断する)
    #[arg(long, value_name = "NAME")]
    pub stdin_filename: Option<String>,
    /// 色付けする
    #[arg(long, value_enum, default_value_t = ColorArg::Auto, value_name = "WHEN")]
    pub color: ColorArg,
}

#[derive(Debug, Args)]
pub struct CalibrateArgs {
    /// 人の書いた文書 (ファイルかディレクトリ。繰り返して複数指定できる)
    #[arg(long, required = true, value_name = "PATH")]
    pub human: Vec<PathBuf>,
    /// 生成された文書 (ファイルかディレクトリ。繰り返して複数指定できる)
    #[arg(long, required = true, value_name = "PATH")]
    pub ai: Vec<PathBuf>,
    /// 出力形式 (記録を残すなら markdown)
    #[arg(short = 'f', long, value_enum, default_value_t = CalibrateFormatArg::Text)]
    pub format: CalibrateFormatArg,
    /// コーパスのジャンル: general / tech / business / essay (別名 blog / minutes)
    #[arg(long, value_parser = parse_genre, value_name = "GENRE")]
    pub genre: Option<Genre>,
    /// 許容する誤検知率 (0 以上 1 以下)
    #[arg(long, default_value_t = crate::calibrate::DEFAULT_TARGET_FP, value_name = "RATE")]
    pub target_fp: f64,
    /// 検証用に取り分ける文書の割合 (0 以上 1 未満)
    #[arg(long, default_value_t = crate::calibrate::DEFAULT_HOLDOUT, value_name = "RATE")]
    pub holdout: f64,
    /// 校正済みに上げる候補とする検出率の下限 (0 以上 1 以下)
    #[arg(long, default_value_t = crate::calibrate::DEFAULT_MIN_DETECTION, value_name = "RATE")]
    pub min_detection: f64,
    /// 実験的なルールと語句を測らない (既定では既定の動作と別に測る)
    #[arg(long)]
    pub no_experimental: bool,
    #[command(flatten)]
    pub config: ConfigArgs,
}

#[derive(Debug, Args)]
pub struct RulesArgs {
    /// 出力形式 (markdown はドキュメント生成用に説明文も含める)
    #[arg(short = 'f', long, value_enum, default_value_t = RulesFormatArg::Text)]
    pub format: RulesFormatArg,
    /// ジャンル (既定で有効かの判定に使う)
    #[arg(long, value_parser = parse_genre, value_name = "GENRE")]
    pub genre: Option<Genre>,
    /// 実験的なルールも有効として表示する
    #[arg(long)]
    pub experimental: bool,
    #[command(flatten)]
    pub config: ConfigArgs,
}

#[derive(Debug, Args)]
pub struct ExplainArgs {
    /// ルールの ID か名前 (例: P01、AI_CONCLUSION)
    #[arg(value_name = "RULE")]
    pub rule: String,
    /// ジャンル (閾値と既定で有効かの判定に使う)
    #[arg(long, value_parser = parse_genre, value_name = "GENRE")]
    pub genre: Option<Genre>,
    #[command(flatten)]
    pub config: ConfigArgs,
}

#[derive(Debug, Args)]
pub struct InitArgs {
    /// すでにある noslop.toml を上書きする
    #[arg(long)]
    pub force: bool,
}

#[derive(Debug, Args)]
pub struct McpArgs {
    #[command(flatten)]
    pub config: ConfigArgs,
}

#[derive(Debug, Subcommand)]
pub enum HookCommand {
    /// Claude Code の PostToolUse フック (Write / Edit / MultiEdit の後に検査して、指摘を Claude に渡す)
    ///
    /// 標準入力でフックの入力 (JSON) を受け取り、指摘があれば additionalContext を標準出力に書く。
    /// 対象外のツール・ファイルや指摘がないときは何も書かない。設定ファイルは入力の cwd から親へ探す。
    ClaudeCode(ClaudeCodeArgs),
}

#[derive(Debug, Clone, Args)]
pub struct ClaudeCodeArgs {
    /// 1 ルールあたりに返す箇所の上限
    #[arg(long, default_value_t = 3, value_parser = parse_limit, value_name = "N")]
    pub brief_limit: usize,
    /// 読みやすさの指摘も返す
    #[arg(long)]
    pub include_readability: bool,
    /// 実験的な (未校正の) ルールと語句も動かす
    #[arg(long)]
    pub experimental: bool,
    /// 文書のジャンル: general / tech / business / essay (別名 blog / minutes)
    #[arg(long, value_parser = parse_genre, value_name = "GENRE")]
    pub genre: Option<Genre>,
    /// 今回変わった行に限らず、ファイル全体の指摘を返す
    #[arg(long)]
    pub whole_file: bool,
    #[command(flatten)]
    pub config: ConfigArgs,
}

#[derive(Debug, Args)]
pub struct SkillInstallArgs {
    /// スキルを入れる AI エージェント
    #[arg(value_enum, value_name = "TARGET")]
    pub target: SkillTarget,
    /// スキルの置き場 (既定は ~/.claude/skills か ~/.codex/skills。
    /// プロジェクトに置くなら .claude/skills など)
    #[arg(long, value_name = "DIR")]
    pub dir: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum SkillTarget {
    /// Claude Code
    #[value(alias = "claude-code")]
    Claude,
    /// Codex CLI
    Codex,
}

impl From<SkillTarget> for crate::skill::Target {
    fn from(v: SkillTarget) -> Self {
        match v {
            SkillTarget::Claude => Self::Claude,
            SkillTarget::Codex => Self::Codex,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum FormatArg {
    /// 端末向け
    Text,
    /// 機械可読な JSON
    Json,
    /// JSON と同じ内容を少ないトークンで表す TOON (LLM に渡す用途)
    Toon,
    /// GitHub Actions の注釈
    #[value(alias = "github-actions")]
    Github,
    /// 改稿指示の Markdown (`--report brief` の既定)
    Markdown,
    /// 改稿指示の Markdown (`--report brief --format markdown` の省略形)
    Brief,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ReportArg {
    /// 全指摘のレポート (抑制した指摘・位置・metrics・fingerprint を含む)
    Full,
    /// 直す箇所をルールごとにまとめた改稿指示 (AI や編集者に渡す)
    Brief,
}

/// `check` が出すもの (内容と形式の組)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CheckOutput {
    Text,
    Json,
    Toon,
    Github,
    BriefMarkdown,
    BriefJson,
    BriefToon,
}

/// `--report` と `--format` から出すものを決める。組み合わせられないものはエラーにする。
fn check_output(
    report: Option<ReportArg>,
    format: Option<FormatArg>,
) -> Result<CheckOutput, &'static str> {
    use CheckOutput as O;
    use FormatArg as F;
    use ReportArg as R;
    // 内容を省いたら形式から決める (markdown・brief なら改稿指示、それ以外は全指摘のレポート)
    let report = report.unwrap_or(match format {
        Some(F::Markdown | F::Brief) => R::Brief,
        _ => R::Full,
    });
    match (report, format) {
        (R::Full, None | Some(F::Text)) => Ok(O::Text),
        (R::Full, Some(F::Json)) => Ok(O::Json),
        (R::Full, Some(F::Toon)) => Ok(O::Toon),
        (R::Full, Some(F::Github)) => Ok(O::Github),
        (R::Full, Some(F::Markdown | F::Brief)) => Err(
            "Markdown で出せるのは改稿指示 (--report brief) だけです。全指摘のレポートは text・json・toon・github で出せます",
        ),
        (R::Brief, None | Some(F::Markdown | F::Brief)) => Ok(O::BriefMarkdown),
        (R::Brief, Some(F::Json)) => Ok(O::BriefJson),
        (R::Brief, Some(F::Toon)) => Ok(O::BriefToon),
        (R::Brief, Some(F::Text | F::Github)) => {
            Err("改稿指示 (--report brief) は markdown・json・toon で出せます")
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum RulesFormatArg {
    Text,
    Json,
    Markdown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum DiffFormatArg {
    /// 端末向け
    Text,
    /// 機械可読な JSON
    Json,
    /// JSON と同じ内容を少ないトークンで表す TOON (LLM に渡す用途)
    Toon,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum CalibrateFormatArg {
    /// 端末向け
    Text,
    /// 機械可読な JSON
    Json,
    /// 記録に残す Markdown
    Markdown,
}

impl From<CalibrateFormatArg> for crate::calibrate::ReportFormat {
    fn from(v: CalibrateFormatArg) -> Self {
        match v {
            CalibrateFormatArg::Text => Self::Text,
            CalibrateFormatArg::Json => Self::Json,
            CalibrateFormatArg::Markdown => Self::Markdown,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum IncludeArg {
    Lists,
    Tables,
    Quotes,
    All,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum LineBreaksArg {
    /// 改行を文の区切りにしない
    Space,
    /// 改行で文を区切る (句点を打たない一文一行の文書向け)
    Sentence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum FailOnArg {
    Never,
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ColorArg {
    Auto,
    Always,
    Never,
}

fn parse_genre(s: &str) -> Result<Genre, String> {
    s.parse()
}

fn parse_limit(s: &str) -> Result<usize, String> {
    match s.parse::<usize>() {
        Ok(n) if n >= 1 => Ok(n),
        _ => Err("1 以上の整数を指定してください".to_string()),
    }
}

impl From<FailOnArg> for FailOn {
    fn from(v: FailOnArg) -> Self {
        match v {
            FailOnArg::Never => FailOn::Never,
            FailOnArg::Info => FailOn::At(Severity::Info),
            FailOnArg::Warning => FailOn::At(Severity::Warning),
            FailOnArg::Error => FailOn::At(Severity::Error),
        }
    }
}

impl From<LineBreaksArg> for LineBreakMode {
    fn from(v: LineBreaksArg) -> Self {
        match v {
            LineBreaksArg::Space => LineBreakMode::Space,
            LineBreaksArg::Sentence => LineBreakMode::Sentence,
        }
    }
}

const EXIT_OK: u8 = 0;
const EXIT_FAILED: u8 = 1;
const EXIT_ERROR: u8 = 2;

/// 標準出力に書く前に溜めるバッファの初期容量。
const OUTPUT_BUFFER_BYTES: usize = 64 * 1024;

/// コマンドラインを解釈して実行する。
pub fn run() -> ExitCode {
    let cli = Cli::parse();
    let code = match cli.command {
        Command::Check(args) => check(args),
        Command::Diff(args) => diff(args),
        Command::Calibrate(args) => calibrate(args),
        Command::Rules(args) => rules(args),
        Command::Explain(args) => explain(args),
        Command::Init(args) => init(args),
        Command::Mcp(args) => mcp(args),
        Command::Hook(HookCommand::ClaudeCode(args)) => crate::hook::claude_code(&args),
        Command::SkillInstall(args) => skill_install(args),
    };
    ExitCode::from(code)
}

fn error(message: impl std::fmt::Display) -> u8 {
    eprintln!("エラー: {message}");
    EXIT_ERROR
}

/// 設定ファイルを探して読み込む (`--config` がなければカレントから親へ探す)。
pub(crate) fn load_config(args: &ConfigArgs) -> Result<Option<LoadedConfig>, ConfigError> {
    load_config_from(args, None)
}

/// 設定ファイルを探して読み込む。`--config` がなければ `start` (省略時はカレント) から親へ探す。
pub(crate) fn load_config_from(
    args: &ConfigArgs,
    start: Option<&Path>,
) -> Result<Option<LoadedConfig>, ConfigError> {
    if args.no_config {
        return Ok(None);
    }
    let path = match &args.config {
        Some(p) => Some(p.clone()),
        None => {
            let dir = match start {
                Some(dir) => dir.to_path_buf(),
                None => std::env::current_dir().map_err(|source| ConfigError::Io {
                    path: PathBuf::from("."),
                    source,
                })?,
            };
            config::discover(&dir)
        }
    };
    path.map(|p| config::load(&p)).transpose()
}

/// 設定ファイルだけからエンジンの設定を組み立てる (CLI の指定は反映しない)。
pub(crate) fn config_engine_options(cfg: Option<&LoadedConfig>) -> EngineOptions {
    let file = cfg.map(|c| c.file.clone()).unwrap_or_default();
    EngineOptions {
        genre: file.genre.unwrap_or_default(),
        experimental: file.experimental.unwrap_or(false),
        scope: Scope {
            lists: file.scope.lists.unwrap_or(false),
            tables: file.scope.tables.unwrap_or(false),
            blockquotes: file.scope.blockquotes.unwrap_or(false),
        },
        parse: ParseOptions {
            line_breaks: file.line_breaks.unwrap_or_default(),
        },
        selection: Selection {
            config_enable: file.rules.enable,
            config_disable: file.rules.disable,
            ..Default::default()
        },
        rule_tables: file.rules.tables,
        custom: file.custom,
        morphology: MorphologyOptions {
            mode: file.morphology.mode.unwrap_or_default(),
            dictionary: file
                .morphology
                .dictionary
                .map(|path| config_relative_path(cfg, path)),
        },
    }
}

/// 設定ファイルに書いたパスを解決する (`~/` はホームディレクトリ、相対パスは設定ファイルの
/// ディレクトリが基準)。
fn config_relative_path(cfg: Option<&LoadedConfig>, path: PathBuf) -> PathBuf {
    if let (Ok(rest), Some(home)) = (path.strip_prefix("~"), std::env::home_dir()) {
        return home.join(rest);
    }
    match cfg.and_then(|c| c.path.parent()) {
        Some(dir) if path.is_relative() => dir.join(path),
        _ => path,
    }
}

/// 設定ファイルと CLI からエンジンの設定を組み立てる (CLI の指定が優先)。
fn engine_options(args: &EngineArgs, cfg: Option<&LoadedConfig>) -> EngineOptions {
    let mut options = config_engine_options(cfg);
    if let Some(genre) = args.genre {
        options.genre = genre;
    }
    options.experimental |= args.experimental;
    for kind in &args.include {
        match kind {
            IncludeArg::Lists => options.scope.lists = true,
            IncludeArg::Tables => options.scope.tables = true,
            IncludeArg::Quotes => options.scope.blockquotes = true,
            IncludeArg::All => options.scope = Scope::ALL,
        }
    }
    if let Some(mode) = args.line_breaks {
        options.parse.line_breaks = mode.into();
    }
    if args.no_dict {
        options.morphology.mode = MorphologyMode::Off;
    } else if let Some(path) = &args.dict {
        options.morphology = MorphologyOptions {
            mode: MorphologyMode::Required,
            dictionary: Some(path.clone()),
        };
    }
    let selection = &mut options.selection;
    selection.cli_enable = args.enable_rules.clone();
    selection.cli_disable = args.ignore_rules.clone();
    selection.only = (!args.only_rules.is_empty()).then(|| args.only_rules.clone());
    selection.no_readability = args.no_readability;
    options
}

pub(crate) fn walk_options(cfg: Option<&LoadedConfig>) -> Result<WalkOptions, ConfigError> {
    let mut options = WalkOptions::default();
    let Some(cfg) = cfg else {
        return Ok(options);
    };
    if let Some(ext) = &cfg.file.files.extensions {
        options.extensions = ext
            .iter()
            .map(|e| e.trim().trim_start_matches('.').to_ascii_lowercase())
            .filter(|e| !e.is_empty())
            .collect();
    }
    if let Some(patterns) = &cfg.file.files.exclude
        && !patterns.is_empty()
    {
        options.exclude = Some(Arc::new(Exclude::new(&cfg.base_dir, patterns)?));
    }
    Ok(options)
}

fn check(args: CheckArgs) -> u8 {
    let output = match check_output(args.report, args.format) {
        Ok(output) => output,
        Err(e) => return error(e),
    };
    let cfg = match load_config(&args.config) {
        Ok(cfg) => cfg,
        Err(e) => return error(e),
    };
    let options = engine_options(&args.engine, cfg.as_ref());
    let fail_on = args
        .fail_on
        .map(FailOn::from)
        .or(cfg.as_ref().and_then(|c| c.file.fail_on))
        .unwrap_or_default();
    let walk_opts = match walk_options(cfg.as_ref()) {
        Ok(w) => w,
        Err(e) => return error(e),
    };
    let engine = match Engine::new(options) {
        Ok(e) => e,
        Err(e) => return error(e),
    };
    let render_opts = RenderOptions {
        show_suppressed: args.show_suppressed,
        quiet: args.quiet,
        genre: engine.options().genre,
        experimental: engine.options().experimental,
        fail_on,
        brief_limit: args.brief_limit,
        brief_compact: false,
        catalog: RuleCatalog::from_engine(&engine),
    };

    let mut inputs = Vec::new();
    let mut paths = Vec::new();
    let mut stdin_used = false;
    for path in &args.paths {
        if !is_stdin(path) {
            paths.push(path.clone());
        } else if !stdin_used {
            stdin_used = true;
            match read_stdin(args.stdin_filename.as_deref()) {
                Ok(input) => inputs.push(input),
                Err(e) => return error(e),
            }
        }
    }
    let collected = walk::collect(&paths, &walk_opts);
    for dir in &collected.empty_dirs {
        eprintln!(
            "警告: {dir} には検査するファイルがありません (拡張子と、.gitignore などの除外の指定を確かめてください)"
        );
    }
    inputs.extend(collected.files.into_iter().map(Input::Path));

    let mut report = engine.run(inputs);
    let mut errors: Vec<_> = collected
        .errors
        .into_iter()
        .map(|e| crate::engine::FileError {
            path: e.path,
            message: e.message,
        })
        .collect();
    errors.append(&mut report.errors);
    errors.sort_by(|a, b| a.path.cmp(&b.path));
    report.errors = errors;

    // 標準出力のロックは改行のたびにフラッシュするので、直接書くと整形済み JSON や
    // 大量の指摘では行数ぶんの write システムコールになる。バッファに溜めてから書く。
    let result = match output {
        CheckOutput::Text => {
            for e in &report.errors {
                eprintln!("エラー: {}: {}", e.path, e.message);
            }
            let mut rendered = Vec::with_capacity(OUTPUT_BUFFER_BYTES);
            output::text::render(&report, &render_opts, &mut rendered)
                .and_then(|()| write_styled(&rendered, args.color))
        }
        CheckOutput::Json => write_buffered(|out| output::json::render(&report, &render_opts, out)),
        CheckOutput::Toon => {
            write_buffered(|out| output::json::render_toon(&report, &render_opts, out))
        }
        CheckOutput::Github => write_buffered(|out| output::github::render(&report, out)),
        CheckOutput::BriefMarkdown => {
            write_buffered(|out| output::brief::render(&report, &render_opts, out))
        }
        CheckOutput::BriefJson => {
            write_buffered(|out| output::brief::render_json(&report, &render_opts, out))
        }
        CheckOutput::BriefToon => {
            write_buffered(|out| output::brief::render_toon(&report, &render_opts, out))
        }
    };
    if let Err(code) = output_result(result) {
        return code;
    }

    if !report.errors.is_empty() {
        EXIT_ERROR
    } else if report.trips(fail_on) {
        EXIT_FAILED
    } else {
        EXIT_OK
    }
}

fn is_stdin(path: &Path) -> bool {
    path.as_os_str() == "-"
}

/// 標準入力を 1 つの入力として読む。形式は表示名の拡張子で決める (なければ Markdown)。
fn read_stdin(stdin_filename: Option<&str>) -> Result<Input, String> {
    let mut source = String::new();
    io::stdin()
        .read_to_string(&mut source)
        .map_err(|e| format!("標準入力を読めません: {e}"))?;
    let (name, format) = match stdin_filename {
        Some(name) => (name.to_string(), SourceFormat::from_path(Path::new(name))),
        None => ("<stdin>".to_string(), SourceFormat::Markdown),
    };
    Ok(Input::Text {
        name,
        source,
        format,
    })
}

/// 色付けの指定に従って、書式つきの出力を標準出力に書く。
///
/// 標準出力のロックは改行のたびにフラッシュするので、描画はバッファに溜めてから 1 度に書く。
fn write_styled(rendered: &[u8], color: ColorArg) -> io::Result<()> {
    let stdout = io::stdout();
    let choice = match color {
        ColorArg::Auto => AutoStream::choice(&stdout),
        ColorArg::Always => ColorChoice::Always,
        ColorArg::Never => ColorChoice::Never,
    };
    if choice == ColorChoice::Never {
        let plain = anstream::adapter::strip_bytes(rendered).into_vec();
        let mut out = stdout.lock();
        out.write_all(&plain).and_then(|()| out.flush())
    } else {
        let mut out = AutoStream::new(stdout.lock(), choice);
        out.write_all(rendered).and_then(|()| out.flush())
    }
}

/// バッファに溜めて標準出力に書く (JSON・TOON・Markdown など書式のない出力)。
fn write_buffered(render: impl FnOnce(&mut dyn Write) -> io::Result<()>) -> io::Result<()> {
    let mut out = io::BufWriter::with_capacity(OUTPUT_BUFFER_BYTES, io::stdout().lock());
    render(&mut out).and_then(|()| out.flush())
}

/// 出力の失敗を終了コードにする。読み手が先に閉じたパイプ (`| head` など) は失敗にしない。
fn output_result(result: io::Result<()>) -> Result<(), u8> {
    match result {
        Err(e) if e.kind() != io::ErrorKind::BrokenPipe => {
            Err(error(format!("出力に失敗しました: {e}")))
        }
        _ => Ok(()),
    }
}

fn diff(args: DiffArgs) -> u8 {
    if is_stdin(&args.before) && is_stdin(&args.after) {
        return error("標準入力 (\"-\") は改稿前と改稿後のどちらか一方にしか使えません");
    }
    let cfg = match load_config(&args.config) {
        Ok(cfg) => cfg,
        Err(e) => return error(e),
    };
    let engine = match Engine::new(engine_options(&args.engine, cfg.as_ref())) {
        Ok(e) => e,
        Err(e) => return error(e),
    };
    let input = |path: &Path| {
        if is_stdin(path) {
            read_stdin(args.stdin_filename.as_deref())
        } else {
            Ok(Input::Path(path.to_path_buf()))
        }
    };
    let (before, after) = match (input(&args.before), input(&args.after)) {
        (Ok(before), Ok(after)) => (before, after),
        (Err(e), _) | (_, Err(e)) => return error(e),
    };
    let (before, after) = match crate::diff::lint_pair(&engine, before, after) {
        Ok(pair) => pair,
        Err(errors) => {
            for e in &errors {
                eprintln!("エラー: {}: {}", e.path, e.message);
            }
            return EXIT_ERROR;
        }
    };
    let report = crate::diff::compare(before, after);

    let result = match args.format {
        DiffFormatArg::Text => {
            let mut rendered = Vec::with_capacity(OUTPUT_BUFFER_BYTES);
            crate::diff::render_text(&report, &mut rendered)
                .and_then(|()| write_styled(&rendered, args.color))
        }
        DiffFormatArg::Json => write_buffered(|out| crate::diff::render_json(&report, out)),
        DiffFormatArg::Toon => write_buffered(|out| crate::diff::render_toon(&report, out)),
    };
    match output_result(result) {
        Ok(()) => EXIT_OK,
        Err(code) => code,
    }
}

fn calibrate(args: CalibrateArgs) -> u8 {
    let cfg = match load_config(&args.config) {
        Ok(cfg) => cfg,
        Err(e) => return error(e),
    };
    let walk = match walk_options(cfg.as_ref()) {
        Ok(w) => w,
        Err(e) => return error(e),
    };
    let mut engine = config_engine_options(cfg.as_ref());
    if let Some(genre) = args.genre {
        engine.genre = genre;
    }
    let options = crate::calibrate::CalibrateOptions {
        human: args.human,
        ai: args.ai,
        engine,
        walk,
        include_experimental: !args.no_experimental,
        target_fp: args.target_fp,
        holdout: args.holdout,
        min_detection: args.min_detection,
    };
    let report = match crate::calibrate::calibrate(&options) {
        Ok(report) => report,
        Err(e) => return error(e),
    };

    let result = write_buffered(|out| crate::calibrate::render(&report, args.format.into(), out));
    if let Err(code) = output_result(result) {
        return code;
    }
    // 結果をファイルに書き出したときにも気づけるよう、読めなかった入力は標準エラーにも出す
    for e in &report.errors {
        eprintln!("エラー: {} {}: {}", e.group.label_ja(), e.path, e.message);
    }
    if report.errors.is_empty() {
        EXIT_OK
    } else {
        EXIT_ERROR
    }
}

/// `rules` / `explain` 用のエンジン (選択は設定ファイルとジャンルだけで決める)。
fn listing_engine(
    config_args: &ConfigArgs,
    genre: Option<Genre>,
    experimental: bool,
) -> Result<Engine, ConfigError> {
    let cfg = load_config(config_args)?;
    let mut options = config_engine_options(cfg.as_ref());
    if let Some(genre) = genre {
        options.genre = genre;
    }
    options.experimental |= experimental;
    Engine::new(options)
}

/// explain と `rules --format markdown` に出すレーン (呼び名は [`Lane::label_ja`] にそろえる)。
fn lane_label(lane: Lane) -> String {
    let score = match lane {
        Lane::Slop => "自然度スコアに入る",
        Lane::Readability | Lane::Custom => "自然度スコアに入らない",
    };
    format!("{} ({score})", lane.label_ja())
}

fn status_label(status: RuleStatus) -> &'static str {
    match status {
        RuleStatus::Stable => "校正済み (既定で有効)",
        RuleStatus::Experimental => "実験的 (--experimental か設定で有効にしたときだけ動く)",
    }
}

/// 校正の基準 (人の文書での誤検知率を、どの重大度以上の指摘で数えたか)。
///
/// 校正済みの組み込みルールだけに付ける (実験的なルールと独自ルールは校正していない)。
fn calibration_basis(entry: &RuleEntry) -> Option<Severity> {
    (entry.builtin && entry.rule.meta().status == RuleStatus::Stable)
        .then(|| entry.rule.calibration_basis())
}

fn calibration_basis_label(basis: Severity) -> &'static str {
    match basis {
        Severity::Info => "重大度を問わず数えた誤検知率",
        Severity::Warning => "警告以上の指摘で数えた誤検知率",
        Severity::Error => "重大の指摘で数えた誤検知率",
    }
}

fn pad(s: &str, width: usize) -> String {
    let w = s.width();
    if w >= width {
        s.to_string()
    } else {
        format!("{s}{}", " ".repeat(width - w))
    }
}

fn rules(args: RulesArgs) -> u8 {
    let engine = match listing_engine(&args.config, args.genre, args.experimental) {
        Ok(e) => e,
        Err(e) => return error(e),
    };
    let text = match args.format {
        RulesFormatArg::Text => rules_text(&engine),
        RulesFormatArg::Json => rules_json(&engine),
        RulesFormatArg::Markdown => rules_markdown(&engine),
    };
    let mut out = io::stdout().lock();
    match out.write_all(text.as_bytes()).and_then(|()| out.flush()) {
        Ok(()) => EXIT_OK,
        Err(e) if e.kind() == io::ErrorKind::BrokenPipe => EXIT_OK,
        Err(e) => error(format!("出力に失敗しました: {e}")),
    }
}

pub(crate) fn rules_text(engine: &Engine) -> String {
    let genre = engine.options().genre;
    let mut s = format!("ジャンル: {genre}\n\n");
    let header = format!(
        "{} {} {} {} {} {} {}\n",
        pad("ID", 5),
        pad("名前", 26),
        pad("レーン", 12),
        pad("状態", 13),
        pad("重大度", 8),
        pad("有効", 5),
        "日本語名"
    );
    s.push_str(&header);
    for e in engine.entries() {
        let m = e.rule.meta();
        s.push_str(&format!(
            "{} {} {} {} {} {} {}\n",
            pad(m.id, 5),
            pad(m.name, 26),
            pad(m.lane.as_str(), 12),
            pad(m.status.as_str(), 13),
            pad(e.severity.unwrap_or(m.default_severity).as_str(), 8),
            pad(if e.enabled { "有効" } else { "-" }, 5),
            m.title
        ));
        s.push_str(&format!("      {}\n", m.summary));
    }
    if engine.entries().is_empty() {
        s.push_str("(ルールがありません)\n");
    }
    s
}

pub(crate) fn rules_json(engine: &Engine) -> String {
    let rows: Vec<serde_json::Value> = engine
        .entries()
        .iter()
        .map(|e| {
            let m = e.rule.meta();
            let options: serde_json::Map<String, serde_json::Value> = e
                .rule
                .options()
                .into_iter()
                .map(|(k, v)| (k.to_string(), serde_json::Value::String(v)))
                .collect();
            serde_json::json!({
                "id": m.id,
                "name": m.name,
                "title": m.title,
                "lane": m.lane,
                "status": m.status,
                "defaultSeverity": m.default_severity,
                "severity": e.severity.unwrap_or(m.default_severity),
                "enabled": e.enabled,
                "builtin": e.builtin,
                "allowedInGenre": e.rule.allowed_in(engine.options().genre),
                "calibrationBasis": calibration_basis(e),
                "summary": m.summary,
                "options": options,
            })
        })
        .collect();
    let v = serde_json::json!({
        "genre": engine.options().genre.as_str(),
        "rules": rows,
    });
    let mut s = serde_json::to_string_pretty(&v).expect("serialize rules");
    s.push('\n');
    s
}

fn rules_markdown(engine: &Engine) -> String {
    let genre = engine.options().genre;
    let mut s = String::from("# noslop のルール\n\n");
    s.push_str(
        "このファイルは `noslop rules --format markdown --no-config` で生成しています。\n\n",
    );
    s.push_str(&format!("ジャンル `{genre}` での既定の状態です。\n\n"));
    s.push_str("| ID | 名前 | 日本語名 | レーン | 状態 | 既定の重大度 | 既定で有効 |\n");
    s.push_str("|---|---|---|---|---|---|---|\n");
    for e in engine.entries() {
        let m = e.rule.meta();
        s.push_str(&format!(
            "| [{id}](#{anchor}) | `{name}` | {title} | {lane} | {status} | {sev} | {enabled} |\n",
            id = m.id,
            anchor = m.id.to_ascii_lowercase(),
            name = m.name,
            title = m.title,
            lane = m.lane.as_str(),
            status = m.status.as_str(),
            sev = m.default_severity.as_str(),
            enabled = if e.enabled { "✓" } else { "" },
        ));
    }
    for e in engine.entries() {
        let m = e.rule.meta();
        s.push_str(&format!("\n## {}\n\n", m.id));
        s.push_str(&format!("**{}** — {}\n\n", m.name, m.title));
        s.push_str(&format!("- レーン: {}\n", lane_label(m.lane)));
        s.push_str(&format!("- 状態: {}\n", status_label(m.status)));
        s.push_str(&format!("- 既定の重大度: {}\n", m.default_severity));
        if let Some(basis) = calibration_basis(e) {
            s.push_str(&format!(
                "- 校正の基準: {}\n",
                calibration_basis_label(basis)
            ));
        }
        let options = e.rule.options();
        if !options.is_empty() {
            let list: Vec<String> = options
                .iter()
                .map(|(k, v)| format!("`{k} = {v}`"))
                .collect();
            s.push_str(&format!("- 設定項目: {}\n", list.join(", ")));
        }
        s.push_str(&format!("\n{}\n", m.summary));
        if !m.explanation.trim().is_empty() {
            s.push_str(&format!(
                "\n{}\n",
                demote_headings(m.explanation.trim_end())
            ));
        }
    }
    s
}

/// 説明文の見出しを `###` 以下にそろえる (ルールごとの節 `##` の下に入れるため)。
fn demote_headings(text: &str) -> String {
    text.lines()
        .map(|line| {
            let hashes = line.chars().take_while(|&c| c == '#').count();
            if (1..3).contains(&hashes) && line[hashes..].starts_with(' ') {
                format!("###{}", &line[hashes..])
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn explain(args: ExplainArgs) -> u8 {
    let engine = match listing_engine(&args.config, args.genre, false) {
        Ok(e) => e,
        Err(e) => return error(e),
    };
    let Some(entry) = engine.find(&args.rule) else {
        return error(format!(
            "未知のルールです: {} (`noslop rules` で一覧を確認できます)",
            args.rule
        ));
    };
    let text = explain_text(&engine, entry);
    let mut out = io::stdout().lock();
    match out.write_all(text.as_bytes()).and_then(|()| out.flush()) {
        Ok(()) => EXIT_OK,
        Err(e) if e.kind() == io::ErrorKind::BrokenPipe => EXIT_OK,
        Err(e) => error(format!("出力に失敗しました: {e}")),
    }
}

pub(crate) fn explain_text(engine: &Engine, entry: &RuleEntry) -> String {
    let m = entry.rule.meta();
    let genre = engine.options().genre;
    let mut s = format!("{} {} — {}\n\n", m.id, m.name, m.title);
    s.push_str(&format!("  レーン      : {}\n", lane_label(m.lane)));
    let status = if entry.builtin {
        status_label(m.status)
    } else {
        "設定ファイルで定義した独自ルール (既定で有効)"
    };
    s.push_str(&format!("  状態        : {status}\n"));
    s.push_str(&format!("  既定の重大度: {}\n", m.default_severity));
    if let Some(sev) = entry.severity {
        s.push_str(&format!("  設定の重大度: {sev}\n"));
    }
    if let Some(basis) = calibration_basis(entry) {
        s.push_str(&format!(
            "  校正の基準  : {}\n",
            calibration_basis_label(basis)
        ));
    }
    let allowed = entry.rule.allowed_in(genre);
    let state = if entry.enabled {
        "有効".to_string()
    } else if !allowed {
        "無効 (このジャンルの慣習と衝突するため既定で止めている)".to_string()
    } else {
        "無効".to_string()
    };
    s.push_str(&format!("  ジャンル    : {genre} では{state}\n"));
    let options = entry.rule.options();
    if !options.is_empty() {
        s.push_str("  設定項目    :\n");
        for (k, v) in options {
            s.push_str(&format!("    {k} = {v}\n"));
        }
    }
    s.push_str(&format!("\n{}\n", m.summary));
    if !m.explanation.trim().is_empty() {
        s.push_str(&format!("\n{}\n", m.explanation.trim_end()));
    }
    s
}

fn init(args: InitArgs) -> u8 {
    let path = Path::new(config::CONFIG_FILE_NAMES[0]);
    if path.exists() && !args.force {
        return error(format!(
            "{} はすでにあります。上書きするには --force を付けてください",
            path.display()
        ));
    }
    if let Err(e) = std::fs::write(path, config::TEMPLATE) {
        return error(format!("{} を書き込めません: {e}", path.display()));
    }
    let mut out = io::stdout();
    let _ = writeln!(out, "{} を作成しました", path.display());
    if out.is_terminal() {
        let _ = writeln!(out, "各項目の説明はファイル内のコメントを見てください");
    }
    EXIT_OK
}

fn skill_install(args: SkillInstallArgs) -> u8 {
    let target = crate::skill::Target::from(args.target);
    let skills_dir = match args.dir {
        Some(dir) => dir,
        None => match std::env::home_dir() {
            Some(home) => target.skills_dir(&home),
            None => {
                return error(
                    "ホームディレクトリが分かりません。--dir でスキルの置き場を指定してください",
                );
            }
        },
    };
    match crate::skill::install(&skills_dir) {
        Ok(path) => {
            println!(
                "noslop のスキルを {} 用に入れました: {}",
                target.name(),
                path.display()
            );
            EXIT_OK
        }
        Err(e) => error(format!(
            "スキルを書き込めません ({}): {e}",
            skills_dir.display()
        )),
    }
}

fn mcp(args: McpArgs) -> u8 {
    // Claude Code は起動したサーバーの環境変数 CLAUDE_PROJECT_DIR にプロジェクトのルートを渡す。
    // サーバーの作業ディレクトリは登録したスコープによって変わるので、あればそこから探す。
    let start = std::env::var_os("CLAUDE_PROJECT_DIR")
        .map(PathBuf::from)
        .filter(|p| p.is_dir());
    // 設定の誤りでサーバーを落とすと、クライアント側では理由が見えにくい。起動は続け、
    // ツールを呼ばれたときに誤りを返す (標準エラーにも出しておく)。
    let base = load_config_from(&args.config, start.as_deref())
        .map(|cfg| config_engine_options(cfg.as_ref()))
        .map_err(|e| e.to_string());
    if let Err(e) = &base {
        eprintln!("エラー: {e}");
    }
    let mut server = crate::mcp::Server::new(base);
    match crate::mcp::serve(&mut server, io::stdin().lock(), io::stdout().lock()) {
        Ok(()) => EXIT_OK,
        Err(e) if e.kind() == io::ErrorKind::BrokenPipe => EXIT_OK,
        Err(e) => error(format!("MCP の入出力に失敗しました: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_definition_is_consistent() {
        Cli::command().debug_assert();
    }

    #[test]
    fn parses_check_options_and_aliases() {
        let cli = Cli::try_parse_from([
            "noslop",
            "lint",
            "a.md",
            "-f",
            "github-actions",
            "--genre",
            "blog",
            "--ignore-rules",
            "P01,R03",
            "--ignore-rules",
            "P02",
            "--include",
            "lists,quotes",
            "--fail-on",
            "warning",
        ])
        .unwrap();
        let Command::Check(args) = cli.command else {
            panic!("check");
        };
        assert_eq!(args.format, Some(FormatArg::Github));
        assert_eq!(args.engine.genre, Some(Genre::Essay));
        assert_eq!(args.engine.ignore_rules, vec!["P01", "R03", "P02"]);
        assert_eq!(
            args.engine.include,
            vec![IncludeArg::Lists, IncludeArg::Quotes]
        );
        assert_eq!(args.fail_on, Some(FailOnArg::Warning));
        let options = engine_options(&args.engine, None);
        assert!(options.scope.lists && options.scope.blockquotes && !options.scope.tables);
        assert_eq!(options.selection.cli_disable.len(), 3);
    }

    #[test]
    fn defaults_to_current_directory_and_rejects_bad_genre() {
        let cli = Cli::try_parse_from(["noslop", "check"]).unwrap();
        let Command::Check(args) = cli.command else {
            panic!("check");
        };
        assert_eq!(args.paths, vec![PathBuf::from(".")]);
        assert!(Cli::try_parse_from(["noslop", "check", "--genre", "poem"]).is_err());
        assert!(Cli::try_parse_from(["noslop", "check", "--config", "a", "--no-config"]).is_err());
    }

    #[test]
    fn cli_values_override_config() {
        let cfg_text = "genre = \"tech\"\nexperimental = true\nline_breaks = \"sentence\"\n[scope]\ntables = true\n[rules]\ndisable = [\"X\"]\n";
        let file = config::ConfigFile::parse(cfg_text, Path::new("noslop.toml")).unwrap();
        let loaded = LoadedConfig {
            path: PathBuf::from("noslop.toml"),
            base_dir: PathBuf::from("."),
            file,
        };
        let cli = Cli::try_parse_from([
            "noslop",
            "check",
            "--genre",
            "business",
            "--line-breaks",
            "space",
        ])
        .unwrap();
        let Command::Check(args) = cli.command else {
            panic!("check");
        };
        let o = engine_options(&args.engine, Some(&loaded));
        assert_eq!(o.genre, Genre::Business);
        assert!(
            o.experimental,
            "設定の experimental は CLI で指定しなくても効く"
        );
        assert_eq!(o.parse.line_breaks, LineBreakMode::Space);
        assert!(o.scope.tables);
        assert_eq!(o.selection.config_disable, vec!["X"]);
    }

    #[test]
    fn report_and_format_choose_the_output() {
        use CheckOutput as O;
        use FormatArg as F;
        use ReportArg as R;
        for (report, format, expected) in [
            (None, None, O::Text),
            (None, Some(F::Json), O::Json),
            (None, Some(F::Toon), O::Toon),
            (None, Some(F::Github), O::Github),
            (None, Some(F::Brief), O::BriefMarkdown),
            (None, Some(F::Markdown), O::BriefMarkdown),
            (Some(R::Full), Some(F::Toon), O::Toon),
            (Some(R::Brief), None, O::BriefMarkdown),
            (Some(R::Brief), Some(F::Json), O::BriefJson),
            (Some(R::Brief), Some(F::Toon), O::BriefToon),
            (Some(R::Brief), Some(F::Brief), O::BriefMarkdown),
        ] {
            assert_eq!(
                check_output(report, format),
                Ok(expected),
                "{report:?} {format:?}"
            );
        }
        for (report, format) in [
            (R::Full, F::Markdown),
            (R::Full, F::Brief),
            (R::Brief, F::Text),
            (R::Brief, F::Github),
        ] {
            assert!(
                check_output(Some(report), Some(format)).is_err(),
                "{report:?} {format:?}"
            );
        }

        let cli =
            Cli::try_parse_from(["noslop", "check", "--report", "brief", "-f", "toon"]).unwrap();
        let Command::Check(args) = cli.command else {
            panic!("check");
        };
        assert_eq!((args.report, args.format), (Some(R::Brief), Some(F::Toon)));
    }

    #[test]
    fn parses_diff_and_calibrate() {
        let cli = Cli::try_parse_from([
            "noslop",
            "diff",
            "-",
            "after.md",
            "--format",
            "json",
            "--genre",
            "tech",
            "--only-rules",
            "P01",
        ])
        .unwrap();
        let Command::Diff(args) = cli.command else {
            panic!("diff");
        };
        assert!(is_stdin(&args.before));
        assert_eq!(args.after, PathBuf::from("after.md"));
        assert_eq!(args.format, DiffFormatArg::Json);
        assert_eq!(args.engine.genre, Some(Genre::Tech));
        assert!(Cli::try_parse_from(["noslop", "diff", "before.md"]).is_err());

        let cli = Cli::try_parse_from([
            "noslop",
            "calibrate",
            "--human",
            "h",
            "--ai",
            "a1",
            "--ai",
            "a2",
            "--target-fp",
            "0.1",
            "-f",
            "markdown",
        ])
        .unwrap();
        let Command::Calibrate(args) = cli.command else {
            panic!("calibrate");
        };
        assert_eq!(args.human, vec![PathBuf::from("h")]);
        assert_eq!(args.ai, vec![PathBuf::from("a1"), PathBuf::from("a2")]);
        assert_eq!(args.target_fp, 0.1);
        assert_eq!(args.holdout, crate::calibrate::DEFAULT_HOLDOUT);
        assert_eq!(args.format, CalibrateFormatArg::Markdown);
        assert!(Cli::try_parse_from(["noslop", "calibrate", "--human", "h"]).is_err());
    }

    #[test]
    fn calibrated_rules_show_their_calibration_basis() {
        let engine = Engine::new(EngineOptions::default()).unwrap();
        let v: serde_json::Value = serde_json::from_str(&rules_json(&engine)).unwrap();
        let basis = |id: &str| {
            v["rules"]
                .as_array()
                .unwrap()
                .iter()
                .find(|r| r["id"] == id)
                .unwrap_or_else(|| panic!("{id}"))["calibrationBasis"]
                .clone()
        };
        assert_eq!(basis("P01"), "warning");
        assert_eq!(basis("P03"), "info", "既定の重大度が情報のルールは情報");
        assert_eq!(basis("R05"), "error", "重大の率で校正した");
        assert!(basis("S01").is_null(), "実験的なルールは校正していない");

        let r05 = engine.find("R05").unwrap();
        assert!(explain_text(&engine, r05).contains("校正の基準  : 重大の指摘で数えた誤検知率"));
        let s01 = engine.find("S01").unwrap();
        assert!(!explain_text(&engine, s01).contains("校正の基準"));
    }

    #[test]
    fn lanes_are_called_by_the_same_names_as_in_the_findings() {
        assert_eq!(lane_label(Lane::Slop), "AI 臭さ (自然度スコアに入る)");
        assert_eq!(
            lane_label(Lane::Readability),
            "読みやすさ (自然度スコアに入らない)"
        );
        assert_eq!(
            lane_label(Lane::Custom),
            "独自ルール (自然度スコアに入らない)"
        );
    }

    #[test]
    fn markdown_headings_are_demoted_under_rule_sections() {
        assert_eq!(
            demote_headings("# 見出し\n## 小見出し\n### そのまま\n本文"),
            "### 見出し\n### 小見出し\n### そのまま\n本文"
        );
    }
}
