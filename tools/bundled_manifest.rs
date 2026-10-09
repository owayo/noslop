//! 同梱辞書の固定情報。ビルドと準備のコマンドで、同じ書式と照合を使う。

use std::fs;
use std::io::{self, Read};
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BundledManifest {
    pub schema: u32,
    pub hasami_tag: String,
    pub file: String,
    pub size: u64,
    pub sha256: String,
    pub format_version: u32,
    pub sources: String,
    pub entries: usize,
    pub ipadic_patch: Option<String>,
}

/// 固定情報を読み、取得先のパスに使ってよい形かを確かめる。
pub fn read_manifest(path: &Path) -> Result<BundledManifest, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let manifest: BundledManifest =
        serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    validate_manifest(&manifest)?;
    Ok(manifest)
}

pub fn validate_manifest(manifest: &BundledManifest) -> Result<(), String> {
    let tag_parts: Vec<_> = manifest
        .hasami_tag
        .strip_prefix('v')
        .unwrap_or("")
        .split('.')
        .collect();
    if manifest.schema != 1
        || manifest.file != "ipadic.hsd"
        || manifest.size == 0
        || manifest.entries == 0
        || manifest.format_version == 0
        || manifest.sources.is_empty()
        || tag_parts.len() != 3
        || tag_parts
            .iter()
            .any(|p| p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()))
        || manifest.sha256.len() != 64
        || !manifest
            .sha256
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(
            "dict/bundled.json の書式・版・辞書名・大きさ・SHA-256 が正しくありません".into(),
        );
    }
    Ok(())
}

/// ファイルがないときと、中身が固定情報と違うときは偽。読み取りの失敗は誤りとして返す。
pub fn matches_file(path: &Path, manifest: &BundledManifest) -> Result<bool, String> {
    let mut file = match fs::File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    if file.metadata().map_err(|e| e.to_string())?.len() != manifest.size {
        return Ok(false);
    }
    let mut digest = Sha256::new();
    let mut buffer = [0; 65536];
    loop {
        let len = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if len == 0 {
            break;
        }
        digest.update(&buffer[..len]);
    }
    let sha256: String = digest
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    Ok(sha256 == manifest.sha256)
}
