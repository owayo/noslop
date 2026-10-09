//! 同梱辞書の準備。本体をビルドしないので、辞書がない初回でも起動できる。

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use hasami::download::{DistributedDict, DownloadOptions};

#[path = "../../bundled_manifest.rs"]
mod bundled_manifest;
#[path = "../../dict_bundled.rs"]
mod update;

use bundled_manifest::{BundledManifest, matches_file, read_manifest};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("エラー: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    match args.as_slice() {
        [command] if command == "prepare" || command == "check" => {
            let manifest = read_manifest(&root.join("dict/bundled.json"))?;
            check_tag(&manifest)?;
            let dir = root.join("dict");
            if command == "check" {
                if !matches_file(&dir.join(&manifest.file), &manifest)? {
                    return Err("同梱辞書がないか固定情報と違います。make dict-bundled で準備してください".into());
                }
            } else {
                ensure(&manifest, &dir, |staging| download(&manifest, staging))?;
            }
            println!("同梱辞書は {} の {} (SHA-256 照合済み)", manifest.hasami_tag, manifest.file);
            Ok(())
        }
        [command, dir, tag] if command == "update" => {
            let tag = tag.to_str().ok_or("タグが UTF-8 ではありません")?;
            update::update(&root, Path::new(dir), tag)
        }
        _ => Err("使い方: cargo run --locked -p noslop-xtask -- prepare | check | update <取得したディレクトリ> <タグ>".into()),
    }
}

fn check_tag(manifest: &BundledManifest) -> Result<(), String> {
    if manifest.hasami_tag != hasami::download::CURRENT_TAG {
        return Err(format!(
            "dict/bundled.json は {}、依存の hasami は {} です。make hasami-update TAG={} でそろえてください",
            manifest.hasami_tag,
            hasami::download::CURRENT_TAG,
            hasami::download::CURRENT_TAG
        ));
    }
    Ok(())
}

/// 既存の辞書も毎回照合する。取得したものは別の場所で照合してから置き換える。
fn ensure(
    manifest: &BundledManifest,
    dir: &Path,
    fetch: impl FnOnce(&Path) -> Result<(), String>,
) -> Result<(), String> {
    let target = dir.join(&manifest.file);
    if matches_file(&target, manifest)? {
        return Ok(());
    }
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let staging = tempfile::tempdir_in(dir).map_err(|e| e.to_string())?;
    fetch(staging.path())?;
    let candidate = staging.path().join(&manifest.file);
    if !matches_file(&candidate, manifest)? {
        return Err("取得した辞書が dict/bundled.json の大きさ・SHA-256 と一致しません".into());
    }
    fs::rename(&candidate, &target)
        .map_err(|e| format!("{} を置き換えられません: {e}", target.display()))
}

fn download(manifest: &BundledManifest, dir: &Path) -> Result<(), String> {
    let dict = DistributedDict::new("ipadic", manifest.size, &manifest.sha256);
    let source = format!(
        "https://github.com/owayo/hasami/releases/download/{}",
        manifest.hasami_tag
    );
    println!("{} の {} を取得します", manifest.hasami_tag, manifest.file);
    hasami::download::download(
        &dict,
        dir,
        DownloadOptions {
            base_url: Some(&source),
            compressed: false,
            force: true,
            progress: None,
        },
    )
    .map_err(|e| format!("同梱辞書を取得できません: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    fn manifest(bytes: &[u8]) -> BundledManifest {
        BundledManifest {
            schema: 1,
            hasami_tag: hasami::download::CURRENT_TAG.into(),
            file: "ipadic.hsd".into(),
            size: bytes.len() as u64,
            sha256: Sha256::digest(bytes)
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect(),
            format_version: hasami::hsd::FORMAT_VERSION,
            sources: "ipadic@test".into(),
            entries: 1,
            ipadic_patch: None,
        }
    }

    #[test]
    fn a_verified_cache_needs_no_network() {
        let dir = tempfile::tempdir().unwrap();
        let bytes = b"verified dictionary";
        fs::write(dir.path().join("ipadic.hsd"), bytes).unwrap();
        ensure(&manifest(bytes), dir.path(), |_| {
            panic!("通信してはいけない")
        })
        .unwrap();
    }

    #[test]
    fn missing_wrong_size_and_wrong_hash_caches_are_replaced() {
        for previous in [None, Some(b"old".as_slice()), Some(b"bad".as_slice())] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("ipadic.hsd");
            if let Some(previous) = previous {
                fs::write(&path, previous).unwrap();
            }
            let bytes = b"new";
            ensure(&manifest(bytes), dir.path(), |staging| {
                fs::write(staging.join("ipadic.hsd"), bytes).map_err(|e| e.to_string())
            })
            .unwrap();
            assert_eq!(fs::read(&path).unwrap(), bytes);
            assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
        }
    }

    #[test]
    fn failed_downloads_and_wrong_contents_preserve_the_previous_file() {
        for fail in [true, false] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("ipadic.hsd");
            fs::write(&path, b"old").unwrap();
            let result = ensure(&manifest(b"new"), dir.path(), |staging| {
                fs::write(staging.join("ipadic.hsd"), b"bad").unwrap();
                if fail {
                    Err("通信の失敗".into())
                } else {
                    Ok(())
                }
            });
            assert!(result.is_err());
            assert_eq!(fs::read(&path).unwrap(), b"old");
            assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
        }
    }

    #[test]
    fn unknown_schemas_and_download_paths_are_rejected() {
        let mut m = manifest(b"dictionary");
        m.schema = 2;
        assert!(bundled_manifest::validate_manifest(&m).is_err());
        m.schema = 1;
        m.file = "../ipadic.hsd".into();
        assert!(bundled_manifest::validate_manifest(&m).is_err());
    }

    #[test]
    fn the_dictionary_tag_must_match_the_linked_dependency() {
        let mut m = manifest(b"dictionary");
        check_tag(&m).unwrap();
        m.hasami_tag = "v0.0.1".into();
        assert!(check_tag(&m).unwrap_err().contains("make hasami-update"));
    }
}
