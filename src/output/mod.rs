//! 結果の出力 (text / json / toon / github / brief)。

pub mod brief;
pub mod github;
pub mod json;
pub mod text;
pub mod toon;

use std::collections::BTreeMap;
use std::io::{self, Write};

use crate::config::FailOn;
use crate::document::Document;
use crate::engine::{Engine, RunReport};
use crate::genre::Genre;
use crate::rules::RuleMeta;

/// 検査結果の内容と形式。引数の既定値や組み合わせの検証は CLI・MCP それぞれで行う。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CheckOutput {
    Text,
    Json,
    Toon,
    Github,
    BriefMarkdown,
    BriefJson,
    BriefToon,
}

impl CheckOutput {
    /// 選んだ形式で書き出す。端末の色・バッファリング・標準エラーへの出力は呼び出し元が扱う。
    pub(crate) fn render(
        self,
        report: &RunReport,
        opts: &RenderOptions,
        out: &mut dyn Write,
    ) -> io::Result<()> {
        match self {
            Self::Text => text::render(report, opts, out),
            Self::Json => json::render(report, opts, out),
            Self::Toon => json::render_toon(report, opts, out),
            Self::Github => github::render(report, out),
            Self::BriefMarkdown => brief::render(report, opts, out),
            Self::BriefJson => brief::render_json(report, opts, out),
            Self::BriefToon => brief::render_toon(report, opts, out),
        }
    }
}

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
