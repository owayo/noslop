//! 独立した依存グラフで、文書用ライブラリの組み込みと依存の境界を確かめる。

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::process::Command;

use serde::Deserialize;

#[derive(Deserialize)]
struct Metadata {
    packages: Vec<Package>,
    resolve: Resolve,
}

#[derive(Deserialize)]
struct Package {
    id: String,
    name: String,
}

#[derive(Deserialize)]
struct Resolve {
    nodes: Vec<Node>,
}

#[derive(Deserialize)]
struct Node {
    id: String,
    dependencies: Vec<String>,
}

pub(super) fn check(root: &Path) -> Result<(), String> {
    let manifest = root.join("tests/fixtures/document-only/Cargo.toml");
    // 本体の lockfile を種にし、組み込み先固有の依存だけを解決する。
    // 本体の版が変わっても、テスト用の lockfile を追跡して追従させる必要はない。
    std::fs::copy(
        root.join("Cargo.lock"),
        manifest.with_file_name("Cargo.lock"),
    )
    .map_err(|e| format!("組み込みの lockfile を準備できません: {e}"))?;
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let metadata = Command::new(&cargo)
        .args(["metadata", "--format-version", "1", "--manifest-path"])
        .arg(&manifest)
        .output()
        .map_err(|e| format!("組み込みの依存を確認できません: {e}"))?;
    if !metadata.status.success() {
        return Err(String::from_utf8_lossy(&metadata.stderr).into_owned());
    }
    let metadata: Metadata = serde_json::from_slice(&metadata.stdout).map_err(|e| e.to_string())?;
    let packages: HashMap<_, _> = metadata
        .packages
        .iter()
        .map(|p| (p.id.as_str(), p.name.as_str()))
        .collect();
    let nodes: HashMap<_, _> = metadata
        .resolve
        .nodes
        .iter()
        .map(|n| (n.id.as_str(), n))
        .collect();
    let library = metadata
        .packages
        .iter()
        .find(|p| p.name == "noslop")
        .ok_or("noslop が依存にありません")?;
    let mut pending = vec![library.id.as_str()];
    let mut seen = HashSet::new();
    while let Some(id) = pending.pop() {
        if !seen.insert(id) {
            continue;
        }
        let name = packages.get(id).ok_or("依存のパッケージが見つかりません")?;
        if name.starts_with("tree-sitter")
            || [
                "clap",
                "ignore",
                "globset",
                "process-wrap",
                "anstream",
                "ureq",
                "ruzstd",
            ]
            .contains(name)
        {
            return Err(format!("文書用のライブラリに不要な依存があります: {name}"));
        }
        let node = nodes.get(id).ok_or("依存のノードが見つかりません")?;
        pending.extend(node.dependencies.iter().map(String::as_str));
    }
    println!("文書用の依存に tree-sitter・CLI・辞書取得を含まないことを確認しました");
    let status = Command::new(cargo)
        .args(["test", "--locked", "--manifest-path"])
        .arg(manifest)
        .arg("--target-dir")
        .arg(root.join("target"))
        .status()
        .map_err(|e| format!("組み込みのテストを実行できません: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err("文書用ライブラリの組み込みテストが失敗しました".into())
    }
}
