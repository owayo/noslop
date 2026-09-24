//! TOON (Token-Oriented Object Notation) での符号化 (`--format toon`)。
//!
//! JSON と同じデータを、インデントと表形式 (一様な object の配列は、項目名の見出し 1 行と
//! 1 要素 1 行) で表す。同じ内容をより少ないトークンで LLM に渡すために使う。符号化は
//! 公式の Rust 実装 [`toon-format`](https://github.com/toon-format/toon-rust) に任せる
//! (対応する仕様は v3.0)。区切りはカンマ、インデントは 2、キーの折りたたみはしない。
//!
//! `toon-format` は `serde_json` の `preserve_order` を有効にするので、`serde_json::Map`
//! (`json!` マクロ) で組み立てる JSON のキー順は、辞書順ではなく書いた順になる。

use std::io;

use serde::Serialize;
use toon_format::EncodeOptions;

/// TOON の文書にする。仕様どおり、末尾に改行は付かない。
pub fn encode<T: Serialize>(value: &T) -> io::Result<String> {
    toon_format::encode(value, &EncodeOptions::default()).map_err(io::Error::other)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Serialize)]
    struct Row {
        line: usize,
        message: &'static str,
    }

    #[derive(Serialize)]
    struct Doc {
        path: &'static str,
        rows: Vec<Row>,
        empty: Vec<Row>,
    }

    #[test]
    fn uniform_objects_become_a_table_in_field_order() {
        let doc = Doc {
            path: "draft.md",
            rows: vec![
                Row {
                    line: 3,
                    message: "「と言えるだろう」は結論の締めです",
                },
                Row {
                    line: 8,
                    message: "カンマ, コロン: を含む",
                },
            ],
            empty: Vec::new(),
        };
        let s = encode(&doc).unwrap();
        assert!(s.starts_with("path: draft.md\n"), "宣言の順: {s}");
        assert!(s.contains("rows[2]{line,message}:\n"), "{s}");
        assert!(
            s.contains("  3,「と言えるだろう」は結論の締めです\n"),
            "{s}"
        );
        assert!(
            s.contains("  8,\"カンマ, コロン: を含む\""),
            "区切り文字とコロンを含む値は引用符で囲む: {s}"
        );
        assert!(!s.ends_with('\n'), "末尾に改行を付けない: {s:?}");
    }
}
