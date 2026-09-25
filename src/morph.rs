//! 形態素解析 (辞書を使う精密判定)。
//!
//! 形態素解析器 [hasami](https://github.com/owayo/hasami) の辞書 (`.hsd`) で、品詞で数えるルール
//! (P15・P16) を判定する。既定の辞書の指定 (`auto`) は、hasami の share ディレクトリに取得した配布辞書を
//! hasami の推奨順で選び、なければバイナリに埋め込んだ IPAdic の辞書 (`dict/ipadic.hsd`。feature
//! `bundled-dict`) を使うので、何も指定しなくても辞書で判定する。辞書を使わないとき (`--no-dict`・
//! 同梱しないビルドで辞書が見つからないとき) は、それらのルールは辞書なしの近似で判定する。
//!
//! - 辞書は実行ごとに 1 度だけ読む ([`resolve`])。ファイルの辞書は mmap し、同梱の辞書はバイナリに
//!   埋め込んだバイト列を複製せずに読む。どちらも読み込みと解析で触れたページだけがメモリに載る。
//!   同梱の辞書はプロセスで 1 度だけ組み立てて共有する
//! - 辞書を使う有効なルールがなければ、辞書を探さない (フックのように起動の速さが要る場面のため)
//! - 文書ごとに解析器を作り ([`DocMorphology`])、ルールが求めた文だけを解析して覚えておく

use std::cell::{OnceCell, RefCell};
use std::fmt;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;
#[cfg(feature = "bundled-dict")]
use std::sync::OnceLock;

use hasami::{Analyzer, CoarsePos, DictError, Dictionary};
use serde::{Deserialize, Serialize};

use crate::dictionaries;
use crate::document::Document;

/// 辞書の使い方 (`[morphology] mode`)。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MorphologyMode {
    /// 辞書が見つかれば使い、なければ辞書なしの近似で判定する (既定)。
    #[default]
    Auto,
    /// 辞書を使う。見つからなければ設定の誤りにする。
    Required,
    /// 辞書を使わない。
    Off,
}

impl MorphologyMode {
    pub fn as_str(self) -> &'static str {
        match self {
            MorphologyMode::Auto => "auto",
            MorphologyMode::Required => "required",
            MorphologyMode::Off => "off",
        }
    }
}

/// 辞書の設定 (設定ファイルと CLI を合わせたもの)。
#[derive(Debug, Clone, Default)]
pub struct MorphologyOptions {
    pub mode: MorphologyMode,
    /// 使う辞書 (`[morphology] dictionary`・`--dict`)。既定は `auto`。
    pub dictionary: DictionaryChoice,
    /// 手元の辞書を探す場所 (`HASAMI_DICT` と share ディレクトリ)。CLI は
    /// [`DictionarySearch::from_env`] で埋める。既定 (空) では手元を探さないので、テストの結果が
    /// 手元に入れた辞書に左右されない。
    pub search: DictionarySearch,
}

/// 辞書の指定のキーワード: 自動で選ぶ (既定)。
pub const AUTO_DICTIONARY: &str = "auto";
/// 辞書の指定のキーワード: 同梱の辞書。
pub const BUNDLED_DICTIONARY: &str = "bundled";

/// 使う辞書の指定 (`[morphology] dictionary`・`--dict`)。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum DictionaryChoice {
    /// `auto`: `HASAMI_DICT` → share ディレクトリの配布辞書 (hasami の推奨順) → 同梱の辞書 ([`resolve`])。
    #[default]
    Auto,
    /// `bundled`: 同梱の辞書 (`dict/ipadic.hsd`)。
    Bundled,
    /// `share:<名前>`: share ディレクトリの `<名前>.hsd` ([`dictionaries::resolve_share`])。
    Share(String),
    /// 辞書のファイル。
    File(PathBuf),
}

impl DictionaryChoice {
    /// 指定を読む。`auto`・`bundled` (どちらも完全一致) と `share:<名前>` のほかは、ファイルのパスとして
    /// そのまま使う (`auto` という名前のファイルは `./auto` と書く)。
    pub fn parse(spec: &Path) -> Self {
        match spec.to_str() {
            Some(AUTO_DICTIONARY) => Self::Auto,
            Some(BUNDLED_DICTIONARY) => Self::Bundled,
            _ => match dictionaries::share_name(spec) {
                Some(name) => Self::Share(name.to_string()),
                None => Self::File(spec.to_path_buf()),
            },
        }
    }
}

/// 手元の辞書を探す場所。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DictionarySearch {
    /// 環境変数 `HASAMI_DICT` (辞書のファイル)。
    pub hasami_dict: Option<PathBuf>,
    /// hasami の share ディレクトリ ([`dictionaries::share_dir`])。
    pub share_dir: Option<PathBuf>,
}

impl DictionarySearch {
    /// 実行中の環境から作る (空の `HASAMI_DICT` は設定されていないとみなす)。
    pub fn from_env() -> Self {
        Self {
            hasami_dict: std::env::var_os(hasami::analyzer::DICT_ENV)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from),
            share_dir: dictionaries::share_dir(),
        }
    }
}

/// 判定の方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Method {
    /// 辞書の品詞で判定した。
    Dictionary,
    /// 辞書を使わず、文字種と表層の近似で判定した。
    Surface,
}

/// 辞書を使わなかった理由。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum FallbackReason {
    /// 設定で使わないことにした (`mode = "off"`・`--no-dict`)。
    Disabled,
    /// 辞書が見つからなかった (`mode = "auto"`)。
    NotFound,
    /// 辞書を使う有効なルールがなかった。
    NotNeeded,
}

/// 辞書の出所。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DictionarySource {
    /// noslop に同梱した辞書 (`dict/ipadic.hsd`)。
    Bundled,
    /// 指定されたファイル (`--dict`・設定の `dictionary`・`HASAMI_DICT`・hasami の既定の場所)。
    File,
}

/// 使った辞書。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DictionaryInfo {
    /// 辞書の名前 (辞書に記録された `name`。例: `ipadic`)。
    pub name: String,
    /// 辞書の出所。
    pub source: DictionarySource,
    /// 辞書のファイル (同梱の辞書なら `null`)。
    pub path: Option<PathBuf>,
}

/// この実行で使った判定の方式 (出力に載せる)。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MorphologyStatus {
    /// 設定で求めた使い方。
    pub requested: MorphologyMode,
    /// 実際に使った方式。
    pub method: Method,
    /// 使った辞書 (辞書なしなら `null`)。
    pub dictionary: Option<DictionaryInfo>,
    /// 辞書を使わなかった理由 (辞書を使ったなら `null`)。
    pub reason: Option<FallbackReason>,
}

impl MorphologyStatus {
    fn surface(requested: MorphologyMode, reason: FallbackReason) -> Self {
        Self {
            requested,
            method: Method::Surface,
            dictionary: None,
            reason: Some(reason),
        }
    }

    /// 人に見せる 1 行 (辞書を使うルールが動かなかったなら `None`)。
    pub fn describe(&self) -> Option<String> {
        match (&self.dictionary, self.reason) {
            (Some(dict), _) if dict.source == DictionarySource::Bundled => {
                Some(format!("辞書あり (同梱の {})", dict.name))
            }
            (Some(dict), _) => Some(format!("辞書あり ({})", dict.name)),
            (None, Some(FallbackReason::NotFound)) => {
                Some("辞書なし (辞書が見つからないため、近似で判定)".into())
            }
            (None, Some(FallbackReason::Disabled)) => Some("辞書なし (設定で無効)".into()),
            _ => None,
        }
    }
}

impl Default for MorphologyStatus {
    fn default() -> Self {
        Self::surface(MorphologyMode::Auto, FallbackReason::NotNeeded)
    }
}

/// 読み込んだ辞書。実行ごとに 1 つ読み、文書どうしで共有する。
#[derive(Clone)]
pub struct Morphology {
    dictionary: Arc<Dictionary>,
    info: DictionaryInfo,
}

impl fmt::Debug for Morphology {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Morphology")
            .field("info", &self.info)
            .finish()
    }
}

impl Morphology {
    /// 辞書のファイルを読み込む。
    pub fn load(path: &Path) -> Result<Self, DictError> {
        let dictionary = Dictionary::load(path)?;
        let info = DictionaryInfo {
            name: dictionary.meta().name().to_string(),
            source: DictionarySource::File,
            path: Some(path.to_path_buf()),
        };
        Ok(Self::from_dictionary(dictionary, info))
    }

    /// 同梱の辞書 (`dict/ipadic.hsd`)。バイナリに埋め込んだバイト列を複製せずに参照する
    /// ([`Dictionary::from_static`])。それでも組み立てるたびに辞書の検査と、品詞・活用の文字列表や
    /// 文字種の表の確保をするので、組み立てはプロセスで 1 度だけにし、以後は同じ辞書を共有する
    /// (MCP サーバーの呼び出しをまたいでも)。
    #[cfg(feature = "bundled-dict")]
    pub fn bundled() -> Result<Self, String> {
        static BUNDLED: OnceLock<Result<Morphology, String>> = OnceLock::new();
        BUNDLED
            .get_or_init(|| {
                let dictionary = Dictionary::from_static(BUNDLED_HSD)
                    .map_err(|e| format!("同梱の形態素解析の辞書を読めません: {e}"))?;
                let info = DictionaryInfo {
                    name: dictionary.meta().name().to_string(),
                    source: DictionarySource::Bundled,
                    path: None,
                };
                Ok(Self::from_dictionary(dictionary, info))
            })
            .clone()
    }

    /// 組み立て済みの辞書から作る。
    pub fn from_dictionary(dictionary: Dictionary, info: DictionaryInfo) -> Self {
        Self {
            dictionary: Arc::new(dictionary),
            info,
        }
    }

    pub fn info(&self) -> &DictionaryInfo {
        &self.info
    }

    /// 文書 1 つの形態素の層を作る。
    pub fn for_document<'a>(&self, doc: &'a Document) -> DocMorphology<'a> {
        DocMorphology {
            doc,
            analyzer: RefCell::new(Analyzer::from_shared(Arc::clone(&self.dictionary))),
            sentences: (0..doc.sentences.len()).map(|_| OnceCell::new()).collect(),
        }
    }
}

/// 同梱の辞書の中身 (hasami の `.hsd`。出所と更新の手順は `dict/README.md`)。
///
/// `Dictionary::from_static` は先頭が 8 バイト境界にあることを求める。`include_bytes!` だけでは
/// 境界がそろわない (そろうかどうかがビルドごとに変わる) ので、`hasami::include_hsd!` で 64 バイト
/// 境界にそろえて埋め込む。パスは `include_bytes!` と同じく、このファイルからの相対パス。マクロは
/// 呼ぶたびに別の静的領域になる (同じ辞書がバイナリに 2 つ入る) ので、埋め込むのはここだけにする。
#[cfg(feature = "bundled-dict")]
static BUNDLED_HSD: &[u8] = hasami::include_hsd!("../dict/ipadic.hsd");

/// 設定から辞書を決めて読み込み、使う方式を返す。
///
/// `needed` は辞書を使う有効なルールがあるか。なければ辞書を探さない。
///
/// 辞書の指定 ([`DictionaryChoice`]) ごとに、次の辞書を使う。
///
/// - `auto` (既定): `HASAMI_DICT` → share ディレクトリ (既定は `~/.local/share/hasami`) の配布辞書の
///   うち、hasami の推奨順で最初に見つかったもの ([`dictionaries::preferred_in`]) → 同梱の辞書。
///   同梱しないビルドでは、同梱の辞書の代わりに share ディレクトリのほかの `*.hsd` を名前順に探す
///   (hasami の既定の探索と同じ)
/// - `bundled`: 同梱の辞書 (同梱しないビルドでは `Err`)
/// - `share:<名前>`: share ディレクトリの `<名前>.hsd` (`noslop dict download` で取得した辞書。
///   [`dictionaries::resolve_share`])。`HASAMI_DICT` には当てない (hasami と同じく、ファイルのパスとして読む)
/// - ファイルのパス: その辞書
///
/// 見つけた辞書が読めないときは `Err` にし、下の候補や同梱の辞書、近似に黙って切り替えない (手元の
/// 辞書が壊れたまま、結果だけが変わるのを避ける)。`auto` で辞書が 1 つも見つからないのは同梱しない
/// ビルドだけで、そのときは辞書なしの近似に戻る (`required` なら `Err`)。
pub fn resolve(
    options: &MorphologyOptions,
    needed: bool,
) -> Result<(Option<Morphology>, MorphologyStatus), String> {
    let requested = options.mode;
    if requested == MorphologyMode::Off {
        return Ok((
            None,
            MorphologyStatus::surface(requested, FallbackReason::Disabled),
        ));
    }
    if !needed {
        return Ok((
            None,
            MorphologyStatus::surface(requested, FallbackReason::NotNeeded),
        ));
    }
    let search = &options.search;
    let morphology = match &options.dictionary {
        DictionaryChoice::File(path) => load_file(path)?,
        DictionaryChoice::Share(name) => {
            let path = dictionaries::resolve_share(search.share_dir.as_deref(), name)
                .map_err(|e| e.to_string())?;
            load_file(&path)?
        }
        DictionaryChoice::Bundled => bundled()?,
        DictionaryChoice::Auto => match auto(search, requested)? {
            Some(morphology) => morphology,
            None => {
                return Ok((
                    None,
                    MorphologyStatus::surface(requested, FallbackReason::NotFound),
                ));
            }
        },
    };
    let status = MorphologyStatus {
        requested,
        method: Method::Dictionary,
        dictionary: Some(morphology.info().clone()),
        reason: None,
    };
    Ok((Some(morphology), status))
}

/// `auto` が選ぶ辞書 (読み込む前)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AutoPick {
    /// `HASAMI_DICT` の辞書。
    HasamiDict(PathBuf),
    /// share ディレクトリの辞書。
    Share(PathBuf),
    /// 同梱の辞書。
    Bundled,
    /// 辞書がない (同梱しないビルドだけ)。
    Nothing,
}

/// `auto` が選ぶ辞書: `HASAMI_DICT` → share ディレクトリの配布辞書 (hasami の推奨順) → 同梱の辞書。
/// 同梱しないビルドでは、同梱の辞書の代わりに share ディレクトリのほかの `*.hsd` を名前順に探す
/// (hasami の既定の探索と同じ)。ファイルがあるかだけを見て、読めるかは確かめない。
pub fn auto_pick(search: &DictionarySearch) -> AutoPick {
    if let Some(path) = &search.hasami_dict {
        return AutoPick::HasamiDict(path.clone());
    }
    if let Some(path) = search
        .share_dir
        .as_deref()
        .and_then(dictionaries::preferred_in)
    {
        return AutoPick::Share(path);
    }
    fallback_pick(search)
}

#[cfg(feature = "bundled-dict")]
fn fallback_pick(_search: &DictionarySearch) -> AutoPick {
    AutoPick::Bundled
}

#[cfg(not(feature = "bundled-dict"))]
fn fallback_pick(search: &DictionarySearch) -> AutoPick {
    match search
        .share_dir
        .as_deref()
        .and_then(hasami::analyzer::preferred_dict_in)
    {
        Some(path) => AutoPick::Share(path),
        None => AutoPick::Nothing,
    }
}

/// `auto` の辞書を読む。辞書が見つからなければ `None` (同梱しないビルドの `auto` だけ)。
fn auto(
    search: &DictionarySearch,
    requested: MorphologyMode,
) -> Result<Option<Morphology>, String> {
    match auto_pick(search) {
        AutoPick::HasamiDict(path) => load_file(&path).map(Some),
        AutoPick::Share(path) => load_chosen(&path).map(Some),
        AutoPick::Bundled => bundled().map(Some),
        AutoPick::Nothing if requested == MorphologyMode::Auto => Ok(None),
        AutoPick::Nothing => {
            let share = match &search.share_dir {
                Some(dir) => dir.join("*.hsd").display().to_string(),
                None => "share ディレクトリ (HASAMI_DATA_DIR・XDG_DATA_HOME・HOME のどれも設定されていません)"
                    .to_string(),
            };
            Err(format!(
                "形態素解析の辞書が見つかりません (探した場所: {} (未設定)、{share})",
                hasami::analyzer::DICT_ENV
            ))
        }
    }
}

fn load_file(path: &Path) -> Result<Morphology, String> {
    Morphology::load(path)
        .map_err(|e| format!("形態素解析の辞書を読めません: {}: {e}", path.display()))
}

/// share ディレクトリから自動で選んだ辞書を読む。読めなければ、取り直すか辞書を指定するよう案内する。
fn load_chosen(path: &Path) -> Result<Morphology, String> {
    load_file(path).map_err(|e| {
        let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        let download = match dictionaries::find(name) {
            Some(dict) => format!("`noslop dict download {} --force` で取り直すか、", dict.name),
            None => String::new(),
        };
        format!(
            "{e} (share ディレクトリから自動で選んだ辞書です。{download}[morphology] の dictionary か --dict で使う辞書を指定してください)"
        )
    })
}

/// `bundled` の辞書。
#[cfg(feature = "bundled-dict")]
fn bundled() -> Result<Morphology, String> {
    Morphology::bundled()
}

/// `bundled` の辞書 (同梱しないビルドにはない)。
#[cfg(not(feature = "bundled-dict"))]
fn bundled() -> Result<Morphology, String> {
    Err(format!(
        "このビルドは形態素解析の辞書を同梱していません (dictionary = \"{BUNDLED_DICTIONARY}\")。辞書のファイルか share:<名前> を指定してください"
    ))
}

/// 形態素 1 つ。
#[derive(Debug, Clone)]
pub struct MorphToken {
    /// 文の本文 ([`Document::sentence_text`]) の中のバイト範囲。
    pub range: Range<usize>,
    /// 粗い品詞。
    pub pos: CoarsePos,
    /// 辞書の品詞 (IPAdic なら「名詞,固有名詞,人名,名」のようなカンマ区切り)。
    pub detail: Arc<str>,
}

/// 文書 1 つの形態素。ルールが求めた文だけを解析して覚えておく。
///
/// 解析器は `&mut` で使うので、文書ごとに作り、文書の中では 1 本のスレッドで使う。
pub struct DocMorphology<'a> {
    doc: &'a Document,
    analyzer: RefCell<Analyzer>,
    sentences: Vec<OnceCell<Option<Vec<MorphToken>>>>,
}

impl DocMorphology<'_> {
    /// 文 (`Document::sentences` の添字) の形態素。解析できなかったら `None`。
    pub fn sentence(&self, index: usize) -> Option<&[MorphToken]> {
        self.sentences
            .get(index)?
            .get_or_init(|| {
                let text = self.doc.sentence_text(&self.doc.sentences[index]);
                let tokens = self.analyzer.borrow_mut().try_tokenize(text).ok()?;
                Some(
                    tokens
                        .iter()
                        .map(|t| MorphToken {
                            range: t.start..t.end,
                            pos: t.coarse_pos(),
                            detail: Arc::clone(&t.pos),
                        })
                        .collect(),
                )
            })
            .as_deref()
    }
}

/// テスト用の小さな辞書。
#[cfg(test)]
pub(crate) mod testing {
    use hasami::DictEntry;
    use hasami::dict::DictBuilder;

    use super::*;

    /// 語と品詞 (IPAdic の 4 階層) の組から辞書を組み立てる。
    ///
    /// 連接のコストはすべて 0 なので、語のコストが低い並びが選ばれる。長い語ほどコストを
    /// 低くしてあるので、辞書にある最も長い語で区切られる。
    pub(crate) fn morphology(words: &[(&str, &str)]) -> Morphology {
        let dictionary = builder(words)
            .build()
            .expect("テスト用の辞書を組み立てられない");
        Morphology::from_dictionary(
            dictionary,
            DictionaryInfo {
                name: "test".into(),
                source: DictionarySource::File,
                path: Some(PathBuf::from("test.hsd")),
            },
        )
    }

    /// [`morphology`] と同じ辞書を、ファイル (`.hsd`) に書く。
    pub(crate) fn write_dictionary(path: &Path, words: &[(&str, &str)]) {
        let builder = builder(words);
        builder
            .write_hsd(path, &builder.write_options(), |_, _| {})
            .expect("テスト用の辞書を書けない");
    }

    fn builder(words: &[(&str, &str)]) -> DictBuilder {
        let mut builder = DictBuilder::new();
        for (surface, pos) in words {
            let cost = 1000 - 10 * surface.chars().count() as i16;
            builder.add_entry(DictEntry {
                surface: (*surface).into(),
                left_id: 1,
                right_id: 1,
                cost,
                pos: (*pos).into(),
                base_form: (*surface).into(),
                ..Default::default()
            });
        }
        builder
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::document::{ParseOptions, SourceFormat};

    const WORDS: &[(&str, &str)] = &[
        ("猫", "名詞,一般,*,*"),
        ("の", "助詞,連体化,*,*"),
        ("家", "名詞,一般,*,*"),
        ("です", "助動詞,*,*,*"),
        ("。", "記号,句点,*,*"),
    ];

    fn doc(text: &str) -> Document {
        Document::parse(
            "t.md",
            text,
            SourceFormat::Markdown,
            &ParseOptions::default(),
        )
    }

    #[test]
    fn sentences_are_tokenized_with_ranges_in_the_sentence() {
        let m = testing::morphology(WORDS);
        let d = doc("猫の家です。猫です。");
        let dm = m.for_document(&d);
        let tokens = dm.sentence(1).unwrap();
        let text = d.sentence_text(&d.sentences[1]);
        let surfaces: Vec<&str> = tokens.iter().map(|t| &text[t.range.clone()]).collect();
        assert_eq!(surfaces, ["猫", "です", "。"]);
        assert_eq!(tokens[0].pos, CoarsePos::Noun);
        let first = dm.sentence(0).unwrap();
        assert_eq!(first[1].pos, CoarsePos::CaseParticle);
        assert_eq!(&*first[1].detail, "助詞,連体化,*,*");
        assert!(dm.sentence(99).is_none());
    }

    fn options(mode: MorphologyMode, dictionary: DictionaryChoice) -> MorphologyOptions {
        MorphologyOptions {
            mode,
            dictionary,
            ..Default::default()
        }
    }

    fn file(path: &str) -> DictionaryChoice {
        DictionaryChoice::File(PathBuf::from(path))
    }

    #[test]
    fn dictionary_specs_are_keywords_share_names_or_files() {
        let parse = |s: &str| DictionaryChoice::parse(Path::new(s));
        assert_eq!(parse("auto"), DictionaryChoice::Auto);
        assert_eq!(parse("bundled"), DictionaryChoice::Bundled);
        assert_eq!(
            parse("share:ipadic"),
            DictionaryChoice::Share("ipadic".into())
        );
        // キーワードは完全一致だけ。それ以外はファイルのパス
        for path in ["./auto", "Auto", "bundled.hsd", "dict/share:x.hsd"] {
            assert_eq!(parse(path), file(path), "{path}");
        }
        assert_eq!(
            MorphologyOptions::default().dictionary,
            DictionaryChoice::Auto
        );
    }

    #[test]
    fn off_and_not_needed_do_not_look_for_a_dictionary() {
        let off = options(MorphologyMode::Off, file("missing.hsd"));
        let (m, status) = resolve(&off, true).unwrap();
        assert!(m.is_none());
        assert_eq!(status.reason, Some(FallbackReason::Disabled));

        let required = options(MorphologyMode::Required, file("missing.hsd"));
        let (m, status) = resolve(&required, false).unwrap();
        assert!(m.is_none());
        assert_eq!(status.reason, Some(FallbackReason::NotNeeded));
        assert_eq!(status.describe(), None);
    }

    #[test]
    fn an_explicit_dictionary_that_cannot_be_read_is_an_error() {
        for mode in [MorphologyMode::Auto, MorphologyMode::Required] {
            let err = resolve(&options(mode, file("missing.hsd")), true).unwrap_err();
            assert!(err.contains("missing.hsd"), "{err}");
        }
    }

    /// `share:<名前>` は share ディレクトリの中だけを指す (名前は share ディレクトリを見る前に確かめる)。
    #[test]
    fn share_specs_must_name_a_file_in_the_share_directory() {
        for spec in ["share:../ipadic", "share:a/b", "share:"] {
            let o = options(
                MorphologyMode::Required,
                DictionaryChoice::parse(Path::new(spec)),
            );
            let err = resolve(&o, true).unwrap_err();
            assert!(err.starts_with("share:"), "{spec}: {err}");
        }
    }

    /// share ディレクトリに配布辞書の名前で置く。中身はどれも [`WORDS`] の辞書。
    fn share_with(names: &[&str]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for name in names {
            testing::write_dictionary(&dir.path().join(format!("{name}.hsd")), WORDS);
        }
        dir
    }

    fn chosen_path(o: &MorphologyOptions) -> Option<PathBuf> {
        let (m, status) = resolve(o, true).unwrap();
        assert_eq!(m.is_some(), status.method == Method::Dictionary);
        status.dictionary.and_then(|d| d.path)
    }

    /// auto は share ディレクトリの配布辞書を hasami の推奨順で選ぶ。HASAMI_DICT があればそれが先。
    #[test]
    fn auto_prefers_the_best_distributed_dictionary_in_the_share_directory() {
        let share = share_with(&["ipadic", "ipadic-neologd-sudachi", "zzz-custom"]);
        let mut o = options(MorphologyMode::Auto, DictionaryChoice::Auto);
        o.search.share_dir = Some(share.path().to_path_buf());
        assert_eq!(
            chosen_path(&o),
            Some(share.path().join("ipadic-neologd-sudachi.hsd"))
        );

        // 推奨の辞書がなければ、次に推奨する辞書
        fs::remove_file(share.path().join("ipadic-neologd-sudachi.hsd")).unwrap();
        assert_eq!(chosen_path(&o), Some(share.path().join("ipadic.hsd")));

        // HASAMI_DICT は share ディレクトリより先
        let other = tempfile::tempdir().unwrap();
        let env_dict = other.path().join("env.hsd");
        testing::write_dictionary(&env_dict, WORDS);
        o.search.hasami_dict = Some(env_dict.clone());
        assert_eq!(chosen_path(&o), Some(env_dict));

        // 明示の指定は auto の探索より先。bundled は share ディレクトリに辞書があっても同梱の辞書
        o.search.hasami_dict = None;
        o.dictionary = DictionaryChoice::Share("ipadic".into());
        assert_eq!(chosen_path(&o), Some(share.path().join("ipadic.hsd")));
        o.dictionary = DictionaryChoice::Bundled;
        let bundled = resolve(&o, true);
        if cfg!(feature = "bundled-dict") {
            let (_, status) = bundled.unwrap();
            assert_eq!(status.dictionary.unwrap().source, DictionarySource::Bundled);
        } else {
            assert!(bundled.unwrap_err().contains("同梱していません"));
        }
    }

    /// share ディレクトリに配布辞書がなければ、同梱の辞書 (同梱しないビルドでは、ほかの `*.hsd` を
    /// 名前順に探し、なければ辞書なしの近似)。既定の探索場所は空なので、手元の辞書は探さない。
    #[test]
    fn auto_without_distributed_dictionaries_uses_the_bundled_one() {
        let share = share_with(&["zzz-custom"]);
        let mut o = options(MorphologyMode::Auto, DictionaryChoice::Auto);
        for share_dir in [None, Some(share.path().to_path_buf())] {
            o.search.share_dir = share_dir.clone();
            let (_, status) = resolve(&o, true).unwrap();
            match (cfg!(feature = "bundled-dict"), &share_dir) {
                (true, _) => {
                    assert_eq!(status.dictionary.unwrap().source, DictionarySource::Bundled)
                }
                (false, Some(dir)) => assert_eq!(
                    status.dictionary.unwrap().path,
                    Some(dir.join("zzz-custom.hsd"))
                ),
                (false, None) => assert_eq!(status.reason, Some(FallbackReason::NotFound)),
            }
        }
    }

    /// auto で選んだ辞書が読めなければ、下の候補や同梱の辞書に切り替えずにエラーにし、取り直しを案内する。
    #[test]
    fn a_broken_dictionary_chosen_by_auto_is_an_error() {
        let share = share_with(&["ipadic"]);
        let broken = share.path().join("ipadic-neologd-sudachi.hsd");
        fs::write(&broken, b"not a dictionary").unwrap();
        let mut o = options(MorphologyMode::Auto, DictionaryChoice::Auto);
        o.search.share_dir = Some(share.path().to_path_buf());
        let err = resolve(&o, true).unwrap_err();
        assert!(err.contains(&broken.display().to_string()), "{err}");
        assert!(
            err.contains("noslop dict download ipadic-neologd-sudachi --force"),
            "{err}"
        );
    }

    /// 同梱の辞書は hasami の IPAdic (出所と更新の手順は dict/README.md)。辞書を差し替えたら、
    /// 手元の文書で P15・P16 の指摘の差分を確かめてから、ここに固定した値を書き換える。
    #[cfg(feature = "bundled-dict")]
    #[test]
    fn the_bundled_dictionary_is_ipadic_from_hasami() {
        assert_eq!(BUNDLED_HSD.len(), 18_125_804);
        let m = Morphology::bundled().unwrap();
        assert_eq!(m.info().name, "ipadic");
        assert_eq!(m.info().source, DictionarySource::Bundled);
        assert_eq!(m.info().path, None);
        assert_eq!(
            m.dictionary.meta().get("sources"),
            Some("ipadic@61b90ba6e669")
        );
        assert_eq!(m.dictionary.entry_count(), 390_849);
        // 2 度目からは同じ辞書を共有する
        let again = Morphology::bundled().unwrap();
        assert!(Arc::ptr_eq(&m.dictionary, &again.dictionary));
    }

    #[test]
    fn status_describes_the_method() {
        let m = testing::morphology(WORDS);
        let status = MorphologyStatus {
            requested: MorphologyMode::Auto,
            method: Method::Dictionary,
            dictionary: Some(m.info().clone()),
            reason: None,
        };
        assert_eq!(status.describe().as_deref(), Some("辞書あり (test)"));
        let not_found = MorphologyStatus::surface(MorphologyMode::Auto, FallbackReason::NotFound);
        assert!(not_found.describe().unwrap().starts_with("辞書なし"));
        let json = serde_json::to_value(&not_found).unwrap();
        assert_eq!(json["method"], "surface");
        assert_eq!(json["reason"], "not-found");
        assert_eq!(json["dictionary"], serde_json::Value::Null);
    }
}
