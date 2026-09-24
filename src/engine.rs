//! ルールの選択と実行。
//!
//! ルールを有効にするかは次の順で決める。
//!
//! 1. `--only-rules` があれば、列挙したルールだけを動かす (`--ignore-rules` で消したものを除く)
//! 2. CLI の明示 (`--ignore-rules` / `--enable-rules`) があればそれに従う。同じ層では無効が優先
//! 3. 設定ファイルの明示 (`[rules] disable` / `enable`、`[rules.X] enabled`) があればそれに従う。
//!    同じ層では無効が優先。CLI の明示は設定より優先する
//! 4. どれもなければ既定: `--no-readability` なら読みやすさのレーンは止め、それ以外は
//!    「校正済み (stable) か `--experimental`」かつ「そのジャンルで既定で動かすルール」なら動かす

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;

use rayon::prelude::*;

use crate::config::{ConfigError, CustomRuleConfig, RuleTable};
use crate::diagnostic::{Diagnostic, Lane, RuleStatus, Severity};
use crate::document::{Document, ParseOptions, SourceFormat};
use crate::genre::Genre;
use crate::morph::{self, DocMorphology, Morphology, MorphologyOptions, MorphologyStatus};
use crate::rules::custom::CustomRule;
use crate::rules::{Rule, RuleContext, Scope, builtin_rules};
use crate::score::{self, Score};
use crate::suppress::{self, RuleNames};

/// ルールの明示的な有効化・無効化の指定。
#[derive(Debug, Clone, Default)]
pub struct Selection {
    /// 設定ファイルの `[rules] enable`。
    pub config_enable: Vec<String>,
    /// 設定ファイルの `[rules] disable`。
    pub config_disable: Vec<String>,
    /// `--enable-rules`。
    pub cli_enable: Vec<String>,
    /// `--ignore-rules`。
    pub cli_disable: Vec<String>,
    /// `--only-rules`。
    pub only: Option<Vec<String>>,
    /// `--no-readability`。
    pub no_readability: bool,
}

/// エンジンの設定 (設定ファイルと CLI を合わせたもの)。
#[derive(Debug, Clone, Default)]
pub struct EngineOptions {
    pub genre: Genre,
    pub experimental: bool,
    pub scope: Scope,
    pub parse: ParseOptions,
    pub selection: Selection,
    /// `[rules.<ID か名前>]` の設定。
    pub rule_tables: BTreeMap<String, RuleTable>,
    pub custom: Vec<CustomRuleConfig>,
    /// 形態素解析の辞書 (`[morphology]`・`--dict`・`--no-dict`)。
    pub morphology: MorphologyOptions,
}

/// エンジンが持つルール 1 つ。
pub struct RuleEntry {
    pub rule: Box<dyn Rule>,
    /// この実行で動かすか。
    pub enabled: bool,
    /// 組み込みルールか (独自ルールなら false)。
    pub builtin: bool,
    /// 診断に適用する重大度の上書き (`[rules.X] severity`)。
    pub severity: Option<Severity>,
}

/// lint のエンジン。
pub struct Engine {
    entries: Vec<RuleEntry>,
    names: RuleNames,
    builtin_ids: HashSet<String>,
    options: EngineOptions,
    /// 読み込んだ辞書 (辞書を使う有効なルールがあり、辞書が見つかったときだけ)。
    morphology: Option<Morphology>,
    morphology_status: MorphologyStatus,
}

impl Engine {
    /// 組み込みルールと設定の独自ルールでエンジンを作る。
    pub fn new(options: EngineOptions) -> Result<Self, ConfigError> {
        let builtin = builtin_rules(options.genre);
        Self::with_rules(builtin, options)
    }

    /// 組み込みルールの代わりに `builtin` を使ってエンジンを作る (テスト用)。
    pub fn with_rules(
        builtin: Vec<Box<dyn Rule>>,
        options: EngineOptions,
    ) -> Result<Self, ConfigError> {
        let builtin_ids: HashSet<String> =
            builtin.iter().map(|r| r.meta().id.to_string()).collect();
        let mut rules: Vec<(Box<dyn Rule>, bool)> =
            builtin.into_iter().map(|r| (r, true)).collect();

        let mut taken: HashMap<String, String> = HashMap::new();
        for (rule, _) in &rules {
            let m = rule.meta();
            taken.insert(m.id.to_ascii_lowercase(), m.id.to_string());
            taken.insert(m.name.to_ascii_lowercase(), m.id.to_string());
        }
        for cfg in &options.custom {
            let rule = CustomRule::from_config(cfg)?;
            let m = rule.meta();
            for key in [m.id, m.name] {
                if let Some(owner) = taken.get(&key.to_ascii_lowercase()) {
                    return Err(ConfigError::Invalid(format!(
                        "独自ルール {} の ID か名前「{key}」が既存のルール {owner} と重なっています",
                        m.id
                    )));
                }
            }
            taken.insert(m.id.to_ascii_lowercase(), m.id.to_string());
            taken.insert(m.name.to_ascii_lowercase(), m.id.to_string());
            rules.push((Box::new(rule), false));
        }

        let names = RuleNames::new(rules.iter().map(|(r, _)| (r.meta().id, r.meta().name)));
        let resolve_all = |keys: &[String], source: &str| -> Result<HashSet<String>, ConfigError> {
            keys.iter()
                .map(|k| {
                    names.resolve(k).map(str::to_string).ok_or_else(|| {
                        ConfigError::Invalid(format!("{source} に未知のルールがあります: {k}"))
                    })
                })
                .collect()
        };
        let sel = &options.selection;
        let config_enable = resolve_all(&sel.config_enable, "設定ファイルの [rules] enable")?;
        let config_disable = resolve_all(&sel.config_disable, "設定ファイルの [rules] disable")?;
        let cli_enable = resolve_all(&sel.cli_enable, "--enable-rules")?;
        let cli_disable = resolve_all(&sel.cli_disable, "--ignore-rules")?;
        let only = sel
            .only
            .as_ref()
            .map(|keys| resolve_all(keys, "--only-rules"))
            .transpose()?;

        let mut tables: HashMap<String, &RuleTable> = HashMap::new();
        for (key, table) in &options.rule_tables {
            let id = names.resolve(key).ok_or_else(|| {
                ConfigError::Invalid(format!("設定ファイルの [rules.{key}] は未知のルールです"))
            })?;
            if tables.insert(id.to_string(), table).is_some() {
                return Err(ConfigError::Invalid(format!(
                    "設定ファイルに {id} の設定が ID と名前で重複しています"
                )));
            }
        }

        let mut entries = Vec::new();
        for (mut rule, is_builtin) in rules {
            let meta = rule.meta();
            let table = tables.get(meta.id).copied();
            if let Some(table) = table {
                for (key, value) in &table.options {
                    rule.configure(key, value).map_err(|e| {
                        ConfigError::Invalid(format!("設定ファイルの [rules.{}]: {e}", meta.id))
                    })?;
                }
            }
            let enabled = if let Some(only) = &only {
                only.contains(meta.id) && !cli_disable.contains(meta.id)
            } else {
                let cli_state = if cli_disable.contains(meta.id) {
                    Some(false)
                } else if cli_enable.contains(meta.id) {
                    Some(true)
                } else {
                    None
                };
                let config_state = if config_disable.contains(meta.id)
                    || table.and_then(|t| t.enabled) == Some(false)
                {
                    Some(false)
                } else if config_enable.contains(meta.id)
                    || table.and_then(|t| t.enabled) == Some(true)
                {
                    Some(true)
                } else {
                    None
                };
                match cli_state.or(config_state) {
                    Some(explicit) => explicit,
                    None if sel.no_readability && meta.lane == Lane::Readability => false,
                    None => {
                        (meta.status == RuleStatus::Stable || options.experimental)
                            && rule.allowed_in(options.genre)
                    }
                }
            };
            entries.push(RuleEntry {
                rule,
                enabled,
                builtin: is_builtin,
                severity: table.and_then(|t| t.severity),
            });
        }

        // 辞書は、辞書を使う有効なルールがあるときだけ探して読む
        let needed = entries
            .iter()
            .any(|e| e.enabled && e.rule.uses_morphology());
        let (morphology, morphology_status) =
            morph::resolve(&options.morphology, needed).map_err(ConfigError::Invalid)?;

        Ok(Self {
            entries,
            names,
            builtin_ids,
            options,
            morphology,
            morphology_status,
        })
    }

    pub fn options(&self) -> &EngineOptions {
        &self.options
    }

    /// この実行で使う判定の方式 (辞書の有無)。
    pub fn morphology(&self) -> &MorphologyStatus {
        &self.morphology_status
    }

    /// 文書の形態素の層 (辞書を読み込んでいるときだけ)。
    pub fn doc_morphology<'a>(&self, doc: &'a Document) -> Option<DocMorphology<'a>> {
        self.morphology.as_ref().map(|m| m.for_document(doc))
    }

    /// すべてのルール (有効・無効を問わず)。組み込みルール、独自ルールの順。
    pub fn entries(&self) -> &[RuleEntry] {
        &self.entries
    }

    /// ID か名前 (大文字小文字は区別しない) でルールを探す。
    pub fn find(&self, key: &str) -> Option<&RuleEntry> {
        let id = self.names.resolve(key)?;
        self.entries.iter().find(|e| e.rule.meta().id == id)
    }

    /// 有効なルールの ID。
    pub fn active_ids(&self) -> Vec<&'static str> {
        self.entries
            .iter()
            .filter(|e| e.enabled)
            .map(|e| e.rule.meta().id)
            .collect()
    }

    /// 組み込みルールの ID か。
    pub fn is_builtin(&self, id: &str) -> bool {
        self.builtin_ids.contains(id)
    }

    /// 原文を読み込んで lint する。
    pub fn lint_source(&self, name: String, source: String, format: SourceFormat) -> FileReport {
        let doc = Document::parse(name, source, format, &self.options.parse);
        self.lint(doc)
    }

    /// 読み込み済みの文書を lint する。
    pub fn lint(&self, doc: Document) -> FileReport {
        let mut diagnostics = Vec::new();
        {
            let morph = self.doc_morphology(&doc);
            let ctx = RuleContext {
                doc: &doc,
                genre: self.options.genre,
                scope: self.options.scope,
                experimental: self.options.experimental,
                morph: morph.as_ref(),
            };
            for entry in self.entries.iter().filter(|e| e.enabled) {
                let start = diagnostics.len();
                entry.rule.check(&ctx, &mut diagnostics);
                if let Some(severity) = entry.severity {
                    for d in &mut diagnostics[start..] {
                        d.severity = severity;
                    }
                }
            }
        }
        for d in &mut diagnostics {
            d.fingerprint = fingerprint(&doc, d);
        }
        let warnings = suppress::apply(&doc, &self.names, &mut diagnostics);
        diagnostics.sort_by(|a, b| {
            (a.span.start, a.rule_id.as_str(), a.span.end).cmp(&(
                b.span.start,
                b.rule_id.as_str(),
                b.span.end,
            ))
        });
        let score = score::compute(
            doc.char_count(),
            diagnostics
                .iter()
                .filter(|d| score::counts(d, |id| self.is_builtin(id))),
        );
        FileReport {
            doc,
            diagnostics,
            warnings,
            score,
        }
    }

    /// 入力をすべて lint する。ファイルは並列に処理し、結果は入力の順に並べる。
    pub fn run(&self, inputs: Vec<Input>) -> RunReport {
        let results: Vec<Result<FileReport, FileError>> = inputs
            .into_par_iter()
            .map(|input| self.process(input))
            .collect();
        let mut report = RunReport {
            morphology: self.morphology_status.clone(),
            ..Default::default()
        };
        for result in results {
            match result {
                Ok(file) => report.files.push(file),
                Err(err) => report.errors.push(err),
            }
        }
        report
    }

    fn process(&self, input: Input) -> Result<FileReport, FileError> {
        match input {
            Input::Path(path) => {
                let name = crate::walk::display(&path);
                let bytes = std::fs::read(&path).map_err(|e| FileError {
                    path: name.clone(),
                    message: format!("読み込めません: {e}"),
                })?;
                let source = String::from_utf8(bytes).map_err(|_| FileError {
                    path: name.clone(),
                    message: "UTF-8 として読めません (文字コードを UTF-8 にしてください)"
                        .to_string(),
                })?;
                Ok(self.lint_source(name, source, SourceFormat::from_path(&path)))
            }
            Input::Text {
                name,
                source,
                format,
            } => Ok(self.lint_source(name, source, format)),
        }
    }
}

/// lint の入力。
#[derive(Debug, Clone)]
pub enum Input {
    Path(PathBuf),
    /// 標準入力などから読んだ文字列。
    Text {
        name: String,
        source: String,
        format: SourceFormat,
    },
}

/// 1 ファイルの結果。
#[derive(Debug)]
pub struct FileReport {
    pub doc: Document,
    /// 指摘 (抑制したものを含む)。位置とルール ID の順。
    pub diagnostics: Vec<Diagnostic>,
    /// 抑制コメントの誤りなど、そのファイルについての警告。
    pub warnings: Vec<String>,
    pub score: Option<Score>,
}

impl FileReport {
    pub fn path(&self) -> &str {
        &self.doc.name
    }

    /// 抑制していない指摘。
    pub fn visible(&self) -> impl Iterator<Item = &Diagnostic> {
        self.diagnostics.iter().filter(|d| !d.is_suppressed())
    }
}

/// 読めなかった入力。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileError {
    pub path: String,
    pub message: String,
}

/// 実行全体の結果。
#[derive(Debug, Default)]
pub struct RunReport {
    pub files: Vec<FileReport>,
    pub errors: Vec<FileError>,
    /// 判定の方式 (形態素解析の辞書を使ったか)。
    pub morphology: MorphologyStatus,
}

impl RunReport {
    /// `fail_on` 以上の未抑制の指摘があるか。
    pub fn trips(&self, fail_on: crate::config::FailOn) -> bool {
        self.files
            .iter()
            .flat_map(FileReport::visible)
            .any(|d| fail_on.trips(d.severity))
    }
}

/// 前回の結果と突き合わせるための識別子。
///
/// 行番号を使わず、ルール ID・空白を除いた一致テキスト・文脈の先頭 32 字から作る
/// (上に行を挿入しても変わらないように)。同じ文が繰り返されると同じ値になるため、
/// 突き合わせは多重集合として行う。
pub fn fingerprint(doc: &Document, d: &Diagnostic) -> String {
    let matched: String = doc
        .slice(d.span)
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    let context: String = d
        .context
        .map(|c| {
            doc.slice(c)
                .chars()
                .filter(|c| !c.is_whitespace())
                .take(32)
                .collect()
        })
        .unwrap_or_default();
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for part in [d.rule_id.as_str(), &matched, &context] {
        for byte in part.bytes().chain(std::iter::once(0xff)) {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
    }
    format!("{hash:016x}")
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::diagnostic::Span;
    use crate::rules::RuleMeta;

    /// テスト用のルール: 文の中の `needle` を指す。
    pub(crate) struct NeedleRule {
        pub meta: &'static RuleMeta,
        pub needle: &'static str,
        pub allowed: bool,
    }

    impl Rule for NeedleRule {
        fn meta(&self) -> &'static RuleMeta {
            self.meta
        }

        fn allowed_in(&self, genre: Genre) -> bool {
            self.allowed || genre != Genre::Business
        }

        fn configure(&mut self, key: &str, value: &toml::Value) -> Result<(), String> {
            match key {
                "needle" => {
                    let s = value
                        .as_str()
                        .ok_or("needle には文字列を指定してください")?;
                    self.needle = Box::leak(s.to_string().into_boxed_str());
                    Ok(())
                }
                _ => Err(format!(
                    "{} に `{key}` という設定項目はありません",
                    self.meta.id
                )),
            }
        }

        fn check(&self, ctx: &RuleContext<'_>, out: &mut Vec<Diagnostic>) {
            for (idx, block) in ctx.scoped_blocks() {
                for s in ctx.doc.block_sentences(idx) {
                    let text = &block.text[s.range.clone()];
                    for (pos, _) in text.match_indices(self.needle) {
                        let start = s.range.start + pos;
                        let span = block.to_source(start..start + self.needle.len());
                        out.push(
                            self.meta
                                .diagnostic(span, format!("「{}」があります", self.needle))
                                .with_context(s.span),
                        );
                    }
                }
            }
        }
    }

    macro_rules! meta {
        ($id:literal, $name:literal, $lane:expr, $status:expr, $sev:expr) => {{
            static META: RuleMeta = RuleMeta {
                id: $id,
                name: $name,
                title: "テスト",
                lane: $lane,
                status: $status,
                default_severity: $sev,
                summary: "テスト用",
                explanation: "テスト用",
            };
            &META
        }};
    }

    /// stable / experimental / readability / business で止まる、の 4 つのテスト用ルール。
    pub(crate) fn test_rules() -> Vec<Box<dyn Rule>> {
        vec![
            Box::new(NeedleRule {
                meta: meta!(
                    "T01",
                    "STABLE_SLOP",
                    Lane::Slop,
                    RuleStatus::Stable,
                    Severity::Warning
                ),
                needle: "言えるでしょう",
                allowed: true,
            }),
            Box::new(NeedleRule {
                meta: meta!(
                    "T02",
                    "EXPERIMENTAL_SLOP",
                    Lane::Slop,
                    RuleStatus::Experimental,
                    Severity::Info
                ),
                needle: "様々な",
                allowed: true,
            }),
            Box::new(NeedleRule {
                meta: meta!(
                    "T03",
                    "READABILITY",
                    Lane::Readability,
                    RuleStatus::Stable,
                    Severity::Info
                ),
                needle: "の設定の",
                allowed: true,
            }),
            Box::new(NeedleRule {
                meta: meta!(
                    "T04",
                    "NOT_FOR_BUSINESS",
                    Lane::Slop,
                    RuleStatus::Stable,
                    Severity::Info
                ),
                needle: "まとめ",
                allowed: false,
            }),
        ]
    }

    fn engine(options: EngineOptions) -> Engine {
        Engine::with_rules(test_rules(), options).unwrap()
    }

    fn ids(options: EngineOptions) -> Vec<&'static str> {
        engine(options).active_ids()
    }

    fn sel(f: impl FnOnce(&mut Selection)) -> EngineOptions {
        let mut options = EngineOptions::default();
        f(&mut options.selection);
        options
    }

    #[test]
    fn default_selection_uses_status_and_genre() {
        assert_eq!(ids(EngineOptions::default()), vec!["T01", "T03", "T04"]);
        let experimental = EngineOptions {
            experimental: true,
            ..Default::default()
        };
        assert_eq!(ids(experimental), vec!["T01", "T02", "T03", "T04"]);
        let business = EngineOptions {
            genre: Genre::Business,
            ..Default::default()
        };
        assert_eq!(ids(business), vec!["T01", "T03"]);
    }

    #[test]
    fn explicit_disable_wins_within_a_layer_and_cli_wins_over_config() {
        let o = sel(|s| {
            s.config_enable = vec!["T02".into()];
            s.config_disable = vec!["T02".into(), "t01".into()];
        });
        assert_eq!(ids(o), vec!["T03", "T04"]);
        let o = sel(|s| {
            s.config_disable = vec!["T01".into()];
            s.cli_enable = vec!["STABLE_SLOP".into()];
        });
        assert_eq!(ids(o), vec!["T01", "T03", "T04"]);
        let o = sel(|s| {
            s.config_enable = vec!["T02".into()];
            s.cli_disable = vec!["T02".into()];
        });
        assert_eq!(ids(o), vec!["T01", "T03", "T04"]);
    }

    #[test]
    fn explicit_enable_overrides_genre_and_status() {
        let mut o = sel(|s| s.cli_enable = vec!["T02".into(), "T04".into()]);
        o.genre = Genre::Business;
        assert_eq!(ids(o), vec!["T01", "T02", "T03", "T04"]);
    }

    #[test]
    fn only_rules_restricts_and_no_readability_keeps_explicit_enables() {
        let o = sel(|s| {
            s.only = Some(vec!["T02".into(), "T03".into()]);
            s.cli_disable = vec!["T03".into()];
            s.config_enable = vec!["T01".into()];
        });
        assert_eq!(ids(o), vec!["T02"]);
        let o = sel(|s| s.no_readability = true);
        assert_eq!(ids(o), vec!["T01", "T04"]);
        let o = sel(|s| {
            s.no_readability = true;
            s.config_enable = vec!["T03".into()];
        });
        assert_eq!(ids(o), vec!["T01", "T03", "T04"]);
    }

    #[test]
    fn unknown_rules_are_errors() {
        let o = sel(|s| s.cli_disable = vec!["NOPE".into()]);
        let err = Engine::with_rules(test_rules(), o).err().unwrap();
        assert!(err.to_string().contains("NOPE"));
        let mut o = EngineOptions::default();
        o.rule_tables.insert("NOPE".into(), RuleTable::default());
        assert!(Engine::with_rules(test_rules(), o).is_err());
    }

    #[test]
    fn rule_tables_override_severity_and_pass_options() {
        let mut o = EngineOptions::default();
        let mut table = RuleTable {
            severity: Some(Severity::Error),
            ..Default::default()
        };
        table
            .options
            .insert("needle".into(), toml::Value::String("言える".into()));
        o.rule_tables.insert("stable_slop".into(), table);
        let e = engine(o);
        let report = e.lint(Document::markdown("これは言えるでしょう。\n"));
        let d = report
            .diagnostics
            .iter()
            .find(|d| d.rule_id == "T01")
            .unwrap();
        assert_eq!(d.severity, Severity::Error);
        assert_eq!(report.doc.slice(d.span), "言える");

        let mut bad = EngineOptions::default();
        let mut table = RuleTable::default();
        table
            .options
            .insert("unknown".into(), toml::Value::Integer(1));
        bad.rule_tables.insert("T01".into(), table);
        let err = Engine::with_rules(test_rules(), bad).err().unwrap();
        assert!(err.to_string().contains("unknown"), "{err}");
    }

    #[test]
    fn custom_rules_join_selection_and_cannot_shadow_builtin_ids() {
        let custom = CustomRuleConfig {
            id: "X01".into(),
            name: Some("TEAM_TERM".into()),
            pattern: "ユーザー様".into(),
            regex: false,
            message: "表記".into(),
            hint: None,
            severity: None,
            lane: None,
        };
        let o = EngineOptions {
            custom: vec![custom.clone()],
            ..Default::default()
        };
        let e = engine(o);
        assert!(e.active_ids().contains(&"X01"));
        assert!(!e.is_builtin("X01"));
        let report = e.lint(Document::markdown("ユーザー様と言えるでしょう。\n"));
        assert_eq!(report.diagnostics.len(), 2);
        // 独自ルールはスコアに入らない: stable の T01 (warning) だけが減点される
        assert_eq!(report.score, None, "100 字未満はスコアなし");

        let clash = EngineOptions {
            custom: vec![CustomRuleConfig {
                id: "t01".into(),
                ..custom
            }],
            ..Default::default()
        };
        assert!(Engine::with_rules(test_rules(), clash).is_err());
    }

    #[test]
    fn lint_sorts_fingerprints_and_suppresses() {
        let e = engine(EngineOptions::default());
        let src = "<!-- noslop-disable-next-line T01 -- 引用 -->\nこれは言えるでしょう。\n\nそれも言えるでしょう。上限の設定の検討。\n";
        let report = e.lint(Document::markdown(src));
        let rows: Vec<_> = report
            .diagnostics
            .iter()
            .map(|d| (d.rule_id.as_str(), d.is_suppressed()))
            .collect();
        assert_eq!(rows, vec![("T01", true), ("T01", false), ("T03", false)]);
        assert!(report.diagnostics.iter().all(|d| d.fingerprint.len() == 16));
        assert!(report.warnings.is_empty());
    }

    #[test]
    fn fingerprint_ignores_line_moves_but_tracks_text() {
        let e = engine(EngineOptions::default());
        let a = e.lint(Document::markdown("前置き。\n\nこれは言えるでしょう。\n"));
        let b = e.lint(Document::markdown(
            "前置き。\n\n追加の段落。\n\n\nこれは言えるでしょう。\n",
        ));
        let c = e.lint(Document::markdown("前置き。\n\nあれは言えるでしょう。\n"));
        assert_eq!(a.diagnostics[0].fingerprint, b.diagnostics[0].fingerprint);
        assert_ne!(a.diagnostics[0].fingerprint, c.diagnostics[0].fingerprint);
    }

    #[test]
    fn score_counts_only_stable_slop_diagnostics() {
        let e = engine(EngineOptions {
            experimental: true,
            ..Default::default()
        });
        // 100 字以上の文書に stable の T01 (warning) が 1 件、実験的な T02 と読みやすさのレーンの T03 が 1 件ずつ
        let filler = "これは十分な長さの文章を作るための文です。".repeat(6);
        let src = format!("{filler}\n\nそれは言えるでしょう。様々な上限の設定の検討。\n");
        let report = e.lint(Document::markdown(src));
        let score = report.score.unwrap();
        assert_eq!(score.deduction, 4.0);
        assert_eq!(score.value, 96);
    }

    #[test]
    fn run_collects_errors_and_keeps_input_order() {
        let dir = tempfile::tempdir().unwrap();
        let good = dir.path().join("a.md");
        let bad = dir.path().join("b.md");
        std::fs::write(&good, "これは言えるでしょう。\n").unwrap();
        std::fs::write(&bad, [0xff, 0xfe, 0x00]).unwrap();
        let e = engine(EngineOptions::default());
        let report = e.run(vec![
            Input::Path(good),
            Input::Path(bad),
            Input::Text {
                name: "<stdin>".into(),
                source: "様々な話。\n".into(),
                format: SourceFormat::Markdown,
            },
        ]);
        assert_eq!(report.files.len(), 2);
        assert_eq!(report.files[1].path(), "<stdin>");
        assert_eq!(report.errors.len(), 1);
        assert!(report.errors[0].message.contains("UTF-8"));
        assert!(report.trips(crate::config::FailOn::At(Severity::Warning)));
        assert!(!report.trips(crate::config::FailOn::Never));
    }

    #[test]
    fn severity_override_applies_before_fail_on() {
        let mut o = EngineOptions::default();
        o.rule_tables.insert(
            "T01".into(),
            RuleTable {
                severity: Some(Severity::Info),
                ..Default::default()
            },
        );
        let e = engine(o);
        let report = RunReport {
            files: vec![e.lint(Document::markdown("これは言えるでしょう。\n"))],
            errors: Vec::new(),
            morphology: Default::default(),
        };
        assert!(!report.trips(crate::config::FailOn::At(Severity::Warning)));
        assert!(report.trips(crate::config::FailOn::At(Severity::Info)));
        let _ = Span::new(0, 0);
    }
}
