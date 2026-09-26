//! コマンドライン。
//!
//! 終了コード:
//!
//! - 0: 完了 (指摘の有無は問わない)
//! - 1: `--fail-on` に指定した重大度以上の、抑制していない指摘がある
//! - 2: 引数・設定・入出力・通信のエラー (読めないファイルがあっても他のファイルは処理して出力する)

use std::io::{self, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anstream::{AutoStream, ColorChoice};
use clap::builder::{PossibleValue, PossibleValuesParser};
use clap::{Args, Parser, Subcommand, ValueEnum};
use unicode_width::UnicodeWidthStr;

use crate::code::CodeLanguage;
use crate::config::{self, ConfigError, ConfigLayers, CustomRuleConfig, FailOn, LoadedConfig};
use crate::diagnostic::{RuleStatus, Severity};
use crate::dictionaries::{self, Check, Distributed, Outcome};
use crate::document::{ParseOptions, SourceFormat};
use crate::engine::{Engine, EngineOptions, Input, RuleEntry, Selection};
use crate::genre::Genre;
use crate::morph::{
    self, AutoPick, DictionaryChoice, DictionarySearch, MorphologyMode, MorphologyOptions,
};
use crate::output::{CheckOutput, RenderOptions, RuleCatalog};
use crate::rules::Scope;
use crate::segment::LineBreakMode;
use crate::walk::{self, Exclude, ExcludeRule, WalkOptions};

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
    /// 設定ファイルのひな形を作る (カレントディレクトリの noslop.toml。--user ならユーザーの設定 ~/.config/noslop/config.toml)
    Init(InitArgs),
    /// MCP サーバーとして標準入出力で待ち受ける (AI エージェントから検査を呼ぶ)
    ///
    /// ツールは check (本文の検査)・diff (改稿の前後の比較)・explain (ルールの説明)・rules (ルールの一覧)。
    /// プロジェクトの設定ファイルは、環境変数 CLAUDE_PROJECT_DIR があればそこから、なければカレントから親へ探し、ユーザーの設定 (~/.config/noslop/config.toml) に重ねる。
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
    /// hasami の配布辞書 (形態素解析の辞書) を取得する・一覧する
    ///
    /// 置き場は hasami の share ディレクトリ (既定は ~/.local/share/hasami。HASAMI_DATA_DIR・XDG_DATA_HOME で変わる)。
    /// 辞書を指定しないとき (dictionary = "auto") は、ここにある辞書を hasami の推奨順で選び、同梱の辞書より先に使う。
    /// 決まった辞書を使うときは --dict share:<名前> か、[morphology] に dictionary = "share:<名前>" を書く。
    #[command(subcommand)]
    Dict(DictCommand),
}

/// 設定ファイルの指定。
#[derive(Debug, Clone, Args)]
pub struct ConfigArgs {
    /// プロジェクトの設定ファイルを指定する (省略時はカレントから親へ noslop.toml / .noslop.toml を探す)。
    /// ユーザーの設定 (~/.config/noslop/config.toml) はこれに重ねる
    #[arg(long, value_name = "PATH")]
    pub config: Option<PathBuf>,
    /// 設定ファイルを読まない (ユーザーの設定も読まない)
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
    /// 形態素解析の辞書: auto (share ディレクトリの辞書を推奨順に、なければ同梱の辞書)・bundled (同梱の辞書)・share:<名前> (noslop dict download で取得した辞書)・ファイルのパス (hasami の .hsd)。
    /// 指定すると辞書を必ず使い、品詞で判定する
    #[arg(long, value_name = "DICT", conflicts_with = "no_dict")]
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
    /// ユーザーの設定 (~/.config/noslop/config.toml) のひな形を作る (ディレクトリがなければ作る)
    #[arg(long)]
    pub user: bool,
    /// すでにある設定ファイルを上書きする
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
    /// Claude Code のフック (編集したファイル、gws で書き込む値、Stop のときのリポジトリの差分を検査して、指摘を Claude に渡す)
    ///
    /// 標準入力でフックの入力 (JSON) を受け取り、イベントごとに検査する。
    /// PostToolUse (Write / Edit / MultiEdit) は書き換えたファイルの変わった行を、PreToolUse (Bash) は gws で Google ドキュメント・スプレッドシートに書き込む値を、Stop はリポジトリの差分 (HEAD との差分と追跡していないファイル) の変わった行を見る。
    /// 対象外のイベント・ツール・ファイルや指摘がないときは何も書かない。設定ファイルは入力の cwd から親へ探す。
    ClaudeCode(HookArgs),
    /// claw-hooks のコマンドフック (`[[command_hooks]]`) の判定器。gws で Google ドキュメント・スプレッドシートに書き込む値を、書き込む前に検査する
    ///
    /// 標準入力で、claw-hooks が解析した gws の呼び出し 1 つの引数 (JSON。プロトコルの版 1) を受け取る。検査は claude-code の PreToolUse と同じ。
    /// ドキュメントの本文に警告以上の指摘があれば、理由を標準エラーに書いて終了コード 2 で終わる (claw-hooks がコマンドを止める)。止めた書き込みと同じものは、止めてから 30 分のあいだ検査せずに通す。
    /// 止めない指摘 (セルなどの短い値・情報だけ・--dry-run・呼び出しを確定できないもの) は標準出力に書いて 0 で終わる (claw-hooks がエージェントに渡す)。対象外や指摘がないときは何も書かずに 0。
    /// 入力や設定の誤りは、標準エラーに書いて 1 で終わる (claw-hooks の on_error に従う)。
    /// 設定ファイルは入力の cwd から親へ探す。
    Command(CommandHookArgs),
    /// 編集したファイルのパスだけを渡すフックから呼ぶ (claw-hooks の extension_hooks のように、フックの入力を渡せない仕組み向け)
    ///
    /// 指摘があれば、claude-code と同じ短い改稿指示をテキストで標準出力に書く。
    /// 対象外のファイルや指摘がないときは何も書かない。
    /// 変わった行は git の差分 (HEAD との比較) から求め、git で追跡していないファイルや git の外ではファイル全体を見る。
    /// 設定ファイルはカレントディレクトリから親へ探す。
    File(FileHookArgs),
    /// リポジトリのコミットしていない変更を検査する (claw-hooks の stop_hooks のように、フックの入力を渡せない仕組みの Stop 向け)
    ///
    /// カレントディレクトリを含む git の作業ツリーで、HEAD との差分 (index と作業ツリーの両方) の変わった行と、追跡していないファイル (.gitignore などで無視するものを除く) の全体に重なる指摘を、claude-code と同じ短い改稿指示でテキストに書く。
    /// 消したファイルは見ず、名前を変えたファイルは変える前との差分の行を見る。
    /// 指摘があれば終了コード 1、なければ何も書かずに 0 で終わる (git の外でも 0)。設定の誤りや git の失敗は、標準エラーに書いて 2 で終わる。
    /// 設定ファイルはカレントディレクトリから親へ探す。
    GitDiff(GitDiffHookArgs),
}

/// `hook command` の引数。
#[derive(Debug, Clone, Args)]
pub struct CommandHookArgs {
    /// 出力の文字数の上限。超えるときは行の単位で後ろを省く (呼び出し側の上限に合わせる)
    #[arg(long, default_value_t = 9_000, value_parser = parse_limit, value_name = "N")]
    pub max_chars: usize,
    #[command(flatten)]
    pub hook: HookArgs,
}

/// `hook git-diff` の引数。
#[derive(Debug, Clone, Args)]
pub struct GitDiffHookArgs {
    /// 出力の文字数の上限。超えるときは行の単位で後ろを省く (呼び出し側の上限に合わせる)
    #[arg(long, default_value_t = 9_000, value_parser = parse_limit, value_name = "N")]
    pub max_chars: usize,
    #[command(flatten)]
    pub hook: HookArgs,
}

/// `hook file` の引数。
#[derive(Debug, Clone, Args)]
pub struct FileHookArgs {
    /// 編集したファイル
    #[arg(value_name = "PATH")]
    pub path: PathBuf,
    /// 出力の文字数の上限。超えるときは行の単位で後ろを省く (呼び出し側の上限に合わせる)
    #[arg(long, default_value_t = 9_000, value_parser = parse_limit, value_name = "N")]
    pub max_chars: usize,
    #[command(flatten)]
    pub hook: HookArgs,
}

/// フックに共通の引数。
#[derive(Debug, Clone, Args)]
pub struct HookArgs {
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

#[derive(Debug, Subcommand)]
pub enum DictCommand {
    /// hasami の配布辞書を取得する (大きさ・SHA-256・辞書の形式を確かめてから置く)
    ///
    /// 取得元は、noslop に組み込んだ hasami のリリースの目録 (dict list の 1 行目の版) の添付ファイル。
    /// 取得した中身は、目録の大きさと SHA-256 で確かめ、辞書として読めることも確かめてから置く。
    /// 途中で失敗しても、すでにあるファイルは消さず、壊さない。
    Download(DictDownloadArgs),
    /// 配布辞書と、取得済みかを表示する (通信しない)
    List(DictListArgs),
}

#[derive(Debug, Args)]
pub struct DictDownloadArgs {
    /// 取得する辞書
    #[arg(
        value_name = "NAME",
        default_value = dictionaries::RECOMMENDED,
        value_parser = dictionary_names()
    )]
    pub name: String,
    /// 保存先 (既定は hasami の share ディレクトリ)
    #[arg(long, value_name = "DIR")]
    pub dir: Option<PathBuf>,
    /// 取得元の URL の接頭辞 (ミラーを使うとき。この後に /<名前>.hsd.zst (--uncompressed なら /<名前>.hsd) を付けて取得する。
    /// どの取得元でも大きさと SHA-256 を確かめる)
    #[arg(long, value_name = "URL", default_value = dictionaries::DEFAULT_SOURCE)]
    pub source: String,
    /// 圧縮版 (.hsd.zst) を使わず、展開前の辞書 (.hsd) を取得する (圧縮版を置いていない取得元向け)
    #[arg(long)]
    pub uncompressed: bool,
    /// 正しいファイルがあっても取り直す。中身の違うファイルも置き換える
    #[arg(long)]
    pub force: bool,
}

#[derive(Debug, Args)]
pub struct DictListArgs {
    /// 取得済みかを確かめる場所 (既定は hasami の share ディレクトリ)
    #[arg(long, value_name = "DIR")]
    pub dir: Option<PathBuf>,
}

/// `dict download` の NAME に書ける名前 (配布辞書の表から作る)。
fn dictionary_names() -> PossibleValuesParser {
    PossibleValuesParser::new(
        dictionaries::DICTIONARIES.map(|d| PossibleValue::new(d.name).help(d.description())),
    )
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
    let args: Vec<std::ffi::OsString> = std::env::args_os().collect();
    let cli = match Cli::try_parse_from(&args) {
        Ok(cli) => cli,
        Err(e) => {
            let _ = e.print();
            return ExitCode::from(parse_error_code(&args, &e));
        }
    };
    let code = match cli.command {
        Command::Check(args) => check(args),
        Command::Diff(args) => diff(args),
        Command::Calibrate(args) => calibrate(args),
        Command::Rules(args) => rules(args),
        Command::Explain(args) => explain(args),
        Command::Init(args) => init(args),
        Command::Mcp(args) => mcp(args),
        Command::Hook(HookCommand::ClaudeCode(args)) => crate::hook::claude_code(&args),
        Command::Hook(HookCommand::Command(args)) => crate::hook::command(&args),
        Command::Hook(HookCommand::File(args)) => crate::hook::file(&args),
        Command::Hook(HookCommand::GitDiff(args)) => crate::hook::git_diff(&args),
        Command::SkillInstall(args) => skill_install(args),
        Command::Dict(DictCommand::Download(args)) => dict_download(args),
        Command::Dict(DictCommand::List(args)) => dict_list(args),
    };
    ExitCode::from(code)
}

/// 引数を解釈できなかったときの終了コード (clap の既定。引数の誤りは 2、`--help` と `--version` は 0)。
///
/// フックの引数の誤りは 1 にする。Claude Code はフックの終了コード 2 を「ツールの呼び出しを止める」
/// (Stop なら「会話を続ける」)、claw-hooks はコマンドフックの判定器の 2 を「コマンドを止める」と読むので、
/// 2 のままでは、設定の書き誤りで作業を止めてしまう (1 なら止めない誤りとして扱われる)。誤りを 2 で
/// 知らせる約束の `hook git-diff` は、そのままにする。
fn parse_error_code(args: &[std::ffi::OsString], error: &clap::Error) -> u8 {
    let hook =
        args.get(1).is_some_and(|a| a == "hook") && !args.get(2).is_some_and(|a| a == "git-diff");
    if hook && error.use_stderr() {
        1
    } else {
        u8::try_from(error.exit_code()).unwrap_or(EXIT_ERROR)
    }
}

fn error(message: impl std::fmt::Display) -> u8 {
    eprintln!("エラー: {message}");
    EXIT_ERROR
}

/// 手元の環境: ユーザーの設定の置き場所 (ホームディレクトリ) と、辞書を探す場所。実行時は
/// [`Environment::from_process`] で作り、入口から渡す。テストでは空 (`Default`) にして、手元の設定と
/// 辞書に左右されないようにする (環境変数を書き換えずに済むように、引数で渡す)。
#[derive(Debug, Clone, Default)]
pub(crate) struct Environment {
    /// ホームディレクトリ (ユーザーの設定 `~/.config/noslop/config.toml` と、設定に書いたパスの `~/`)。
    pub home: Option<PathBuf>,
    /// 辞書を探す場所 (`HASAMI_DICT` と share ディレクトリ)。
    pub dictionaries: DictionarySearch,
    /// noslop のキャッシュの置き場所 ([`cache_dir`])。フックの状態 (gws の書き込みを止めた記録) を
    /// 置く。`None` なら状態を使う機能は動かない (gws のフックは書き込みを止めず、知らせるだけにする)。
    pub cache_dir: Option<PathBuf>,
}

impl Environment {
    /// 実行中のプロセスの環境から作る。
    pub(crate) fn from_process() -> Self {
        let home = std::env::home_dir().filter(|h| h.is_absolute());
        Self {
            cache_dir: cache_dir(
                std::env::consts::OS,
                std::env::var_os("XDG_CACHE_HOME").as_deref(),
                std::env::var_os("LOCALAPPDATA").as_deref(),
                home.as_deref(),
            ),
            home,
            dictionaries: DictionarySearch::from_env(),
        }
    }
}

/// noslop のキャッシュの置き場所。`XDG_CACHE_HOME` (絶対パスのときだけ) があれば
/// `$XDG_CACHE_HOME/noslop`、なければ macOS は `~/Library/Caches/noslop`、Windows は
/// `%LOCALAPPDATA%\noslop\cache`、ほかは `~/.cache/noslop`。`os` は `std::env::consts::OS` の値。
fn cache_dir(
    os: &str,
    xdg_cache_home: Option<&std::ffi::OsStr>,
    local_app_data: Option<&std::ffi::OsStr>,
    home: Option<&Path>,
) -> Option<PathBuf> {
    fn absolute(v: Option<&std::ffi::OsStr>) -> Option<&Path> {
        v.map(Path::new).filter(|p| p.is_absolute())
    }
    if let Some(dir) = absolute(xdg_cache_home) {
        return Some(dir.join("noslop"));
    }
    match os {
        "windows" => absolute(local_app_data).map(|dir| dir.join("noslop").join("cache")),
        "macos" => home.map(|h| h.join("Library").join("Caches").join("noslop")),
        _ => home.map(|h| h.join(".cache").join("noslop")),
    }
}

#[cfg(test)]
mod cache_dir_tests {
    use super::*;

    #[test]
    fn the_cache_dir_follows_xdg_then_the_platform() {
        let base = std::env::temp_dir();
        let other = base.join("other");
        let home = base.join("home");
        let at = |os: &str, xdg: Option<&Path>, local: Option<&Path>| {
            cache_dir(
                os,
                xdg.map(Path::as_os_str),
                local.map(Path::as_os_str),
                Some(&home),
            )
        };
        // XDG_CACHE_HOME はどの OS でも先に見る
        for os in ["linux", "macos", "windows"] {
            assert_eq!(at(os, Some(&base), Some(&other)), Some(base.join("noslop")));
        }
        assert_eq!(
            at("linux", None, None),
            Some(home.join(".cache").join("noslop"))
        );
        assert_eq!(
            at("macos", None, None),
            Some(home.join("Library").join("Caches").join("noslop"))
        );
        assert_eq!(
            at("windows", None, Some(&other)),
            Some(other.join("noslop").join("cache"))
        );
        assert_eq!(at("windows", None, None), None);
        // 相対パスの XDG_CACHE_HOME は使わない
        assert_eq!(
            at("linux", Some(Path::new("relative")), None),
            Some(home.join(".cache").join("noslop"))
        );
        assert_eq!(cache_dir("linux", None, None, None), None);
        assert_eq!(Environment::default().cache_dir, None);
    }
}

/// 設定ファイルを読み込む (`--config` がなければカレントから親へ探す)。
pub(crate) fn load_config(
    args: &ConfigArgs,
    env: &Environment,
) -> Result<ConfigLayers, ConfigError> {
    load_config_from(args, None, env)
}

/// 設定ファイルを読み込む。プロジェクトの設定は、`--config` がなければ `start` (省略時はカレント)
/// から親へ探す。ユーザーの設定はホームディレクトリの `.config/noslop/config.toml`。
pub(crate) fn load_config_from(
    args: &ConfigArgs,
    start: Option<&Path>,
    env: &Environment,
) -> Result<ConfigLayers, ConfigError> {
    if args.no_config {
        return Ok(ConfigLayers::default());
    }
    // ユーザーの設定は、置いてあれば読む (読めなければ設定の誤り)
    let user = env
        .home
        .as_deref()
        .map(config::user_config_path)
        .filter(|path| path.exists())
        .map(|path| config::load(&path))
        .transpose()?;
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
    let project = path.map(|p| config::load(&p)).transpose()?;
    Ok(ConfigLayers { user, project })
}

/// 設定ファイルだけからエンジンの設定を組み立てる (CLI の指定は反映しない)。ユーザーの設定の上に、
/// プロジェクトの設定を項目ごとに重ねる。辞書を探す場所は `env` から埋める。
pub(crate) fn config_engine_options(cfg: &ConfigLayers, env: &Environment) -> EngineOptions {
    let user = cfg.user.as_ref().map(|c| &c.file);
    let project = cfg.project.as_ref().map(|c| &c.file);
    let rules =
        |file: Option<&config::ConfigFile>| file.map(|f| f.rules.clone()).unwrap_or_default();
    let (user_rules, project_rules) = (rules(user), rules(project));
    EngineOptions {
        genre: cfg.pick(|f| f.genre).unwrap_or_default(),
        experimental: cfg.pick(|f| f.experimental).unwrap_or(false),
        scope: Scope {
            lists: cfg.pick(|f| f.scope.lists).unwrap_or(false),
            tables: cfg.pick(|f| f.scope.tables).unwrap_or(false),
            blockquotes: cfg.pick(|f| f.scope.blockquotes).unwrap_or(false),
        },
        parse: ParseOptions {
            line_breaks: cfg.pick(|f| f.line_breaks).unwrap_or_default(),
        },
        selection: Selection {
            user_enable: user_rules.enable,
            user_disable: user_rules.disable,
            config_enable: project_rules.enable,
            config_disable: project_rules.disable,
            ..Default::default()
        },
        user_rule_tables: user_rules.tables,
        rule_tables: project_rules.tables,
        custom: merge_custom(
            user.map(|f| f.custom.as_slice()).unwrap_or_default(),
            project.map(|f| f.custom.as_slice()).unwrap_or_default(),
        ),
        morphology: MorphologyOptions {
            mode: cfg.pick(|f| f.morphology.mode).unwrap_or_default(),
            dictionary: cfg
                .pick_with_source(|f| f.morphology.dictionary.clone())
                .map(|(spec, loaded)| config_dictionary(loaded, &spec, env.home.as_deref()))
                .unwrap_or_default(),
            search: env.dictionaries.clone(),
        },
    }
}

/// 独自ルールを重ねる。両方の設定のものを使い、同じ ID (大文字小文字は区別しない) はプロジェクトの
/// 定義で丸ごと置き換える。
fn merge_custom(user: &[CustomRuleConfig], project: &[CustomRuleConfig]) -> Vec<CustomRuleConfig> {
    let replaced = |c: &CustomRuleConfig| project.iter().any(|p| p.id.eq_ignore_ascii_case(&c.id));
    user.iter()
        .filter(|c| !replaced(c))
        .chain(project)
        .cloned()
        .collect()
}

/// 設定ファイルに書いた辞書の指定を読む。ファイルのパスは、`~/` をホームディレクトリに、相対パスを
/// その設定ファイルのディレクトリ基準に直す (層を重ねた後では、どのファイルに書いたか分からないため)。
fn config_dictionary(cfg: &LoadedConfig, spec: &Path, home: Option<&Path>) -> DictionaryChoice {
    match DictionaryChoice::parse(spec) {
        DictionaryChoice::File(path) => {
            DictionaryChoice::File(config_relative_path(cfg, path, home))
        }
        choice => choice,
    }
}

/// 設定ファイルに書いたパスを解決する (`~/` はホームディレクトリ、相対パスは設定ファイルの
/// ディレクトリが基準)。
fn config_relative_path(cfg: &LoadedConfig, path: PathBuf, home: Option<&Path>) -> PathBuf {
    if let (Ok(rest), Some(home)) = (path.strip_prefix("~"), home) {
        return home.join(rest);
    }
    match cfg.path.parent() {
        Some(dir) if path.is_relative() => dir.join(path),
        _ => path,
    }
}

/// 設定ファイルと CLI からエンジンの設定を組み立てる (CLI の指定が優先)。
fn engine_options(args: &EngineArgs, cfg: &ConfigLayers, env: &Environment) -> EngineOptions {
    let mut options = config_engine_options(cfg, env);
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
    } else if let Some(spec) = &args.dict {
        options.morphology.mode = MorphologyMode::Required;
        options.morphology.dictionary = DictionaryChoice::parse(spec);
    }
    let selection = &mut options.selection;
    selection.cli_enable = args.enable_rules.clone();
    selection.cli_disable = args.ignore_rules.clone();
    selection.only = (!args.only_rules.is_empty()).then(|| args.only_rules.clone());
    selection.no_readability = args.no_readability;
    options
}

/// 設定ファイルから、検査するファイルの集め方を組み立てる。
///
/// `[files] exclude` は丸ごと置き換える: プロジェクトの設定に書いてあれば (空の配列でも) それだけを、
/// 設定ファイルのディレクトリ基準で使う。なければユーザーの設定の `exclude` を、検査の起点 (渡した
/// ディレクトリ) 基準で使う。
pub(crate) fn walk_options(cfg: &ConfigLayers) -> Result<WalkOptions, ConfigError> {
    fn normalize(extensions: &[String]) -> Vec<String> {
        extensions
            .iter()
            .map(|e| e.trim().trim_start_matches('.').to_ascii_lowercase())
            .filter(|e| !e.is_empty())
            .collect()
    }
    let mut options = WalkOptions::default();
    if let Some(ext) = cfg.pick(|f| f.files.extensions.clone()) {
        options.extensions = normalize(&ext);
    }
    if let Some(ext) = cfg.pick(|f| f.code.extensions.clone()) {
        options.code_extensions = normalize(&ext);
        for ext in &options.code_extensions {
            if CodeLanguage::from_extension(ext).is_none() {
                let known: Vec<&str> = CodeLanguage::known_extensions().collect();
                return Err(ConfigError::Invalid(format!(
                    "[code] extensions の `{ext}` はコードの拡張子として読めません (読めるのは {})",
                    known.join("・")
                )));
            }
        }
    }
    fn exclude_of(c: Option<&LoadedConfig>) -> Option<(&[String], &LoadedConfig)> {
        c.and_then(|c| c.file.files.exclude.as_deref().map(|p| (p, c)))
    }
    options.exclude = match (
        exclude_of(cfg.project.as_ref()),
        exclude_of(cfg.user.as_ref()),
    ) {
        (Some((patterns, _)), _) | (None, Some((patterns, _))) if patterns.is_empty() => None,
        (Some((patterns, project)), _) => Some(ExcludeRule::Fixed(Arc::new(Exclude::new(
            &project.base_dir,
            patterns,
        )?))),
        (None, Some((patterns, user))) => {
            // 書式の誤りは、検査を始める前に知らせる
            Exclude::new(&user.base_dir, patterns)?;
            Some(ExcludeRule::PerRoot(patterns.into()))
        }
        (None, None) => None,
    };
    Ok(options)
}

fn check(args: CheckArgs) -> u8 {
    let output = match check_output(args.report, args.format) {
        Ok(output) => output,
        Err(e) => return error(e),
    };
    let env = Environment::from_process();
    let cfg = match load_config(&args.config, &env) {
        Ok(cfg) => cfg,
        Err(e) => return error(e),
    };
    let options = engine_options(&args.engine, &cfg, &env);
    let fail_on = args
        .fail_on
        .map(FailOn::from)
        .or(cfg.pick(|f| f.fail_on))
        .unwrap_or_default();
    let walk_opts = match walk_options(&cfg) {
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
    let result = if output == CheckOutput::Text {
        for e in &report.errors {
            eprintln!("エラー: {}: {}", e.path, e.message);
        }
        let mut rendered = Vec::with_capacity(OUTPUT_BUFFER_BYTES);
        output
            .render(&report, &render_opts, &mut rendered)
            .and_then(|()| write_styled(&rendered, args.color))
    } else {
        write_buffered(|out| output.render(&report, &render_opts, out))
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
    let env = Environment::from_process();
    let cfg = match load_config(&args.config, &env) {
        Ok(cfg) => cfg,
        Err(e) => return error(e),
    };
    let engine = match Engine::new(engine_options(&args.engine, &cfg, &env)) {
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
    let env = Environment::from_process();
    let cfg = match load_config(&args.config, &env) {
        Ok(cfg) => cfg,
        Err(e) => return error(e),
    };
    let walk = match walk_options(&cfg) {
        Ok(w) => w,
        Err(e) => return error(e),
    };
    let mut engine = config_engine_options(&cfg, &env);
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
    let env = Environment::from_process();
    let cfg = load_config(config_args, &env)?;
    let mut options = config_engine_options(&cfg, &env);
    if let Some(genre) = genre {
        options.genre = genre;
    }
    options.experimental |= experimental;
    Engine::new(options)
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
        s.push_str(&format!("- レーン: {}\n", m.lane.label_ja()));
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
    s.push_str(&format!("  レーン      : {}\n", m.lane.label_ja()));
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
    let (path, template) = if args.user {
        let Some(home) = Environment::from_process().home else {
            return error(
                "ホームディレクトリが分かりません。ユーザーの設定の置き場所 (~/.config/noslop/config.toml) を決められません",
            );
        };
        (config::user_config_path(&home), config::user_template())
    } else {
        (
            PathBuf::from(config::CONFIG_FILE_NAMES[0]),
            config::template(),
        )
    };
    let shown = display_path(&path);
    if path.exists() && !args.force {
        return error(format!(
            "{shown} はすでにあります。上書きするには --force を付けてください"
        ));
    }
    // ユーザーの設定は ~/.config/noslop を作ってから書く
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty())
        && let Err(e) = std::fs::create_dir_all(dir)
    {
        return error(format!("{} を作れません: {e}", display_path(dir)));
    }
    if let Err(e) = std::fs::write(&path, template) {
        return error(format!("{shown} を書き込めません: {e}"));
    }
    let mut out = io::stdout();
    let _ = writeln!(out, "{shown} を作成しました");
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

/// 進み具合の行を書き換える間隔。
const PROGRESS_INTERVAL: Duration = Duration::from_millis(200);

fn dict_download(args: DictDownloadArgs) -> u8 {
    let Some(dict) = dictionaries::find(&args.name) else {
        return error(format!("配布辞書ではありません: {}", args.name));
    };
    let share = dictionaries::share_dir();
    let Some(dir) = args.dir.or_else(|| share.clone()) else {
        return error(
            "share ディレクトリが分かりません (HASAMI_DATA_DIR・XDG_DATA_HOME・HOME のどれも設定されていません。Windows では LOCALAPPDATA も見ます)。--dir で保存先を指定してください",
        );
    };
    let compressed = !args.uncompressed;
    let mut progress = DownloadProgress::new(dict, &dir, compressed, io::stderr().is_terminal());
    let result = dictionaries::download(
        dict,
        &dir,
        &args.source,
        args.force,
        compressed,
        &mut |r, t| progress.update(r, t),
    );
    progress.finish();
    match result {
        Ok(outcome) => {
            let pick = morph::auto_pick(&DictionarySearch::from_env());
            let _ = writeln!(
                io::stdout(),
                "{}",
                download_report(dict, &outcome, &dir, share.as_deref(), &pick)
            );
            EXIT_OK
        }
        Err(e) => error(e),
    }
}

/// `dict download` がうまくいったときに標準出力へ出す、結果と使い方の行。`pick` は辞書を指定しない
/// とき (`auto`) に使う辞書。
fn download_report(
    dict: &Distributed,
    outcome: &Outcome,
    dir: &Path,
    share: Option<&Path>,
    pick: &AutoPick,
) -> String {
    let message = match outcome {
        Outcome::Present(path) => format!(
            "{} は取得済みです (大きさと SHA-256 を確かめました): {}",
            dict.name,
            display_path(path)
        ),
        Outcome::Downloaded(path) => format!(
            "{} を取得しました (大きさ・SHA-256・辞書の形式を確かめました): {}",
            dict.name,
            display_path(path)
        ),
    };
    if !is_share_dir(dir, share) {
        return format!(
            "{message}\n使うときは --dict にこのファイルのパスを指定してください (share:<名前> は share ディレクトリの辞書を指します)"
        );
    }
    let auto = match pick {
        AutoPick::Share(path) if path.file_stem().and_then(|s| s.to_str()) == Some(dict.name) => {
            "辞書を指定しないとき (dictionary = \"auto\") は、この辞書を使います".to_string()
        }
        other => format!(
            "辞書を指定しないとき (dictionary = \"auto\") は、{}を使います",
            auto_pick_label(other)
        ),
    };
    let usage = share_usage(&format!("{}{}", dictionaries::SHARE_PREFIX, dict.name));
    format!("{message}\n{auto}\n{usage}")
}

/// `share:<名前>` の辞書に決めて使うときの案内の行。
fn share_usage(spec: &str) -> String {
    format!(
        "決まった辞書を使うときは --dict {spec} か、設定ファイルの [morphology] に dictionary = \"{spec}\" を書いてください"
    )
}

/// `auto` が選ぶ辞書の呼び名 (「〜を使います」の前に置く)。
fn auto_pick_label(pick: &AutoPick) -> String {
    match pick {
        AutoPick::HasamiDict(path) => format!("HASAMI_DICT の辞書 ({}) ", display_path(path)),
        AutoPick::Share(path) => format!(
            "{} ({}) ",
            path.file_stem().and_then(|s| s.to_str()).unwrap_or(""),
            display_path(path)
        ),
        AutoPick::Bundled => "同梱の ipadic ".to_string(),
        AutoPick::Nothing => "辞書なしの近似".to_string(),
    }
}

/// 「今は〜を使います」の行 (呼び名が英数字で始まるときは、「今は」との間に空白を入れる)。
fn auto_pick_now(pick: &AutoPick) -> String {
    let label = auto_pick_label(pick);
    let space = if label.starts_with(|c: char| c.is_ascii()) {
        " "
    } else {
        ""
    };
    format!("今は{space}{label}を使います")
}

/// `dir` が share ディレクトリか (相対パスや末尾の区切りの違いは問わない)。
fn is_share_dir(dir: &Path, share: Option<&Path>) -> bool {
    let absolute = |p: &Path| std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf());
    share.is_some_and(|share| absolute(dir) == absolute(share))
}

/// `dict download` の進み具合の表示。
///
/// 取得を始めたら (最初の呼び出しで) 取得する辞書と保存先を標準出力に書く。受信量は標準エラーが
/// 端末のときだけ、`\r` で 1 行を書き換えて出す (0.2 秒以上の間隔で。受信し終えたときは必ず出す)。
struct DownloadProgress<'a> {
    dict: &'a Distributed,
    dir: &'a Path,
    /// 圧縮版を取るか (受け取る量と、展開後の大きさを分けて示す)。
    compressed: bool,
    terminal: bool,
    started: bool,
    shown: Option<Instant>,
}

impl<'a> DownloadProgress<'a> {
    fn new(dict: &'a Distributed, dir: &'a Path, compressed: bool, terminal: bool) -> Self {
        Self {
            dict,
            dir,
            compressed,
            terminal,
            started: false,
            shown: None,
        }
    }

    fn update(&mut self, received: u64, total: u64) {
        if !self.started {
            self.started = true;
            let mut out = io::stdout();
            let _ = writeln!(
                out,
                "{}",
                download_heading(self.dict, self.dir, self.compressed)
            );
            let _ = out.flush();
        }
        if !self.terminal {
            return;
        }
        let now = Instant::now();
        let due = self
            .shown
            .is_none_or(|last| now.duration_since(last) >= PROGRESS_INTERVAL);
        if !due && received < total {
            return;
        }
        self.shown = Some(now);
        let percent = (received * 100).checked_div(total).unwrap_or(100);
        let mut err = io::stderr();
        let _ = write!(
            err,
            "\r受信 {} / {} MB ({percent}%)",
            megabytes(received),
            megabytes(total)
        );
        let _ = err.flush();
    }

    /// 進み具合の行を閉じる (後の出力が同じ行に続かないように)。
    fn finish(&mut self) {
        if self.shown.take().is_some() {
            let _ = writeln!(io::stderr());
        }
    }
}

/// `dict download` が取得を始めるときの 1 行。圧縮版を取るなら、受け取る大きさと展開後の大きさを分けて示す
/// (受信の進み具合は、受け取る大きさに対して数える)。
fn download_heading(dict: &Distributed, dir: &Path, compressed: bool) -> String {
    let transfer = dict.transfer_size(compressed);
    let size = if transfer == dict.size {
        format!("{} MB", megabytes(dict.size))
    } else {
        format!(
            "圧縮版 {} MB、展開後 {} MB",
            megabytes(transfer),
            megabytes(dict.size)
        )
    };
    format!(
        "{} ({size}) を取得しています: {}",
        dict.name,
        display_path(dir)
    )
}

/// 大きさを 10 進の MB (1,000,000 バイト) で、小数 1 桁で表す。
fn megabytes(bytes: u64) -> String {
    format!("{:.1}", bytes as f64 / 1_000_000.0)
}

/// 利用者に見せるパス。ホームディレクトリの下なら `~` から書く。
fn display_path(path: &Path) -> String {
    if let Some(home) = std::env::home_dir()
        && !home.as_os_str().is_empty()
        && let Ok(rest) = path.strip_prefix(&home)
    {
        return if rest.as_os_str().is_empty() {
            "~".to_string()
        } else {
            format!("~{}{}", std::path::MAIN_SEPARATOR, rest.display())
        };
    }
    path.display().to_string()
}

fn dict_list(args: DictListArgs) -> u8 {
    let share = dictionaries::share_dir();
    let Some(dir) = args.dir.or_else(|| share.clone()) else {
        return error(
            "share ディレクトリが分かりません (HASAMI_DATA_DIR・XDG_DATA_HOME・HOME のどれも設定されていません。Windows では LOCALAPPDATA も見ます)。--dir で確かめる場所を指定してください",
        );
    };
    let listed = dictionaries::list(&dir);
    let in_share = is_share_dir(&dir, share.as_deref());
    // 辞書を指定しないときに今使う辞書 (share ディレクトリを見たときだけ示す)
    let pick = in_share.then(|| morph::auto_pick(&DictionarySearch::from_env()));
    let text = dict_list_text(
        &dir,
        in_share,
        &listed,
        cfg!(feature = "bundled-dict"),
        pick.as_ref(),
    );
    let mut out = io::stdout().lock();
    let written = out.write_all(text.as_bytes()).and_then(|()| out.flush());
    if let Err(e) = written
        && e.kind() != io::ErrorKind::BrokenPipe
    {
        return error(format!("出力に失敗しました: {e}"));
    }
    if listed.iter().any(|entry| entry.check.is_err()) {
        EXIT_ERROR
    } else {
        EXIT_OK
    }
}

/// `dict list` の表。`in_share` は `dir` が share ディレクトリか、`bundled` は辞書を同梱したビルドか、
/// `pick` は辞書を指定しないとき (`auto`) に今使う辞書。
fn dict_list_text(
    dir: &Path,
    in_share: bool,
    listed: &[dictionaries::Listed],
    bundled: bool,
    pick: Option<&AutoPick>,
) -> String {
    let mut rows = vec![[
        "名前".to_string(),
        "大きさ".to_string(),
        "状態".to_string(),
        "中身".to_string(),
    ]];
    for entry in listed {
        let state = match &entry.check {
            Ok(Check::Missing) => "未取得".to_string(),
            Ok(Check::Verified) => "取得済み (大きさと SHA-256 を確かめました)".to_string(),
            Ok(Check::Differs) => "中身が違う (hasami の別の版か、壊れています)".to_string(),
            Err(e) => format!("確かめられません ({e})"),
        };
        rows.push([
            entry.dictionary.name.to_string(),
            format!("{} MB", megabytes(entry.dictionary.size)),
            state,
            entry.dictionary.description().to_string(),
        ]);
    }
    let width = |column: usize| rows.iter().map(|r| r[column].width()).max().unwrap_or(0);
    let widths = [width(0), width(1), width(2)];

    let mut s = format!(
        "hasami {} の配布辞書 (保存先: {})\n\n",
        dictionaries::HASAMI_TAG,
        display_path(dir)
    );
    for (i, row) in rows.iter().enumerate() {
        // 大きさは右にそろえる (見出しは左)
        let size = if i == 0 {
            pad(&row[1], widths[1])
        } else {
            pad_start(&row[1], widths[1])
        };
        s.push_str(&format!(
            "{}  {}  {}  {}\n",
            pad(&row[0], widths[0]),
            size,
            pad(&row[2], widths[2]),
            row[3]
        ));
    }
    s.push('\n');
    if in_share {
        s.push_str(&format!(
            "取得するときは noslop dict download <名前> (名前を省くと {})\n",
            dictionaries::RECOMMENDED
        ));
        s.push_str(&share_usage("share:<名前>"));
        s.push('\n');
    } else {
        s.push_str(&format!(
            "取得するときは noslop dict download <名前> --dir {} (名前を省くと {})\n",
            dir.display(),
            dictionaries::RECOMMENDED
        ));
        s.push_str(
            "使うときは --dict にファイルのパスを指定してください (share:<名前> は share ディレクトリの辞書を指します)\n",
        );
    }
    let order = dictionaries::preference_order();
    let place = if in_share {
        "ここにある辞書"
    } else {
        "share ディレクトリの辞書"
    };
    let otherwise = if bundled {
        "、なければ同梱の ipadic を使います"
    } else {
        "ます (このビルドは辞書を同梱していません)"
    };
    let elsewhere = if in_share {
        ""
    } else {
        " (ここにある辞書は使いません)"
    };
    s.push_str(&format!(
        "辞書を指定しないとき (dictionary = \"auto\") は、{place}を hasami の推奨順 ({order}) で使い{otherwise}{elsewhere}\n"
    ));
    if let Some(pick) = pick {
        s.push_str(&match pick {
            AutoPick::Nothing => {
                "今は辞書が見つからないので、辞書なしの近似で判定します\n".to_string()
            }
            other => format!("{}\n", auto_pick_now(other)),
        });
    }
    s
}

/// 右にそろえる ([`pad`] の逆)。
fn pad_start(s: &str, width: usize) -> String {
    let w = s.width();
    if w >= width {
        s.to_string()
    } else {
        format!("{}{s}", " ".repeat(width - w))
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
    let env = Environment::from_process();
    let base = load_config_from(&args.config, start.as_deref(), &env)
        .map(|cfg| config_engine_options(&cfg, &env))
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
        let options = engine_options(
            &args.engine,
            &ConfigLayers::default(),
            &Environment::default(),
        );
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
        let o = engine_options(&args.engine, &project_only(loaded), &Environment::default());
        assert_eq!(o.genre, Genre::Business);
        assert!(
            o.experimental,
            "設定の experimental は CLI で指定しなくても効く"
        );
        assert_eq!(o.parse.line_breaks, LineBreakMode::Space);
        assert!(o.scope.tables);
        assert_eq!(o.selection.config_disable, vec!["X"]);
    }

    fn loaded(dir: &Path, name: &str, text: &str) -> LoadedConfig {
        let path = dir.join(name);
        LoadedConfig {
            file: config::ConfigFile::parse(text, &path).unwrap(),
            base_dir: dir.to_path_buf(),
            path,
        }
    }

    fn project_only(project: LoadedConfig) -> ConfigLayers {
        ConfigLayers {
            user: None,
            project: Some(project),
        }
    }

    /// ユーザーの設定の上に、プロジェクトの設定を項目ごとに重ねる。
    #[test]
    fn user_and_project_configs_are_layered_per_item() {
        let user_dir = Path::new("home").join(".config").join("noslop");
        let user = loaded(
            &user_dir,
            "config.toml",
            r#"
genre = "tech"
experimental = true
fail_on = "error"
[scope]
lists = true
[rules]
disable = ["R03"]
[rules.R01]
threshold = -0.5
[[custom]]
id = "X01"
pattern = "a"
message = "user"
[[custom]]
id = "X02"
pattern = "b"
message = "user"
[morphology]
dictionary = "dict/mine.hsd"
[files]
extensions = ["md"]
exclude = ["drafts/"]
"#,
        );
        let project = loaded(
            Path::new("project"),
            "noslop.toml",
            r#"
genre = "essay"
[rules]
enable = ["LONG_SENTENCE"]
[[custom]]
id = "x01"
pattern = "c"
message = "project"
[morphology]
mode = "required"
"#,
        );
        let layers = ConfigLayers {
            user: Some(user.clone()),
            project: Some(project.clone()),
        };
        let o = config_engine_options(&layers, &Environment::default());
        assert_eq!(
            o.genre,
            Genre::Essay,
            "プロジェクトに書いた項目はプロジェクト"
        );
        assert!(o.experimental, "プロジェクトに書いていない項目はユーザー");
        assert!(o.scope.lists && !o.scope.tables);
        assert_eq!(
            layers.pick(|f| f.fail_on),
            Some(FailOn::At(Severity::Error))
        );
        // ルールの選択と設定は層ごとに渡し、別名の解決と優先はエンジンに任せる
        assert_eq!(o.selection.user_disable, vec!["R03"]);
        assert_eq!(o.selection.config_enable, vec!["LONG_SENTENCE"]);
        assert!(o.user_rule_tables.contains_key("R01") && o.rule_tables.is_empty());
        // 独自ルールは ID (大文字小文字を問わない) でプロジェクトが置き換える
        let custom: Vec<(&str, &str)> = o
            .custom
            .iter()
            .map(|c| (c.id.as_str(), c.message.as_str()))
            .collect();
        assert_eq!(custom, [("X02", "user"), ("x01", "project")]);
        // 辞書の相対パスは、書いたファイル (ユーザーの設定) のディレクトリが基準
        assert_eq!(o.morphology.mode, MorphologyMode::Required);
        assert_eq!(
            o.morphology.dictionary,
            DictionaryChoice::File(user_dir.join("dict/mine.hsd"))
        );

        // ユーザーの exclude は検査の起点が基準。プロジェクトに exclude があれば (空でも) そちらだけ
        let walk = walk_options(&layers).unwrap();
        assert_eq!(walk.extensions, vec!["md"]);
        assert!(matches!(walk.exclude, Some(ExcludeRule::PerRoot(_))));
        let mut replaced = project.clone();
        replaced.file.files.exclude = Some(Vec::new());
        let walk = walk_options(&ConfigLayers {
            user: Some(user.clone()),
            project: Some(replaced.clone()),
        })
        .unwrap();
        assert!(walk.exclude.is_none());
        replaced.file.files.exclude = Some(vec!["vendor/".into()]);
        let walk = walk_options(&ConfigLayers {
            user: Some(user),
            project: Some(replaced),
        })
        .unwrap();
        assert!(matches!(walk.exclude, Some(ExcludeRule::Fixed(_))));

        // 設定がなければ既定値
        let o = config_engine_options(&ConfigLayers::default(), &Environment::default());
        assert_eq!(o.genre, Genre::General);
        assert_eq!(o.morphology.dictionary, DictionaryChoice::Auto);
        assert!(o.custom.is_empty());
    }

    /// ユーザーの設定は `<home>/.config/noslop/config.toml`。`--config` はプロジェクトの設定だけを
    /// 差し替え、`--no-config` はどちらも読まない。
    #[test]
    fn the_user_config_is_read_from_the_home_directory() {
        let home = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let args = |config: Option<PathBuf>, no_config: bool| ConfigArgs { config, no_config };
        let env = Environment {
            home: Some(home.path().to_path_buf()),
            ..Default::default()
        };
        let load = |a: &ConfigArgs| load_config_from(a, Some(project.path()), &env);

        // どちらもなければ空
        let layers = load(&args(None, false)).unwrap();
        assert!(layers.user.is_none() && layers.project.is_none());

        let user_path = config::user_config_path(home.path());
        assert_eq!(user_path, home.path().join(".config/noslop/config.toml"));
        std::fs::create_dir_all(user_path.parent().unwrap()).unwrap();
        std::fs::write(&user_path, "genre = \"tech\"\n").unwrap();
        std::fs::write(project.path().join("noslop.toml"), "genre = \"essay\"\n").unwrap();
        std::fs::write(other.path().join("other.toml"), "experimental = true\n").unwrap();

        let layers = load(&args(None, false)).unwrap();
        assert_eq!(layers.user.as_ref().unwrap().path, user_path);
        assert_eq!(layers.pick(|f| f.genre), Some(Genre::Essay));

        let layers = load(&args(Some(other.path().join("other.toml")), false)).unwrap();
        assert_eq!(
            layers.pick(|f| f.genre),
            Some(Genre::Tech),
            "--config でもユーザーの設定は重ねる"
        );
        assert_eq!(layers.pick(|f| f.experimental), Some(true));

        let layers = load(&args(None, true)).unwrap();
        assert!(layers.user.is_none() && layers.project.is_none());

        // ユーザーの設定の書式の誤りは、プロジェクトの設定と同じく設定の誤り
        std::fs::write(&user_path, "genre = 1\n").unwrap();
        let err = load(&args(None, false)).unwrap_err();
        assert!(err.to_string().contains("config.toml"), "{err}");
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
    fn parses_dict_subcommands() {
        let cli = Cli::try_parse_from(["noslop", "dict", "download"]).unwrap();
        let Command::Dict(DictCommand::Download(args)) = cli.command else {
            panic!("dict download");
        };
        assert_eq!(args.name, "ipadic-neologd-sudachi");
        assert_eq!(args.dir, None);
        assert_eq!(args.source, dictionaries::DEFAULT_SOURCE);
        assert!(!args.force);

        let cli = Cli::try_parse_from([
            "noslop",
            "dict",
            "download",
            "ipadic",
            "--dir",
            "d",
            "--source",
            "https://mirror.example.com/hasami",
            "--force",
        ])
        .unwrap();
        let Command::Dict(DictCommand::Download(args)) = cli.command else {
            panic!("dict download");
        };
        assert_eq!(args.name, "ipadic");
        assert_eq!(args.dir, Some(PathBuf::from("d")));
        assert_eq!(args.source, "https://mirror.example.com/hasami");
        assert!(args.force);
        // 名前は配布辞書の表にあるものだけ
        for name in dictionaries::DICTIONARIES.map(|d| d.name) {
            assert!(Cli::try_parse_from(["noslop", "dict", "download", name]).is_ok());
        }
        let err = Cli::try_parse_from(["noslop", "dict", "download", "unidic"]).unwrap_err();
        assert!(err.to_string().contains("ipadic-neologd-sudachi"), "{err}");

        let cli = Cli::try_parse_from(["noslop", "dict", "list", "--dir", "d"]).unwrap();
        let Command::Dict(DictCommand::List(args)) = cli.command else {
            panic!("dict list");
        };
        assert_eq!(args.dir, Some(PathBuf::from("d")));
    }

    #[test]
    fn keywords_and_share_specs_in_the_config_are_not_relative_paths() {
        let loaded = LoadedConfig {
            path: Path::new("conf").join("noslop.toml"),
            base_dir: PathBuf::from("conf"),
            file: config::ConfigFile::default(),
        };
        let resolve = |spec: &str| config_dictionary(&loaded, Path::new(spec), None);
        assert_eq!(
            resolve("share:ipadic-neologd"),
            DictionaryChoice::Share("ipadic-neologd".into())
        );
        assert_eq!(resolve("auto"), DictionaryChoice::Auto);
        assert_eq!(resolve("bundled"), DictionaryChoice::Bundled);
        assert_eq!(
            resolve("test.hsd"),
            DictionaryChoice::File(Path::new("conf").join("test.hsd"))
        );
        // auto という名前のファイルは ./auto と書く
        assert_eq!(
            resolve("./auto"),
            DictionaryChoice::File(Path::new("conf").join("./auto"))
        );
    }

    #[test]
    fn sizes_are_shown_in_decimal_megabytes() {
        assert_eq!(megabytes(18_125_804), "18.1");
        assert_eq!(megabytes(237_760_279), "237.8");
        assert_eq!(megabytes(0), "0.0");
    }

    #[test]
    fn paths_under_the_home_directory_start_with_a_tilde() {
        let Some(home) = std::env::home_dir().filter(|h| h.is_absolute()) else {
            return;
        };
        let sep = std::path::MAIN_SEPARATOR;
        assert_eq!(
            display_path(&home.join(".local").join("share")),
            format!("~{sep}.local{sep}share")
        );
        assert_eq!(display_path(&home), "~");
        assert_eq!(display_path(Path::new("rel")), "rel");
    }

    #[test]
    fn dict_list_shows_the_state_of_each_dictionary() {
        let listed: Vec<dictionaries::Listed> = dictionaries::DICTIONARIES
            .iter()
            .zip([Ok(Check::Verified), Ok(Check::Differs), Ok(Check::Missing)])
            .map(|(dictionary, check)| dictionaries::Listed {
                dictionary,
                path: PathBuf::from(dictionary.file_name()),
                check,
            })
            .collect();
        // 目録 (dict/catalog.json) は Release のたびに変わりうるので、名前と大きさは目録から組み立てる
        let [verified, differs, missing] = [0, 1, 2].map(|i| listed[i].dictionary);
        let chosen = AutoPick::Share(Path::new("share").join(verified.file_name()));
        let text = dict_list_text(Path::new("share"), true, &listed, true, Some(&chosen));
        let line = |name: &str| {
            text.lines()
                .find(|l| l.split_whitespace().next() == Some(name))
                .unwrap_or_else(|| panic!("{name}\n{text}"))
        };
        assert!(text.starts_with(&format!(
            "hasami {} の配布辞書 (保存先: share)\n",
            dictionaries::HASAMI_TAG
        )));
        assert!(line(verified.name).contains(&format!(
            "{} MB  取得済み (大きさと SHA-256 を確かめました)",
            megabytes(verified.size)
        )));
        assert!(line(differs.name).contains("中身が違う (hasami の別の版か、壊れています)"));
        assert!(line(missing.name).contains(&format!("{} MB  未取得", megabytes(missing.size))));
        let recommended = dictionaries::find(dictionaries::RECOMMENDED).unwrap();
        assert!(line(recommended.name).ends_with(recommended.description()));
        assert!(text.contains(&format!(
            "noslop dict download <名前> (名前を省くと {})",
            dictionaries::RECOMMENDED
        )));
        assert!(text.contains("dictionary = \"share:<名前>\""));
        let order = dictionaries::preference_order();
        assert!(
            text.contains(&format!(
                "辞書を指定しないとき (dictionary = \"auto\") は、ここにある辞書を hasami の推奨順 ({order}) で使い、なければ同梱の ipadic を使います\n"
            )),
            "{text}"
        );
        assert!(
            text.ends_with(&format!(
                "今は {} ({}) を使います\n",
                verified.name,
                display_path(&Path::new("share").join(verified.file_name()))
            )),
            "{text}"
        );
        let bundled = dict_list_text(
            Path::new("share"),
            true,
            &listed,
            true,
            Some(&AutoPick::Bundled),
        );
        assert!(
            bundled.ends_with("今は同梱の ipadic を使います\n"),
            "{bundled}"
        );

        // share ディレクトリでない場所には、今使う辞書を出さない
        let text = dict_list_text(Path::new("elsewhere"), false, &listed, false, None);
        assert!(text.contains("--dir elsewhere"), "{text}");
        assert!(!text.contains("dictionary = \"share:"), "{text}");
        assert!(!text.contains("今は"), "{text}");
        assert!(
            text.contains(&format!(
                "share ディレクトリの辞書を hasami の推奨順 ({order}) で使います (このビルドは辞書を同梱していません) (ここにある辞書は使いません)"
            )),
            "{text}"
        );
        let text = dict_list_text(
            Path::new("share"),
            true,
            &listed,
            false,
            Some(&AutoPick::Nothing),
        );
        assert!(
            text.contains(&format!(
                "ここにある辞書を hasami の推奨順 ({order}) で使います (このビルドは辞書を同梱していません)\n"
            )),
            "{text}"
        );
        assert!(text.ends_with("今は辞書が見つからないので、辞書なしの近似で判定します\n"));
    }

    /// 取得できたときは結果と使い方を出す。share ディレクトリなら share:<名前>、ほかの場所ならファイルの
    /// パスで指定するよう案内する (統合テストは目録の実物を取れないので、ここで確かめる)。
    #[test]
    fn download_reports_the_result_and_how_to_use_the_dictionary() {
        let dict = dictionaries::find(dictionaries::RECOMMENDED).unwrap();
        let share = Path::new("share");
        let placed = share.join(dict.file_name());
        let name = dict.name;

        let this = AutoPick::Share(placed.clone());
        let downloaded = download_report(
            dict,
            &Outcome::Downloaded(placed.clone()),
            share,
            Some(share),
            &this,
        );
        let lines: Vec<&str> = downloaded.lines().collect();
        assert!(lines[0].starts_with(&format!(
            "{name} を取得しました (大きさ・SHA-256・辞書の形式を確かめました): "
        )));
        assert_eq!(
            lines[1],
            "辞書を指定しないとき (dictionary = \"auto\") は、この辞書を使います"
        );
        assert_eq!(
            lines[2],
            format!(
                "決まった辞書を使うときは --dict share:{name} か、設定ファイルの [morphology] に dictionary = \"share:{name}\" を書いてください"
            )
        );

        // 推奨順で先の辞書があれば、そちらを使うと知らせる
        let better = AutoPick::Share(share.join("better.hsd"));
        let present = download_report(dict, &Outcome::Present(placed), share, Some(share), &better);
        assert!(present.starts_with(&format!(
            "{name} は取得済みです (大きさと SHA-256 を確かめました): "
        )));
        assert!(
            present.contains("辞書を指定しないとき (dictionary = \"auto\") は、better ("),
            "{present}"
        );

        let elsewhere = Path::new("elsewhere");
        let report = download_report(
            dict,
            &Outcome::Downloaded(elsewhere.join(dict.file_name())),
            elsewhere,
            Some(share),
            &this,
        );
        assert!(report.contains("使うときは --dict にこのファイルのパスを指定してください"));
        assert!(!report.contains(&format!("share:{name}")), "{report}");
        assert!(!report.contains("auto"), "{report}");
    }

    #[test]
    fn markdown_headings_are_demoted_under_rule_sections() {
        assert_eq!(
            demote_headings("# 見出し\n## 小見出し\n### そのまま\n本文"),
            "### 見出し\n### 小見出し\n### そのまま\n本文"
        );
    }
}
