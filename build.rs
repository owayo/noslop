//! 配布辞書の目録 (`dict/catalog.json`) を検証し、`src/dictionaries.rs` に取り込む定数を作る。
//!
//! 目録は、hasami のリリースに添付された `dictionaries.json` をそのまま置いたもの。Release の
//! ワークフロー (`make dict-catalog`) が hasami の新しいリリースに合わせて差し替える。壊れた目録で
//! ビルドが通らないよう、ここで形を確かめ、合わなければビルドを止める。目録の辞書の形式が依存の
//! hasami で読めるか (`format_version`) は、ビルド用の依存から hasami を参照できないので、
//! `src/dictionaries.rs` のテストで確かめる。

use std::collections::HashSet;
use std::env;
use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;

use serde::Deserialize;

/// 目録。noslop が使う項目だけを読む (ほかの項目は無視する)。
#[derive(Deserialize)]
struct Catalog {
    hasami_version: String,
    format_version: u32,
    recommended: String,
    dictionaries: Vec<Entry>,
}

#[derive(Deserialize)]
struct Entry {
    name: String,
    summary: String,
    file: String,
    size: u64,
    sha256: String,
    /// zstd で圧縮した同じ辞書 (`<名前>.hsd.zst`)。`noslop dict download` は既定でこちらを取る。
    #[serde(default)]
    compressed: Option<Compressed>,
}

#[derive(Deserialize)]
struct Compressed {
    file: String,
    size: u64,
    sha256: String,
}

/// SHA-256 の書き方 (16 進の小文字 64 桁) か。
fn is_sha256(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn main() {
    println!("cargo:rerun-if-changed=dict/catalog.json");
    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let path = manifest.join("dict").join("catalog.json");
    let raw = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let mut catalog: Catalog = serde_json::from_str(&raw)
        .unwrap_or_else(|e| panic!("dict/catalog.json を読めません: {e}"));
    if let Err(problem) = validate(&catalog) {
        panic!("dict/catalog.json が正しくありません: {problem}");
    }
    // 一覧は小さい辞書から並べる (目録は推奨の辞書が先頭)
    catalog.dictionaries.sort_by_key(|d| d.size);
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR")).join("catalog.rs");
    fs::write(&out, generate(&catalog)).unwrap_or_else(|e| panic!("{}: {e}", out.display()));
}

/// 目録の形を確かめる。名前はファイル名と CLI の引数に、SHA-256 は照合に使うので厳しく見る。
fn validate(catalog: &Catalog) -> Result<(), String> {
    let parts: Vec<&str> = catalog.hasami_version.split('.').collect();
    if parts.len() != 3
        || parts
            .iter()
            .any(|p| p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()))
    {
        return Err(format!(
            "hasami_version が数字 3 つの版ではありません: {:?}",
            catalog.hasami_version
        ));
    }
    if catalog.format_version == 0 {
        return Err("format_version が 0 です".to_string());
    }
    if catalog.dictionaries.is_empty() {
        return Err("辞書がありません".to_string());
    }
    let mut names = HashSet::new();
    for dict in &catalog.dictionaries {
        let name = &dict.name;
        if name.is_empty()
            || name.starts_with('-')
            || !name
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        {
            return Err(format!(
                "辞書の名前に使えない文字があります (英小文字・数字・- だけ): {name:?}"
            ));
        }
        if !names.insert(name.as_str()) {
            return Err(format!("辞書の名前が重なっています: {name}"));
        }
        if dict.file != format!("{name}.hsd") {
            return Err(format!(
                "{name} のファイル名が <名前>.hsd ではありません: {:?}",
                dict.file
            ));
        }
        if dict.size == 0 {
            return Err(format!("{name} の大きさが 0 です"));
        }
        if !is_sha256(&dict.sha256) {
            return Err(format!(
                "{name} の SHA-256 が 16 進の小文字 64 桁ではありません: {:?}",
                dict.sha256
            ));
        }
        if let Some(c) = &dict.compressed {
            // 取得元の URL の末尾になるので、名前から決まる形だけを許す
            if c.file != format!("{}.zst", dict.file) {
                return Err(format!(
                    "{name} の圧縮版のファイル名が <名前>.hsd.zst ではありません: {:?}",
                    c.file
                ));
            }
            if c.size == 0 {
                return Err(format!("{name} の圧縮版の大きさが 0 です"));
            }
            if !is_sha256(&c.sha256) {
                return Err(format!(
                    "{name} の圧縮版の SHA-256 が 16 進の小文字 64 桁ではありません: {:?}",
                    c.sha256
                ));
            }
        }
    }
    if !names.contains(catalog.recommended.as_str()) {
        return Err(format!(
            "推奨の辞書 {:?} が目録にありません",
            catalog.recommended
        ));
    }
    Ok(())
}

/// `src/dictionaries.rs` に `include!` で取り込む Rust のコード。文字列は `{:?}` で書き、
/// 目録の値をそのまま Rust の文字列リテラルにする。
fn generate(catalog: &Catalog) -> String {
    let tag = format!("v{}", catalog.hasami_version);
    let mut code = String::new();
    let _ = writeln!(
        code,
        "// dict/catalog.json から build.rs が作ったコード。手で書き換えない\n"
    );
    let _ = writeln!(
        code,
        "/// 配布辞書を取る hasami のリリースのタグ (目録の `hasami_version`)。"
    );
    let _ = writeln!(code, "pub const HASAMI_TAG: &str = {tag:?};\n");
    let _ = writeln!(
        code,
        "/// 配布辞書の取得元 (URL の接頭辞。この後に `/<名前>.hsd.zst` か `/<名前>.hsd` を付けて取得する)。[`HASAMI_TAG`] の"
    );
    let _ = writeln!(code, "/// リリースの添付ファイルを指す。");
    let _ = writeln!(
        code,
        "pub const DEFAULT_SOURCE: &str = {:?};\n",
        format!("https://github.com/owayo/hasami/releases/download/{tag}")
    );
    let _ = writeln!(
        code,
        "/// 目録の辞書の形式の版 (目録の `format_version`)。依存の hasami が読める版と同じでなければならない。"
    );
    let _ = writeln!(
        code,
        "pub const CATALOG_FORMAT_VERSION: u32 = {};\n",
        catalog.format_version
    );
    let _ = writeln!(
        code,
        "/// `noslop dict download` で名前を省いたときに取る辞書 (目録の `recommended`)。"
    );
    let _ = writeln!(
        code,
        "pub const RECOMMENDED: &str = {:?};\n",
        catalog.recommended
    );
    let _ = writeln!(
        code,
        "/// [`HASAMI_TAG`] の配布辞書 (小さい順)。大きさと SHA-256 は目録の `size` と `sha256`、圧縮版は `compressed`。"
    );
    let _ = writeln!(
        code,
        "pub const DICTIONARIES: [Distributed; {}] = [",
        catalog.dictionaries.len()
    );
    for dict in &catalog.dictionaries {
        let compressed = match &dict.compressed {
            Some(c) => format!(
                "Some(Compressed {{ file: {:?}, size: {}, sha256: {:?} }})",
                c.file, c.size, c.sha256
            ),
            None => "None".to_string(),
        };
        let _ = writeln!(
            code,
            "    Distributed {{ name: {:?}, summary: {:?}, size: {}, sha256: {:?}, compressed: {compressed} }},",
            dict.name, dict.summary, dict.size, dict.sha256
        );
    }
    let _ = writeln!(code, "];");
    code
}
