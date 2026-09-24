//! 形態素解析 (辞書を使う精密判定)。
//!
//! 形態素解析器 [hasami](https://github.com/owayo/hasami) の辞書 (`.hsd`) で、品詞で数えるルール
//! (P15・P16) を判定する。既定のビルド (feature `bundled-dict`) は IPAdic の辞書
//! (`dict/ipadic.hsd`) をバイナリに埋め込んでいるので、何も指定しなくても辞書で判定する。
//! 辞書を使わないとき (`--no-dict`・同梱しないビルドで辞書が見つからないとき) は、それらのルールは
//! 辞書なしの近似で判定する。
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
    /// 辞書のパスか、`share:<名前>` (hasami の share ディレクトリの `<名前>.hsd`)。なければ
    /// `HASAMI_DICT`、次に同梱の辞書を使う (同梱しないビルドでは hasami の既定の場所
    /// `~/.local/share/hasami` を探す)。
    pub dictionary: Option<PathBuf>,
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
/// 探す順は、明示のパス (`--dict`・設定の `dictionary`) → `HASAMI_DICT` → 同梱の辞書。
/// `~/.local/share/hasami` は探さない (同じ版の noslop なら、手元に入れた辞書によらず同じ結果に
/// するため)。同梱しないビルドでは、同梱の辞書の代わりに hasami の既定の場所を探す。
///
/// 明示の指定が `share:<名前>` なら、hasami の share ディレクトリの `<名前>.hsd` を使う
/// (`noslop dict download` で取得した辞書。[`dictionaries::resolve_share`])。`HASAMI_DICT` には
/// 当てない (hasami と同じく、ファイルのパスとして読む)。
///
/// 指定した辞書が読めないときは `Err` にし、同梱の辞書や近似に黙って切り替えない。
/// 同梱しないビルドで、`auto` なのに辞書が見つからないときだけ、辞書なしの近似に戻る
/// (`required` なら `Err`)。
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
    let explicit = match &options.dictionary {
        Some(spec) => Some(explicit_dictionary(spec)?),
        None => std::env::var_os(hasami::analyzer::DICT_ENV)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from),
    };
    let morphology = match explicit {
        Some(path) => load_file(&path)?,
        None => match fallback(requested)? {
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

/// 明示の指定 (`--dict`・設定の `dictionary`) を辞書のファイルに直す。`share:<名前>` なら share
/// ディレクトリの辞書 (なければエラー)、それ以外はファイルのパスとしてそのまま使う。
fn explicit_dictionary(spec: &Path) -> Result<PathBuf, String> {
    match dictionaries::share_name(spec) {
        Some(name) => dictionaries::resolve_share(name).map_err(|e| e.to_string()),
        None => Ok(spec.to_path_buf()),
    }
}

fn load_file(path: &Path) -> Result<Morphology, String> {
    Morphology::load(path)
        .map_err(|e| format!("形態素解析の辞書を読めません: {}: {e}", path.display()))
}

/// 明示の指定がないときの辞書 (同梱の辞書)。
#[cfg(feature = "bundled-dict")]
fn fallback(_requested: MorphologyMode) -> Result<Option<Morphology>, String> {
    Morphology::bundled().map(Some)
}

/// 明示の指定がないときの辞書 (同梱しないビルドでは hasami の既定の場所)。
#[cfg(not(feature = "bundled-dict"))]
fn fallback(requested: MorphologyMode) -> Result<Option<Morphology>, String> {
    match hasami::analyzer::default_dict_path() {
        Ok(path) => load_file(&path).map(Some),
        Err(DictError::NotFound(_)) if requested == MorphologyMode::Auto => Ok(None),
        Err(DictError::NotFound(searched)) => Err(format!(
            "形態素解析の辞書が見つかりません (探した場所: {})",
            searched.join("、")
        )),
        Err(e) => Err(format!("形態素解析の辞書を探せません: {e}")),
    }
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
        let dictionary = builder.build().expect("テスト用の辞書を組み立てられない");
        Morphology::from_dictionary(
            dictionary,
            DictionaryInfo {
                name: "test".into(),
                source: DictionarySource::File,
                path: Some(PathBuf::from("test.hsd")),
            },
        )
    }
}

#[cfg(test)]
mod tests {
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

    #[test]
    fn off_and_not_needed_do_not_look_for_a_dictionary() {
        let off = MorphologyOptions {
            mode: MorphologyMode::Off,
            dictionary: Some(PathBuf::from("missing.hsd")),
        };
        let (m, status) = resolve(&off, true).unwrap();
        assert!(m.is_none());
        assert_eq!(status.reason, Some(FallbackReason::Disabled));

        let required = MorphologyOptions {
            mode: MorphologyMode::Required,
            dictionary: Some(PathBuf::from("missing.hsd")),
        };
        let (m, status) = resolve(&required, false).unwrap();
        assert!(m.is_none());
        assert_eq!(status.reason, Some(FallbackReason::NotNeeded));
        assert_eq!(status.describe(), None);
    }

    #[test]
    fn an_explicit_dictionary_that_cannot_be_read_is_an_error() {
        for mode in [MorphologyMode::Auto, MorphologyMode::Required] {
            let options = MorphologyOptions {
                mode,
                dictionary: Some(PathBuf::from("missing.hsd")),
            };
            let err = resolve(&options, true).unwrap_err();
            assert!(err.contains("missing.hsd"), "{err}");
        }
    }

    /// `share:<名前>` は share ディレクトリの中だけを指す (名前は環境変数を見る前に確かめる)。
    #[test]
    fn share_specs_must_name_a_file_in_the_share_directory() {
        for spec in ["share:../ipadic", "share:a/b", "share:"] {
            let options = MorphologyOptions {
                mode: MorphologyMode::Required,
                dictionary: Some(PathBuf::from(spec)),
            };
            let err = resolve(&options, true).unwrap_err();
            assert!(err.starts_with("share:"), "{spec}: {err}");
        }
        // share: で始まらなければファイルのパス
        assert_eq!(
            explicit_dictionary(Path::new("dict/share:x.hsd")).unwrap(),
            PathBuf::from("dict/share:x.hsd")
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
