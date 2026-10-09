//! 同梱の辞書 (`dict/ipadic.hsd`) を、依存の hasami と同じリリースの `ipadic.hsd` に置き換える。
//!
//! `make dict-bundled-update` (`tools/dict-bundled.sh --update`) が、hasami のリリースから取得した
//! `ipadic.hsd` と `dictionaries.json` のディレクトリとタグを渡す。独立した準備コマンドから呼ぶ。
//!
//! 1. タグがリンクした hasami の版 (`hasami::download::CURRENT_TAG`) と同じことを確かめる
//! 2. 辞書の大きさと SHA-256 を `dictionaries.json` の `ipadic` の項目と照らす
//! 3. リンクした hasami で辞書を読み、名前・作った hasami の版・出典を確かめる
//! 4. `dict/ipadic.hsd` を置き換え、`dict/README.md` の表 (出所・形式・出典・語数・SHA-256) と
//!    THIRD_PARTY_NOTICES.md の同梱の辞書の版と、固定情報 (`dict/bundled.json`) を書き換える
//!
//! 1〜3 と書き換える文書の組み立てが済むまで、ファイルには触らない。書き換えた結果は
//! `dictionaries::tests::the_bundled_dictionary_matches_dict_readme` と
//! `morph::tests::the_bundled_dictionary_is_ipadic_from_hasami` が確かめる。

use std::fs;
use std::path::Path;

use crate::bundled_manifest::{BundledManifest, read_manifest, validate_manifest};
use hasami::Dictionary;
use hasami::hsd::meta::KEY_HASAMI_VERSION;
use serde::Deserialize;

/// 同梱する辞書の名前 (目録の `name`)。
const NAME: &str = "ipadic";

/// hasami のリリースの目録 (`dictionaries.json`) のうち、ここで使う項目。
#[derive(Deserialize)]
struct Catalog {
    hasami_version: String,
    dictionaries: Vec<Entry>,
}

#[derive(Deserialize)]
struct Entry {
    name: String,
    file: String,
    size: u64,
    sha256: String,
    sources: String,
}

pub(super) fn update(root: &Path, dir: &Path, tag: &str) -> Result<(), String> {
    let version = tag
        .strip_prefix('v')
        .ok_or_else(|| format!("タグの形式が違います: {tag} (v26.10.100 のように書く)"))?;
    if tag != hasami::download::CURRENT_TAG {
        return Err(format!(
            "リンクした hasami は {} で、タグ {tag} と違います。Cargo.toml の hasami のタグを {tag} にして \
             cargo update -p hasami を実行してから取り直してください (make hasami-update)",
            hasami::download::CURRENT_TAG
        ));
    }

    let entry = catalog_entry(&dir.join("dictionaries.json"), version)?;
    let candidate = dir.join(&entry.file);
    let size = fs::metadata(&candidate)
        .map_err(|e| format!("{} を読めません: {e}", candidate.display()))?
        .len();
    if size != entry.size {
        return Err(format!(
            "{} の大きさが {size} バイトで、目録の {} バイトと違います",
            entry.file, entry.size
        ));
    }
    let sha256 = hasami::download::sha256_file(&candidate)
        .map_err(|e| format!("{} を読めません: {e}", candidate.display()))?;
    if sha256 != entry.sha256 {
        return Err(format!(
            "{} の SHA-256 が {sha256} で、目録の {} と違います",
            entry.file, entry.sha256
        ));
    }

    let dict = Dictionary::load(&candidate)
        .map_err(|e| format!("{} を依存の hasami で読めません: {e}", entry.file))?;
    let meta = dict.meta();
    if meta.name() != NAME {
        return Err(format!(
            "{} の辞書の名前が {} で、{NAME} ではありません",
            entry.file,
            meta.name()
        ));
    }
    let built_by = meta.get(KEY_HASAMI_VERSION).unwrap_or("(なし)");
    if built_by != version {
        return Err(format!(
            "{} は hasami {built_by} で作った辞書で、{tag} のものではありません",
            entry.file
        ));
    }
    let sources = meta.get("sources").unwrap_or("");
    if sources != entry.sources {
        return Err(format!(
            "{} の出典 (sources) が {sources} で、目録の {} と違います",
            entry.file, entry.sources
        ));
    }

    let manifest_path = root.join("dict/bundled.json");
    let previous = read_manifest(&manifest_path)?;
    let manifest = BundledManifest {
        schema: 1,
        hasami_tag: tag.into(),
        file: entry.file.clone(),
        size: entry.size,
        sha256: sha256.clone(),
        format_version: hasami::hsd::FORMAT_VERSION,
        sources: sources.into(),
        entries: dict.entry_count(),
        ipadic_patch: meta.get("ipadic_patch").map(str::to_string),
    };
    validate_manifest(&manifest)?;
    let manifest_text = serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())? + "\n";
    let readme_path = root.join("dict/README.md");
    let readme = rewrite_readme(
        &read(&readme_path)?,
        tag,
        sources,
        dict.entry_count(),
        &sha256,
    )?;
    let notices_path = root.join("THIRD_PARTY_NOTICES.md");
    let notices = rewrite_notices(&read(&notices_path)?, version)?;

    // THIRD_PARTY_NOTICES.md の mecab-ipadic からの変更点は、辞書のメタデータの ipadic_patch を写している
    let target = root.join("dict/ipadic.hsd");
    let patch_changed = previous.ipadic_patch != manifest.ipadic_patch;

    // 同じディレクトリの一時ファイルに写してから置き換える (途中で失敗しても既存の辞書を壊さない)
    let staged = root.join("dict/ipadic.hsd.tmp");
    fs::copy(&candidate, &staged).map_err(|e| format!("{} に写せません: {e}", staged.display()))?;
    fs::rename(&staged, &target)
        .map_err(|e| format!("{} を置き換えられません: {e}", target.display()))?;
    write(&readme_path, &readme)?;
    write(&notices_path, &notices)?;
    write(&manifest_path, &manifest_text)?;

    println!(
        "同梱の辞書を hasami {tag} の {} にしました (語数 {}、SHA-256 {sha256})",
        entry.file,
        grouped(dict.entry_count())
    );
    if patch_changed {
        println!(
            "注意: 辞書のメタデータの ipadic_patch が変わりました。THIRD_PARTY_NOTICES.md の mecab-ipadic からの\
             変更点の記述を、新しい値に合わせて確かめてください:\n  {}",
            meta.get("ipadic_patch").unwrap_or("(なし)")
        );
    }
    Ok(())
}

/// 目録から同梱する辞書の項目を取り出す。目録の版がタグと違えば誤り。
fn catalog_entry(path: &Path, version: &str) -> Result<Entry, String> {
    let catalog: Catalog = serde_json::from_str(&read(path)?)
        .map_err(|e| format!("{} を読めません: {e}", path.display()))?;
    if catalog.hasami_version != version {
        return Err(format!(
            "目録の hasami_version が {} で、v{version} のものではありません",
            catalog.hasami_version
        ));
    }
    let mut entries = catalog.dictionaries.into_iter().filter(|e| e.name == NAME);
    match (entries.next(), entries.next()) {
        (Some(entry), None) => Ok(entry),
        (None, _) => Err(format!("目録に {NAME} の項目がありません")),
        (Some(_), Some(_)) => Err(format!("目録に {NAME} の項目が 2 つ以上あります")),
    }
}

/// dict/README.md の表の出所・形式・語数・SHA-256 の行を書き直し、元のデータの行の出典を差し替える。
fn rewrite_readme(
    text: &str,
    tag: &str,
    sources: &str,
    entries: usize,
    sha256: &str,
) -> Result<String, String> {
    let rows = [
        (
            "出所",
            format!("hasami {tag} のリリースに添付された `ipadic.hsd`"),
        ),
        ("形式", format!("HSD v{}", hasami::hsd::FORMAT_VERSION)),
        ("語数", grouped(entries)),
        ("SHA-256", format!("`{sha256}`")),
    ];
    let mut found = [false; 4];
    let mut sources_found = false;
    let mut lines = Vec::new();
    for line in text.split('\n') {
        if let Some(i) = rows
            .iter()
            .position(|(label, _)| line.starts_with(&format!("| {label} | ")))
        {
            if found[i] {
                return Err(format!(
                    "dict/README.md に「{}」の行が 2 つあります",
                    rows[i].0
                ));
            }
            found[i] = true;
            lines.push(format!("| {} | {} |", rows[i].0, rows[i].1));
        } else if line.starts_with("| 元のデータ | ") {
            if sources_found {
                return Err("dict/README.md に「元のデータ」の行が 2 つあります".into());
            }
            sources_found = true;
            lines.push(
                replace_between(line, "`sources=", "`", sources).ok_or_else(|| {
                    "dict/README.md の「元のデータ」の行に `sources=...` がありません".to_string()
                })?,
            );
        } else {
            lines.push(line.to_string());
        }
    }
    if let Some(i) = found.iter().position(|f| !f) {
        return Err(format!(
            "dict/README.md に「{}」の行がありません",
            rows[i].0
        ));
    }
    if !sources_found {
        return Err("dict/README.md に「元のデータ」の行がありません".into());
    }
    Ok(lines.join("\n"))
}

/// THIRD_PARTY_NOTICES.md の、同梱の辞書がどの版の hasami の配布辞書かを書いた 2 か所 (日本語と英語) の版を差し替える。
fn rewrite_notices(text: &str, version: &str) -> Result<String, String> {
    let mut text = text.to_string();
    for (start, end) in [
        ("`dict/ipadic.hsd`。hasami v", " の配布辞書"),
        ("the distributed dictionary of hasami v", ";"),
    ] {
        if text.matches(start).count() != 1 {
            return Err(format!(
                "THIRD_PARTY_NOTICES.md に「{start}」がちょうど 1 つではありません"
            ));
        }
        text = replace_between(&text, start, end, version).ok_or_else(|| {
            format!("THIRD_PARTY_NOTICES.md の「{start}」のあとに「{end}」がありません")
        })?;
    }
    Ok(text)
}

/// `start` の直後から次の `end` の手前までを `value` に差し替える (最初の `start` だけ)。
fn replace_between(text: &str, start: &str, end: &str, value: &str) -> Option<String> {
    let from = text.find(start)? + start.len();
    let to = from + text[from..].find(end)?;
    Some(format!("{}{value}{}", &text[..from], &text[to..]))
}

/// 3 桁ごとにカンマで区切る (390860 → 390,860)。
fn grouped(n: usize) -> String {
    let digits = n.to_string();
    let groups: Vec<&str> = digits
        .as_bytes()
        .rchunks(3)
        .rev()
        .map(|chunk| std::str::from_utf8(chunk).expect("数字は ASCII"))
        .collect();
    groups.join(",")
}

fn read(path: &Path) -> Result<String, String> {
    fs::read_to_string(path).map_err(|e| format!("{} を読めません: {e}", path.display()))
}

fn write(path: &Path, text: &str) -> Result<(), String> {
    fs::write(path, text).map_err(|e| format!("{} に書けません: {e}", path.display()))
}
