<p align="center">
  <img src="docs/images/app.png" width="128" alt="noslop">
</p>

<h1 align="center">noslop</h1>

<p align="center">
  日本語の文章から「AI 臭さ」を機械的に拾う Rust 製の Linter
</p>

<!-- standard:badges:start -->
<h3 align="center">対応プラットフォーム</h3>

<p align="center">
  <img src="https://img.shields.io/badge/Linux-FCC624?logo=linux&amp;logoColor=black" alt="Linux">
  <img src="https://img.shields.io/badge/macOS-000000?logo=apple&amp;logoColor=white" alt="macOS">
  <img src="https://img.shields.io/badge/Windows-0078D6" alt="Windows">
</p>

<p align="center">
  <a href="https://github.com/owayo/noslop/actions/workflows/ci.yml"><img src="https://github.com/owayo/noslop/actions/workflows/ci.yml/badge.svg?branch=main" alt="CI"></a>
  <a href="https://github.com/owayo/noslop/releases/latest"><img src="https://img.shields.io/github/v/release/owayo/noslop" alt="Release"></a>
  <a href="LICENSE"><img src="https://img.shields.io/github/license/owayo/noslop" alt="License"></a>
</p>
<!-- standard:badges:end -->

---

noslop は、LLM が書いた日本語（議事録・ブログ・技術文書）に出やすい癖を、決まった規則で指摘する Linter です。`と言えるでしょう` のような定型句、`〜ではなく` の対比の繰り返し、文の長さがそろいすぎた単調なリズム、見出しと太字だけで組んだ教科書的な構成を検出します。

noslop は「この文章は AI が書いた」と判定する道具ではありません。書き手は自分の文章の癖に気づきにくく、長い文書を目で追うと必ず見落としが出ます。noslop は疑わしい箇所を並べるところまでを受け持ち、直すか残すかは書き手が文脈で決めます。残すと決めた箇所には、理由を添えた抑制コメントを書けます。

文字種と語句のパターン、文長の統計で判定し、品詞で数えるルール（「の」の連鎖・連続漢字）は、同梱した形態素解析の辞書で判定します。単一のバイナリで動き、起動も速く、大きなリポジトリでもファイルを並列に処理します。

## 機能

- **辞書を同梱**: 形態素解析器 [hasami](https://github.com/owayo/hasami) の IPAdic の辞書をバイナリに同梱し、「の」の連鎖（P16）と連続漢字（P15）を品詞で判定する。インストールも設定も要らない（[形態素解析の辞書](docs/dictionaries.md#形態素解析の辞書)）
- **校正済みの閾値**: 人間とモデル 7 種の文書で誤検知率を確かめた語句と閾値だけを既定で有効にする。未校正のものは実験的ルールとして明示的に有効にしたときだけ動く
- **2 つのレーン**: AI 臭さ（`slop`）と読みやすさ（`readability`）の指摘を分けて出す。文書全体の点数は出さず、指摘とその件数だけを並べる
- **Markdown を理解する**: コードブロック・インラインコード・URL・文書の先頭の front matter（YAML の `---`・TOML の `+++`）を除き、見出し・リスト・表・引用を区別して解析する。文書の途中の `---` は区切り線か見出しの下線として読み、本文を捨てない
- **コードのコメントも読む**: コードのファイルは、tree-sitter で取り出したコメントだけを検査する。文字列の中の `//` やヒアドキュメントをコメントと取り違えず、指摘は元のファイルの行・列で示す（[コードのコメント](docs/code-comments.md#コードのコメント)）
- **括弧を考慮した文分割**: 形態素解析器 [hasami](https://github.com/owayo/hasami) の辞書を使わない文分割を使う。「」や（）の内側の句点では文を切らず、閉じ忘れた括弧があっても後続の文を巻き込まない。`Yahoo!ニュース` のように文末記号を含む語の途中でも切らない
- **判断を記録できる**: `<!-- noslop-disable-next-line P01 -- 引用のため -->` のように、残す理由を文書に書ける
- **CI 向けの出力**: text（色付き）・JSON（安定したスキーマ）・GitHub Actions の注釈に対応する。既定ではジョブを落とさない
- **AI エージェントに渡せる**: 直す箇所をルールごとにまとめた改稿指示を、Markdown・JSON・TOON（同じ内容を少ないトークンで表す形式）で出せる。MCP サーバー（`noslop mcp`）、Claude Code のフック（`noslop hook claude-code`。書いた直後、gws で Google ドキュメント・スプレッドシートに書き込む前、応答を終えたとき）、claw-hooks から呼ぶ入口（`noslop hook command` など）、スキル（`noslop skill-install`）で、書いた AI 自身に見直させる
- **改稿を比べる**: `noslop diff` で、改稿で新しく出た指摘・消えた数字や固有名詞・文書全体に一律に当てた直しを確かめる
- **手元のコーパスで校正できる**: `noslop calibrate` で、人の文書と生成文書からルールごとの誤検知率・検出率を測り、閾値を選ぶ

コメントを読めるコードの言語（拡張子と設定は [コードのコメント](docs/code-comments.md#コードのコメント) にあります）:

<p align="center">
  <img src="https://img.shields.io/badge/Rust-000000?logo=rust&amp;logoColor=white" alt="Rust">
  <img src="https://img.shields.io/badge/C-A8B9CC?logo=c&amp;logoColor=white" alt="C">
  <img src="https://img.shields.io/badge/C++-00599C?logo=cplusplus&amp;logoColor=white" alt="C++">
  <img src="https://img.shields.io/badge/Python-3776AB?logo=python&amp;logoColor=white" alt="Python">
  <img src="https://img.shields.io/badge/JavaScript-F7DF1E?logo=javascript&amp;logoColor=black" alt="JavaScript">
  <img src="https://img.shields.io/badge/TypeScript-3178C6?logo=typescript&amp;logoColor=white" alt="TypeScript">
  <img src="https://img.shields.io/badge/TSX-61DAFB?logo=react&amp;logoColor=black" alt="TSX">
  <img src="https://img.shields.io/badge/Go-00ADD8?logo=go&amp;logoColor=white" alt="Go">
  <img src="https://img.shields.io/badge/PHP-777BB4?logo=php&amp;logoColor=white" alt="PHP">
  <img src="https://img.shields.io/badge/Java-ED8B00?logo=openjdk&amp;logoColor=white" alt="Java">
  <img src="https://img.shields.io/badge/Kotlin-7F52FF?logo=kotlin&amp;logoColor=white" alt="Kotlin">
  <img src="https://img.shields.io/badge/Swift-F05138?logo=swift&amp;logoColor=white" alt="Swift">
  <img src="https://img.shields.io/badge/C%23-512BD4?logo=dotnet&amp;logoColor=white" alt="C#">
  <img src="https://img.shields.io/badge/Bash-4EAA25?logo=gnubash&amp;logoColor=white" alt="Bash">
  <img src="https://img.shields.io/badge/Ruby-CC342D?logo=ruby&amp;logoColor=white" alt="Ruby">
  <img src="https://img.shields.io/badge/Lua-2C2D72?logo=lua&amp;logoColor=white" alt="Lua">
  <img src="https://img.shields.io/badge/HTML-E34F26?logo=html5&amp;logoColor=white" alt="HTML">
  <img src="https://img.shields.io/badge/CSS-663399?logo=css&amp;logoColor=white" alt="CSS">
  <img src="https://img.shields.io/badge/YAML-CB171E?logo=yaml&amp;logoColor=white" alt="YAML">
  <img src="https://img.shields.io/badge/TOML-9C4121?logo=toml&amp;logoColor=white" alt="TOML">
</p>

## 動作環境

- **OS**: macOS、Linux、Windows
- **Rust**: 1.98 以上（ソースからビルドする場合）

## インストール

<!-- standard:install:start -->
### Homebrew (macOS/Linux)

```bash
brew install owayo/noslop/noslop
```

### Cargo

Rust 1.98 以上が必要です。

```bash
cargo install --git https://github.com/owayo/noslop --locked
```

### GitHub Releases から

[Releases](https://github.com/owayo/noslop/releases/latest) から自分の環境のアーカイブを取得して展開し、`noslop` を `PATH` の通った場所に置きます。各リリースには、取得したファイルを確かめるための `SHA256SUMS` も添付しています。

| プラットフォーム | ファイル |
|---|---|
| Linux (x86_64) | `noslop-x86_64-unknown-linux-gnu.tar.gz` |
| Linux (ARM64) | `noslop-aarch64-unknown-linux-gnu.tar.gz` |
| macOS (Intel) | `noslop-x86_64-apple-darwin.tar.gz` |
| macOS (Apple Silicon) | `noslop-aarch64-apple-darwin.tar.gz` |
| Windows (x86_64) | `noslop-x86_64-pc-windows-msvc.zip` |

macOS でブラウザから取得した場合は、実行の前に隔離属性を外します: `xattr -d com.apple.quarantine noslop`。

### ソースから

[mise](https://mise.jdx.dev/) が必要です (Rust のツールチェーンは `mise.toml` で固定しています)。

```bash
git clone https://github.com/owayo/noslop.git
cd noslop
make install
```

`make install` は `/usr/local/bin` に入れます。場所を変えるときは `INSTALL_PATH` を指定します (例: `make install INSTALL_PATH="$HOME/.local/bin"`)。
<!-- standard:install:end -->

アーカイブには `LICENSE` と `THIRD_PARTY_NOTICES.md` も含まれます。Homebrew での更新、スキルの導入先、ソースから入れる場合の指定は [インストールの補足](docs/installation.md) を参照してください。

ソースからの導入では、Claude Code と Codex CLI のスキルも `~/.claude/skills/noslop/` と `~/.codex/skills/noslop/` に入ります。`make uninstall` はスキルを残します。

## 使い方

ファイルを指定して検査します。ディレクトリを渡すと、その配下の Markdown とテキストをまとめて検査します（`.gitignore`・`.ignore`・`.noslopignore` を尊重します）。

```bash
noslop check README.md
noslop check docs --genre tech
```

改稿指示を AI や編集者に渡すなら `--report brief` を使います。JSON・TOON でも出せます。

```bash
noslop check README.md --report brief
noslop check README.md --report brief --format toon
```

改稿後は、指摘の変化に加えて、数字や固有名詞の消失、文書全体に一律に当てた直しを確認できます。

```bash
noslop diff draft-v1.md draft-v2.md
noslop explain R01
```

既定では指摘があっても終了コードは 0 です。指摘で CI を止める場合だけ `--fail-on warning` などを付けます。引数・設定・入出力の誤りは終了コード 2 です。フックの終了コードは呼び出す側の約束に合わせています。

| 調べたいこと | 文書 |
|---|---|
| 全コマンド・オプション・終了コード | [CLI リファレンス](docs/cli-reference.md) |
| ルールの ID・レーン・状態 | [ルール一覧](docs/rule-catalog.md)・[各ルールの説明](docs/rules.md) |
| コメントの対応言語と取り出し方 | [コードのコメント](docs/code-comments.md) |
| text・JSON・TOON・GitHub 注釈・改稿指示の例 | [出力形式](docs/output.md) |
| CI に組み込む例 | [GitHub Actions で使う](docs/github-actions.md) |
| スキル・MCP・フックの導入 | [AI エージェントへの導入](docs/agent-setup.md)・[連携の仕様](docs/integrations.md) |
| 改稿の確認事項と、点数を出さない理由 | [改稿の比較と指摘の読み方](docs/revision.md) |
| 辞書の取得・選択・辞書なしの動作 | [形態素解析の辞書](docs/dictionaries.md) |
| 校正の根拠と測り直しの手順 | [校正の考え方](docs/calibration-background.md)・[校正手順](docs/calibration.md) |

## 設定

ユーザーの設定は `~/.config/noslop/config.toml`、プロジェクトの設定は `noslop.toml`（または `.noslop.toml`）に置きます。既定値 → ユーザーの設定 → プロジェクトの設定 → CLI の順に、書いた項目を上書きします。

```bash
noslop init --user
noslop init
```

```toml
genre = "tech"

[morphology]
dictionary = "bundled"
```

`bundled` は、品詞で数えるルールを元の校正と同じ同梱の IPAdic で判定する指定です。既定の `auto` は、取得済みの配布辞書があればそちらを先に選びます。

重ね方・独自ルール・抑制コメント・ジャンルは [設定と抑制コメント](docs/configuration.md) に、全項目は [設定の例](examples/noslop.toml) にあります。

## 開発

<!-- standard:dev:start -->
[mise](https://mise.jdx.dev/) が必要です。ツールの版は `mise.toml` で固定しています。

```bash
make setup   # ツールチェーン (mise) と依存を取得する
make ci      # CI と同じ検査 (書き換えない)
```

| コマンド | 説明 |
|---|---|
| `make setup` | ツールチェーン (mise) と依存を取得する |
| `make build` | デバッグ版をビルドする |
| `make release` | リリース版をビルドする |
| `make run` | デバッグ版を実行する (引数は ARGS="...") |
| `make test` | テストを実行する |
| `make lint` | clippy を警告ゼロで通す |
| `make fmt` | コードを整形する (書き換える) |
| `make fmt-check` | 整形済みかを確かめる (書き換えない) |
| `make check` | 整形と静的検査 (書き換えない) |
| `make ci` | CI と同じ検査 (書き換えない) |
| `make install` | リリース版を INSTALL_PATH (既定 /usr/local/bin) に入れる |
| `make uninstall` | INSTALL_PATH から取り除く |
| `make clean` | ビルド成果物を消す |

`make` でターゲットの一覧を表示します。リリースは GitHub Actions で行います (**Actions → Release → Run workflow**)。
<!-- standard:dev:end -->

辞書を同梱しないビルドのテスト、ルール一覧の再生成、辞書目録の検証、リリースの仕組みとロードマップは [開発とリリース](docs/development.md) を参照してください。

## ライセンス

<!-- standard:license:start -->
[MIT](LICENSE)
<!-- standard:license:end -->

noslop のルール体系・語句カタログ・閾値の一部は、MIT ライセンスで公開されている日本語の文章作法プロジェクトに由来します。著作権表示とライセンス全文は [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) にあります。

文分割には、日本語の形態素解析器 [hasami](https://github.com/owayo/hasami)（MIT）の辞書を使わない文分割を使っています。hasami が組み込む例外表（文末記号を含む語の一覧）は、SudachiDict などの辞書データから抽出しています。出典と著作権表示は [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) にあります。

既定のバイナリ（feature `bundled-dict`）には、hasami が mecab-ipadic から作った形態素解析の辞書を同梱しています。mecab-ipadic のライセンス（NAIST-2003）の条文は [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) に、辞書の出所は [dict/README.md](dict/README.md) にあります。

見る観点の一部は、[textlint-rule-preset-ai-writing](https://github.com/textlint-ja/textlint-rule-preset-ai-writing)（MIT）も参考にしています。コードと語句の一覧は含みません。
