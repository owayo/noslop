//! 検査するファイルを集める。
//!
//! - ディレクトリは再帰的にたどる。`.gitignore` / `.ignore` / `.noslopignore` を尊重し、
//!   隠しファイルは飛ばす (git リポジトリの外でも `.gitignore` を読む)
//! - ディレクトリから集めるファイルは拡張子で絞り、設定の `exclude` (.gitignore と同じ書式) に一致する
//!   ものを除く。基準は、プロジェクトの設定なら設定ファイルのディレクトリ、ユーザーの設定なら検査の
//!   起点 (渡したディレクトリ) ([`ExcludeRule`])
//! - コマンドラインで直接指定したファイルは、拡張子や除外の指定に関係なく必ず検査する
//! - シンボリックリンクのファイルはリンク先を見て判定する (ディレクトリのリンクはたどらない)

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ignore::WalkBuilder;
use ignore::gitignore::{Gitignore, GitignoreBuilder};

use crate::config::ConfigError;

/// 既定で検査する拡張子。
pub const DEFAULT_EXTENSIONS: [&str; 3] = ["md", "markdown", "txt"];

/// 独自の除外ファイル名 (`.gitignore` と同じ書式)。
pub const IGNORE_FILE_NAME: &str = ".noslopignore";

/// ファイルを集める条件。
#[derive(Clone)]
pub struct WalkOptions {
    /// 小文字の拡張子 (ドットなし)。
    pub extensions: Vec<String>,
    pub exclude: Option<ExcludeRule>,
}

impl WalkOptions {
    /// `root` を起点に検査するとき、`path` を除外するか (設定の `exclude`。ディレクトリをたどらずに
    /// 1 つのファイルだけを見るフック向け)。
    pub fn is_excluded(&self, root: &Path, path: &Path, is_dir: bool) -> bool {
        self.exclude
            .as_ref()
            .and_then(|rule| rule.under(root).ok())
            .is_some_and(|exclude| exclude.is_excluded(path, is_dir))
    }
}

/// 設定の `exclude` とその基準。
#[derive(Clone)]
pub enum ExcludeRule {
    /// プロジェクトの設定の除外。基準は設定ファイルのディレクトリ。
    Fixed(Arc<Exclude>),
    /// ユーザーの設定の除外。基準は検査の起点 (渡したディレクトリ)。どのプロジェクトにも当てるので、
    /// 置き場 (`~/.config/noslop`) を基準にはできない。
    PerRoot(Arc<[String]>),
}

impl ExcludeRule {
    /// `root` を起点に検査するときの除外。
    pub fn under(&self, root: &Path) -> Result<Arc<Exclude>, ConfigError> {
        match self {
            ExcludeRule::Fixed(exclude) => Ok(Arc::clone(exclude)),
            ExcludeRule::PerRoot(patterns) => Exclude::new(root, patterns).map(Arc::new),
        }
    }
}

impl Default for WalkOptions {
    fn default() -> Self {
        Self {
            extensions: DEFAULT_EXTENSIONS.iter().map(|s| s.to_string()).collect(),
            exclude: None,
        }
    }
}

/// 設定の `exclude`。
pub struct Exclude {
    root: PathBuf,
    matcher: Gitignore,
}

impl Exclude {
    /// `root` (設定ファイルのディレクトリ) 基準で除外パターンを組み立てる。
    pub fn new(root: &Path, patterns: &[String]) -> Result<Self, ConfigError> {
        let root = absolute(root);
        let mut builder = GitignoreBuilder::new(&root);
        for pattern in patterns {
            builder.add_line(None, pattern).map_err(|e| {
                ConfigError::Invalid(format!(
                    "[files] exclude のパターンが正しくありません: {pattern}: {e}"
                ))
            })?;
        }
        let matcher = builder.build().map_err(|e| {
            ConfigError::Invalid(format!("[files] exclude を組み立てられません: {e}"))
        })?;
        Ok(Self { root, matcher })
    }

    /// 除外するパスか。
    pub fn is_excluded(&self, path: &Path, is_dir: bool) -> bool {
        let path = absolute(path);
        if !path.starts_with(&self.root) {
            return false;
        }
        self.matcher
            .matched_path_or_any_parents(&path, is_dir)
            .is_ignore()
    }
}

fn absolute(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}

/// 集められなかったパス。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalkError {
    pub path: String,
    pub message: String,
}

/// 集めた結果。
#[derive(Debug, Default)]
pub struct Collected {
    /// 検査するファイル (重複を除いてパス順)。
    pub files: Vec<PathBuf>,
    /// 集められなかったパス。
    pub errors: Vec<WalkError>,
    /// 直接指定したディレクトリのうち、検査するファイルが 1 件も見つからなかったもの (表示用のパス)。
    ///
    /// `.gitignore` の `dir/**` のようにファイルに当たる除外は、指定したディレクトリの中身にも
    /// 当たる。エラーにはしないが、黙って 0 件で終わると理由に気づけないので呼び出し側で知らせる。
    pub empty_dirs: Vec<String>,
}

/// パスの一覧から検査するファイルを集める。
pub fn collect(paths: &[PathBuf], options: &WalkOptions) -> Collected {
    let mut files = BTreeSet::new();
    let mut errors = Vec::new();
    let mut empty_dirs = Vec::new();
    for path in paths {
        if path.is_file() {
            files.insert(clean(path));
            continue;
        }
        if !path.is_dir() {
            errors.push(WalkError {
                path: display(path),
                message: "ファイルまたはディレクトリが見つかりません".to_string(),
            });
            continue;
        }
        let mut builder = WalkBuilder::new(path);
        builder
            .hidden(true)
            .git_ignore(true)
            .git_global(true)
            .git_exclude(true)
            .require_git(false)
            .add_custom_ignore_filename(IGNORE_FILE_NAME);
        if let Some(rule) = &options.exclude {
            let exclude = match rule.under(path) {
                Ok(exclude) => exclude,
                Err(e) => {
                    errors.push(WalkError {
                        path: display(path),
                        message: e.to_string(),
                    });
                    continue;
                }
            };
            builder.filter_entry(move |entry| {
                let is_dir = entry.file_type().is_some_and(|t| t.is_dir());
                !exclude.is_excluded(entry.path(), is_dir)
            });
        }
        let mut found = 0usize;
        let mut failed = false;
        for entry in builder.build() {
            match entry {
                Ok(entry) => {
                    let p = entry.path();
                    if p.is_file() && has_extension(p, &options.extensions) {
                        files.insert(clean(p));
                        found += 1;
                    }
                }
                Err(e) => {
                    failed = true;
                    errors.push(WalkError {
                        path: display(path),
                        message: format!("ディレクトリをたどれません: {e}"),
                    });
                }
            }
        }
        // たどれなかったときはエラーとして出ているので、空とは言わない
        if found == 0 && !failed {
            empty_dirs.push(display(path));
        }
    }
    Collected {
        files: files.into_iter().collect(),
        errors,
        empty_dirs,
    }
}

/// 拡張子が一覧のどれかか (大文字小文字は区別しない)。
pub(crate) fn has_extension(path: &Path, extensions: &[String]) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| extensions.iter().any(|x| x.eq_ignore_ascii_case(e)))
}

/// 先頭の `./` を取り除く (表示とソートを安定させるため)。
fn clean(path: &Path) -> PathBuf {
    path.strip_prefix(".")
        .ok()
        .filter(|p| !p.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| path.to_path_buf())
}

/// 表示用のパス文字列 (区切りは `/` にそろえる)。
pub fn display(path: &Path) -> String {
    let s = path.to_string_lossy();
    if std::path::MAIN_SEPARATOR == '\\' {
        s.replace('\\', "/")
    } else {
        s.into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn names(root: &Path, files: &[PathBuf]) -> Vec<String> {
        files
            .iter()
            .map(|f| display(f.strip_prefix(root).unwrap_or(f)))
            .collect()
    }

    #[test]
    fn collects_by_extension_and_respects_ignore_files() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("docs/sub")).unwrap();
        fs::create_dir_all(root.join("build")).unwrap();
        fs::create_dir_all(root.join(".hidden")).unwrap();
        fs::write(root.join("README.md"), "a").unwrap();
        fs::write(root.join("docs/a.markdown"), "a").unwrap();
        fs::write(root.join("docs/sub/b.txt"), "a").unwrap();
        fs::write(root.join("docs/c.rs"), "a").unwrap();
        fs::write(root.join("build/out.md"), "a").unwrap();
        fs::write(root.join(".hidden/x.md"), "a").unwrap();
        fs::write(root.join(".gitignore"), "build/\n").unwrap();
        fs::write(root.join(".noslopignore"), "docs/sub/\n").unwrap();
        let c = collect(&[root.to_path_buf()], &WalkOptions::default());
        assert!(c.errors.is_empty(), "{:?}", c.errors);
        assert!(c.empty_dirs.is_empty());
        assert_eq!(names(root, &c.files), vec!["README.md", "docs/a.markdown"]);
    }

    #[test]
    fn directories_without_files_are_reported() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        // `dir/**` はファイルに当たるので、指定したディレクトリの中身まで除外される。
        // `dir/` はディレクトリに当たるだけなので、その中を起点にすれば読める
        fs::create_dir_all(root.join("corpus/human")).unwrap();
        fs::create_dir_all(root.join("drafts/new")).unwrap();
        fs::create_dir_all(root.join("code")).unwrap();
        fs::write(root.join("corpus/human/a.md"), "a").unwrap();
        fs::write(root.join("drafts/new/b.md"), "a").unwrap();
        fs::write(root.join("code/main.rs"), "a").unwrap();
        fs::write(root.join(".gitignore"), "/corpus/**\n/drafts/\n").unwrap();
        let paths = [
            root.join("corpus/human"),
            root.join("drafts/new"),
            root.join("code"),
        ];
        let c = collect(&paths, &WalkOptions::default());
        assert!(c.errors.is_empty(), "{:?}", c.errors);
        assert_eq!(names(root, &c.files), vec!["drafts/new/b.md"]);
        assert_eq!(
            c.empty_dirs,
            vec![display(&paths[0]), display(&paths[2])],
            "中身が除外されたディレクトリと、対象の拡張子がないディレクトリ"
        );
    }

    #[test]
    fn exclude_patterns_are_relative_to_the_config_dir() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("vendor")).unwrap();
        fs::create_dir_all(root.join("docs")).unwrap();
        fs::write(root.join("vendor/v.md"), "a").unwrap();
        fs::write(root.join("docs/CHANGELOG.md"), "a").unwrap();
        fs::write(root.join("docs/keep.md"), "a").unwrap();
        let exclude = Exclude::new(root, &["vendor/".into(), "CHANGELOG.md".into()]).unwrap();
        let options = WalkOptions {
            exclude: Some(ExcludeRule::Fixed(Arc::new(exclude))),
            ..Default::default()
        };
        let c = collect(&[root.to_path_buf()], &options);
        assert_eq!(names(root, &c.files), vec!["docs/keep.md"]);
    }

    /// ユーザーの設定の除外は、検査の起点 (渡したディレクトリ) が基準。
    #[test]
    fn user_exclude_patterns_are_relative_to_each_search_root() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for file in [
            "a/drafts/x.md",
            "a/keep.md",
            "b/drafts/y.md",
            "b/sub/drafts/z.md",
        ] {
            fs::create_dir_all(root.join(file).parent().unwrap()).unwrap();
            fs::write(root.join(file), "a").unwrap();
        }
        let options = WalkOptions {
            exclude: Some(ExcludeRule::PerRoot(vec!["/drafts/".to_string()].into())),
            ..Default::default()
        };
        // 先頭の / は起点に当てる。起点ごとに基準が変わる
        let c = collect(&[root.join("a"), root.join("b")], &options);
        assert_eq!(
            names(root, &c.files),
            vec!["a/keep.md", "b/sub/drafts/z.md"]
        );
        // 起点が変われば、同じパターンでも当たる場所が変わる
        let c = collect(&[root.join("b/sub")], &options);
        assert!(c.files.is_empty(), "{:?}", c.files);

        // フックのように 1 つのファイルだけを見るときも、起点を基準にする
        assert!(options.is_excluded(&root.join("a"), &root.join("a/drafts/x.md"), false));
        assert!(!options.is_excluded(root, &root.join("a/drafts/x.md"), false));
    }

    #[test]
    fn explicit_files_are_always_included_and_missing_paths_are_errors() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("notes.text");
        fs::write(&file, "a").unwrap();
        let missing = dir.path().join("missing.md");
        let c = collect(
            &[file.clone(), missing, file.clone()],
            &WalkOptions::default(),
        );
        assert_eq!(c.files, vec![file]);
        assert_eq!(c.errors.len(), 1);
        assert!(c.errors[0].message.contains("見つかりません"));
        assert!(
            c.empty_dirs.is_empty(),
            "ファイルの指定は空のディレクトリに数えない"
        );
    }

    #[test]
    fn invalid_exclude_pattern_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert!(Exclude::new(dir.path(), &["docs/{a".into()]).is_err());
    }
}
