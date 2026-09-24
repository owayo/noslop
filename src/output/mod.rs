//! 結果の出力 (text / json / toon / github / brief)。

pub mod brief;
pub mod github;
pub mod json;
pub mod text;
pub mod toon;

use std::collections::BTreeMap;

use crate::config::FailOn;
use crate::document::Document;
use crate::engine::Engine;
use crate::genre::Genre;
use crate::rules::RuleMeta;

/// 出力の設定。
#[derive(Debug, Clone, Default)]
pub struct RenderOptions {
    /// 抑制した指摘も text に出すか。
    pub show_suppressed: bool,
    /// 指摘のないファイルとサマリを出さないか。
    pub quiet: bool,
    /// JSON の `settings` に載せる値。
    pub genre: Genre,
    pub experimental: bool,
    pub fail_on: FailOn,
    /// brief で 1 ルールあたりに並べる箇所の上限 (`None` なら既定の 5)。
    pub brief_limit: Option<usize>,
    /// brief を短くする (フックから AI に返すとき)。
    pub brief_compact: bool,
    /// brief がルールの日本語名・説明を引くための目録。
    pub catalog: RuleCatalog,
}

/// ルール ID からメタ情報を引く目録 (組み込みルールと独自ルール)。
#[derive(Debug, Clone, Default)]
pub struct RuleCatalog {
    metas: BTreeMap<&'static str, &'static RuleMeta>,
}

impl RuleCatalog {
    /// エンジンが持つすべてのルール (有効・無効を問わず) から作る。
    pub fn from_engine(engine: &Engine) -> Self {
        let metas = engine
            .entries()
            .iter()
            .map(|e| {
                let m = e.rule.meta();
                (m.id, m)
            })
            .collect();
        Self { metas }
    }

    pub fn get(&self, id: &str) -> Option<&'static RuleMeta> {
        self.metas.get(id).copied()
    }
}

/// 行・列・オフセット。行と列は 1 始まりで、列は Unicode スカラー値の個数。
/// オフセットはファイル上の UTF-8 バイト位置 (先頭の BOM を含めて数える)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct Position {
    pub line: usize,
    pub column: usize,
    pub offset: usize,
}

impl Position {
    /// [`Document::source`] 上のバイト位置から作る。
    pub fn of(doc: &Document, offset: usize) -> Self {
        let (line, column) = doc.line_col(offset);
        Self {
            line,
            column,
            offset: doc.file_offset(offset),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offsets_count_the_bom_but_columns_do_not() {
        let src = "\u{FEFF}これはと言えるでしょう。";
        let doc = Document::markdown(src);
        let at = doc.source.find("と言える").unwrap();
        let pos = Position::of(&doc, at);
        assert_eq!(pos.offset, src.find("と言える").unwrap());
        assert_eq!((pos.line, pos.column), (1, 4));
    }
}
