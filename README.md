<p align="center">
  <img src="docs/images/app.png" width="128" alt="noslop">
</p>

<h1 align="center">noslop</h1>

<p align="center">
  <strong>日本語の文章から「AI 臭さ」を機械的に拾う Linter</strong>
</p>

<h3 align="center">対応プラットフォーム</h3>

<p align="center">
  <img src="https://img.shields.io/badge/Linux-FCC624?logo=linux&amp;logoColor=black" alt="Linux">
  <img src="https://img.shields.io/badge/macOS-000000?logo=apple&amp;logoColor=white" alt="macOS">
  <img src="https://img.shields.io/badge/Windows-0078D6" alt="Windows">
</p>

<p align="center">
  <a href="https://github.com/owayo/noslop/actions/workflows/release.yml">
    <img alt="Release" src="https://github.com/owayo/noslop/actions/workflows/release.yml/badge.svg?branch=main">
  </a>
  <a href="https://github.com/owayo/noslop/actions/workflows/ci.yml">
    <img alt="CI" src="https://github.com/owayo/noslop/actions/workflows/ci.yml/badge.svg?branch=main">
  </a>
  <a href="https://github.com/owayo/noslop/releases/latest">
    <img alt="Version" src="https://img.shields.io/github/v/release/owayo/noslop">
  </a>
  <a href="LICENSE">
    <img alt="License" src="https://img.shields.io/github/license/owayo/noslop">
  </a>
</p>

---

## 概要

noslop は、LLM が書いた日本語（議事録・ブログ・技術文書）に出やすい癖を、決まった規則で指さす Linter です。`と言えるでしょう` のような定型句、`〜ではなく` の対比の繰り返し、文の長さがそろいすぎた単調なリズム、見出しと太字だけで組んだ教科書的な構成を検出します。

noslop は「この文章は AI が書いた」と判定する道具ではありません。書き手は自分の文章の癖に気づきにくく、長い文書を目で追うと必ず見落としが出ます。noslop は疑わしい箇所を漏れなく並べるところまでを受け持ち、直すか残すかは書き手が文脈で決めます。残すと決めた箇所には、理由を添えた抑制コメントを書けます。

文字種と語句のパターン、文長の統計で判定し、品詞で数えるルール（「の」の連鎖・連続漢字）は、同梱した形態素解析の辞書で判定します。単一のバイナリで動き、起動も速く、大きなリポジトリでもファイルを並列に処理します。

## 特徴

- **辞書を同梱**: 形態素解析器 [hasami](https://github.com/owayo/hasami) の IPAdic の辞書をバイナリに同梱し、「の」の連鎖（P16）と連続漢字（P15）を品詞で判定する。インストールも設定も要らない（[形態素解析の辞書](#形態素解析の辞書)）
- **校正済みの閾値**: 人間とモデル 7 種の文書で誤検知率を確かめた語句と閾値だけを既定で有効にする。未校正のものは実験的ルールとして明示的に有効にしたときだけ動く
- **2 つのレーン**: AI 臭さ（`slop`）と読みやすさ（`readability`）の指摘を分けて出す。文書全体の点数は出さず、指摘とその件数だけを並べる
- **Markdown を理解する**: コードブロック・インラインコード・URL・文書の先頭の front matter（YAML の `---`・TOML の `+++`）を除き、見出し・リスト・表・引用を区別して解析する。文書の途中の `---` は区切り線か見出しの下線として読み、本文を捨てない
- **コードのコメントも読む**: コードのファイルは、tree-sitter で取り出したコメントだけを検査する。文字列の中の `//` やヒアドキュメントをコメントと取り違えず、指摘は元のファイルの行・列で示す（[コードのコメント](#コードのコメント)）
- **括弧を考慮した文分割**: 形態素解析器 [hasami](https://github.com/owayo/hasami) の辞書を使わない文分割を使う。「」や（）の内側の句点では文を切らず、閉じ忘れた括弧があっても後続の文を巻き込まない。`Yahoo!ニュース` のように文末記号を含む語の途中でも切らない
- **判断を記録できる**: `<!-- noslop-disable-next-line P01 -- 引用のため -->` のように、残す理由を文書に書ける
- **CI 向けの出力**: text（色付き）・JSON（安定したスキーマ）・GitHub Actions の注釈に対応する。既定ではジョブを落とさない
- **AI エージェントに渡せる**: 直す箇所をルールごとにまとめた改稿指示を、Markdown・JSON・TOON（同じ内容を少ないトークンで表す形式）で出せる。MCP サーバー（`noslop mcp`）、Claude Code のフック（`noslop hook claude-code`。書いた直後、gws で Google ドキュメント・スプレッドシートに書き込む前、応答を終えたとき）、スキル（`noslop skill-install`）で、書いた AI 自身に見直させる
- **改稿を比べる**: `noslop diff` で、改稿で新しく出た指摘・消えた数字や固有名詞・文書全体に一律に当てた直しを確かめる
- **手元のコーパスで校正できる**: `noslop calibrate` で、人の文書と生成文書からルールごとの誤検知率・検出率を測り、閾値を選ぶ

コメントを読めるコードの言語（拡張子と設定は [コードのコメント](#コードのコメント) にあります）:

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

### Homebrew (macOS・Linux)

```bash
brew install owayo/noslop/noslop
```

tap ([owayo/homebrew-noslop](https://github.com/owayo/homebrew-noslop)) を足して、Releases のビルド済みのバイナリを入れます。更新は `brew upgrade noslop` です。Claude Code・Codex CLI のスキルは入らないので、使うなら `noslop skill-install claude` (Codex CLI なら `codex`) を実行します ([AI エージェントと使う](#ai-エージェントと使う))。

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

アーカイブには、バイナリのほかに `LICENSE` と `THIRD_PARTY_NOTICES.md`（同梱の辞書と、コードのコメントを読む文法のライセンスの表示）が入っています。

### cargo

```bash
cargo install --git https://github.com/owayo/noslop --locked
```

`--locked` を付けると、CI とリリースのビルドと同じく `Cargo.lock` のとおりに依存を解決します。

### ソースから

```bash
git clone https://github.com/owayo/noslop
cd noslop
make install   # /usr/local/bin にインストール（INSTALL_PATH で変更可）
```

Makefile は [mise](https://mise.jdx.dev/) で `mise.toml` の Rust を使います。mise を使わない場合は `make install SYSTEM_TOOLS=1` で、`PATH` 上の cargo でビルドします。

`make install` は、バイナリを入れたあと、そのバイナリで Claude Code と Codex CLI のスキル（`~/.claude/skills/noslop/SKILL.md`・`~/.codex/skills/noslop/SKILL.md`）も入れます（[AI エージェントと使う](#ai-エージェントと使う)）。入れる先は `SKILL_TARGETS` で選べます（`make install SKILL_TARGETS=claude`、入れないなら `make install SKILL_TARGETS=`）。`make uninstall` はバイナリだけを取り除き、スキルは残します。

## 使い方

### コマンド

| コマンド | 内容 |
|---------|------|
| `noslop check [PATH]...` | ファイルやディレクトリを検査する（別名 `lint`）。省略時はカレントディレクトリ |
| `noslop diff <BEFORE> <AFTER>` | 改稿の前後を比べる（新しく出た指摘・消えた事実・改稿の偏り） |
| `noslop rules` | ルールの一覧を表示する |
| `noslop explain <RULE>` | ルールの説明（何を見るか・なぜ問題か・直し方・例・根拠）を表示する |
| `noslop init` | プロジェクトの設定ファイル `noslop.toml` の雛形を作る。`--user` ならユーザーの設定 `~/.config/noslop/config.toml` の雛形を作る |
| `noslop mcp` | MCP サーバーとして標準入出力で待ち受ける（AI エージェントから検査を呼ぶ） |
| `noslop hook claude-code` | Claude Code の PostToolUse フックとして、書き換えたファイルの指摘を返す |
| `noslop hook file <PATH>` | 編集したファイルのパスだけを渡すフックの仕組み（claw-hooks など）から呼び、コミットしていない変更に重なる指摘をテキストで返す |
| `noslop skill-install <claude\|codex>` | Claude Code・Codex CLI に noslop のスキルを入れる |
| `noslop dict download [NAME]` | hasami の配布辞書（既定は `ipadic-neologd-sudachi`）を share ディレクトリに取得する。既定では圧縮版を取って展開し、大きさと SHA-256 を確かめてから置く。取得した辞書は、辞書を指定しないとき（`auto`）に使われる |
| `noslop dict list` | 配布辞書と取得済みかを表示し、辞書を指定しないときに使う辞書を示す（通信しない） |
| `noslop calibrate --human <PATH> --ai <PATH>` | 人の文書と生成文書のコーパスで、ルールの誤検知率・検出率と閾値を測る |

```bash
# 1 ファイルを検査する
noslop check docs/intro.md

# ディレクトリ配下の Markdown とテキストをまとめて検査する（.gitignore・.ignore・.noslopignore を尊重）
noslop check .

# 標準入力から読む（AI の下書きをそのまま流し込む用途）
pbpaste | noslop check - --stdin-filename draft.md

# AI や編集者に渡す改稿指示を作る
noslop check draft.md --format brief | pbcopy

# 改稿指示を JSON・TOON（少ないトークンで LLM に渡す）で出す
noslop check draft.md --report brief --format json
noslop check draft.md --report brief --format toon

# 技術文書の閾値で検査し、実験的ルールも有効にする
noslop check docs --genre tech --experimental

# 特定のルールを無視する
noslop check draft.md --ignore-rules P01,R03

# 改稿の前後を比べる（直前のコミットの版と比べるなら標準入力から渡す）
noslop diff draft-v1.md draft-v2.md
git show HEAD:docs/intro.md | noslop diff - docs/intro.md --stdin-filename docs/intro.md

# ルールの説明を読む
noslop explain R01
```

ディレクトリを渡したときは、拡張子 `md` / `markdown` / `txt` のファイルを対象にします（設定で変更できます）。ファイルを直接指定した場合は拡張子に関わらず検査します。渡したディレクトリから検査するファイルが 1 件も見つからなければ、標準エラーに警告を出します（`.gitignore` に `dir/**` のようなファイルに当たる行があると、渡したディレクトリの中身も除外されます。除外するなら `dir/` と書いてください）。

### `check` のオプション

| オプション | 短縮形 | 説明 |
|-----------|-------|------|
| `--report <KIND>` | | 出力する内容。`full`（全指摘のレポート。既定）/ `brief`（直す箇所をルールごとにまとめた改稿指示） |
| `--format <FORMAT>` | `-f` | 出力形式。全指摘のレポートは `text`（既定）/ `json` / `toon` / `github`（別名 `github-actions`）、改稿指示は `markdown`（既定）/ `json` / `toon`。`brief` は `--report brief --format markdown` の省略形 |
| `--genre <GENRE>` | | ジャンル。`general`（既定）/ `tech` / `business` / `essay`。別名 `blog`→essay、`minutes`→business |
| `--ignore-rules <IDS>` | | 無視するルール（カンマ区切り、ID か名前） |
| `--enable-rules <IDS>` | | 追加で有効にするルール（実験的ルールを個別に有効にする用途） |
| `--only-rules <IDS>` | | 指定したルールだけを動かす |
| `--experimental` | | 実験的なルールと語句をすべて有効にする |
| `--config <PATH>` | | プロジェクトの設定ファイルを指定する（ユーザーの設定は重ねて読む） |
| `--no-config` | | 設定ファイルを読まない（ユーザーの設定もプロジェクトの設定も） |
| `--fail-on <LEVEL>` | | この重大度以上の指摘があれば終了コード 1 にする。`never`（既定）/ `info` / `warning` / `error` |
| `--stdin-filename <NAME>` | | 標準入力（`-`）を読むときの表示名。拡張子で形式を決める |
| `--show-suppressed` | | 抑制コメントで残した指摘も表示する |
| `--no-readability` | | 読みやすさの指摘を出さない |
| `--include <KINDS>` | | 語句パターン系ルールをリスト・表・引用にも当てる。`lists` / `tables` / `quotes` / `all`（カンマ区切り） |
| `--line-breaks <MODE>` | | 段落内の改行の扱い。`space`（既定）/ `sentence` |
| `--dict <DICT>` | | 形態素解析の辞書を指定し、必ず使う。`auto`（share ディレクトリの一番良い辞書、なければ同梱の辞書）/ `bundled`（同梱の IPAdic）/ `share:<名前>` / ファイルのパス（hasami の `.hsd`）。詳細は[形態素解析の辞書](#形態素解析の辞書) |
| `--no-dict` | | 形態素解析の辞書を使わず、辞書なしの近似で判定する |
| `--color <WHEN>` | | 色付けの有無。`auto`（既定）/ `always` / `never`。`NO_COLOR` も尊重する |
| `--quiet` | `-q` | 指摘のないファイルとサマリを表示しない |
| `--brief-limit <N>` | | `brief` 形式で、1 ルールあたりに並べる箇所の上限（既定 5） |
| `--help` | `-h` | ヘルプを表示する |
| `--version` | `-V` | バージョンを表示する |

### `diff` のオプション

`noslop diff <BEFORE> <AFTER>` は、`check` のルールの選び方と文書の読み方のオプション（`--genre`・`--ignore-rules`・`--enable-rules`・`--only-rules`・`--experimental`・`--no-readability`・`--include`・`--line-breaks`・`--dict`・`--no-dict`・`--config`・`--no-config`）をそのまま受け付けます。どちらか一方は `-` で標準入力から読めます。

| オプション | 短縮形 | 説明 |
|-----------|-------|------|
| `--format <FORMAT>` | `-f` | 出力形式。`text`（既定）/ `json` / `toon` |
| `--stdin-filename <NAME>` | | 標準入力（`-`）を読むときの表示名。拡張子で形式を決める |
| `--color <WHEN>` | | 色付けの有無。`auto`（既定）/ `always` / `never` |

### `rules` / `explain` / `init`

| コマンド | オプション | 説明 |
|---------|-----------|------|
| `noslop rules` | `-f, --format <text\|json\|markdown>` | 一覧の形式。`markdown` は各ルールの説明文も含む（`docs/rules.md` の生成用） |
| | `--genre <GENRE>` / `--experimental` | そのジャンル・設定で既定で有効かの判定に使う |
| | `--config <PATH>` / `--no-config` | 有効・無効の列に設定ファイルを反映するか |
| `noslop explain <RULE>` | | ID か名前で、メタ情報・設定できる閾値の現在値・説明文を表示する |
| `noslop init` | `--user` / `--force` | `noslop.toml` のひな形をカレントディレクトリに作る。`--user` ならユーザーの設定のひな形を `~/.config/noslop/config.toml` に作る（ディレクトリがなければ作る）。既にあれば `--force` で上書き |
| `noslop mcp` | `--config <PATH>` / `--no-config` | 設定ファイルの指定。ツールと登録の仕方は [docs/integrations.md](docs/integrations.md) |
| `noslop hook claude-code` | `--brief-limit <N>` / `--include-readability` / `--experimental` / `--genre <GENRE>` / `--whole-file` | 返す箇所の上限（既定 3）、読みやすさの指摘を含めるか、変わった行に限らずファイル全体を見るか。詳細は [docs/integrations.md](docs/integrations.md) |
| `noslop hook file <PATH>` | `hook claude-code` と同じもの / `--max-chars <N>` | 出力の文字数の上限（既定 9000。超える分は行の単位で省く）。変わった行は git の HEAD との差分から求める。詳細は [docs/integrations.md](docs/integrations.md) |
| `noslop skill-install <claude\|codex>` | `--dir <DIR>` | スキルの置き場（既定は `~/.claude/skills` か `~/.codex/skills`。プロジェクトに置くなら `.claude/skills` など）。`noslop/SKILL.md` を書き、すでにあれば上書きする |
| `noslop dict download [NAME]` | `--dir <DIR>` / `--source <URL>` / `--uncompressed` / `--force` | NAME は `ipadic` / `ipadic-neologd` / `ipadic-neologd-sudachi`（既定）。保存先（既定は hasami の share ディレクトリ）、取得元の URL（ミラー用）、圧縮版を使わずに展開前の辞書を取るか（圧縮版を置いていないミラー用）、正しいファイルがあっても取り直すか（中身の違うファイルを置き換えるときにも要る）。詳細は[別の辞書を使う](#別の辞書を使う) |
| `noslop dict list` | `--dir <DIR>` | 取得済みかを確かめる場所（既定は share ディレクトリ） |
| `noslop calibrate` | `--human <PATH>` / `--ai <PATH>`（必須・繰り返し可）、`--genre`、`--target-fp`、`--holdout`、`--min-detection`、`--no-experimental`、`-f, --format <text\|json\|markdown>` | コーパスでの測り方。手順は [docs/calibration.md](docs/calibration.md) |

### 終了コード

| コード | 意味 |
|-------|------|
| `0` | 検査が終わった（指摘の有無は問わない。`diff` は確認事項があっても 0） |
| `1` | `--fail-on` で指定した重大度以上の指摘があった（`check` のみ） |
| `2` | 引数・設定・入出力のエラー（読めないファイルがあった場合も、読めたファイルの結果を出したうえで 2） |

既定の `--fail-on never` では、指摘があっても終了コードは 0 です。noslop は疑いを示す道具で、件数でビルドを止める設計にはしていません。

## ルール

ルールは 3 系統に分かれます。

| 系統 | ID | 見るもの |
|------|----|---------|
| 語句パターン | `P` | 文の中の定型句・言い回し |
| リズム・統計 | `R` | 文長のばらつき、文末・文頭の反復、段落の構成など、文や段落をまたいだ集計 |
| 構造 | `S` | 太字・箇条書き・見出しなど Markdown の体裁 |

各ルールはレーンとステータスを持ちます。

- **レーン** — `slop` は AI 臭さの検出です。`readability` は読みやすさの指摘（長すぎる一文、二重否定など）で、AI らしさとは無関係なので、AI 臭さとは分けて出します。設定ファイルの独自ルールは、既定で `custom` のレーンに入ります。出力では、それぞれを「AI 臭さ」「読みやすさ」「独自ルール」と表示します。
- **ステータス** — `stable` はコーパスで誤検知率を確かめたもので、既定で有効です。`experimental` は未校正か、辞書なしの近似で校正条件から外れるもので、`--experimental` か設定で有効にしたときだけ動きます。語句ルールは語句ごとにもステータスを持ちます。

| ID | 名前 | 内容 | レーン | ステータス |
|----|------|------|-------|-----------|
| P01 | `AI_CONCLUSION` | 結論の押し付け・まとめ口調 | slop | stable |
| P02 | `AI_PREFACE` | 定型の前置き・予告 | slop | stable |
| P03 | `AI_CONJUNCTION` | 空疎な接続 | slop | stable |
| P04 | `REDUNDANT_VERB` | 冗長な動詞表現 | readability | experimental |
| P05 | `REDUNDANT_MODIFIER` | ぼやけた修飾語 | slop | experimental |
| P06 | `OVER_EMPHASIS` | 過剰な強調 | slop | stable |
| P07 | `HEDGING` | 予防線・免責 | slop | stable |
| P08 | `EMPTY_ADJECTIVE` | 空虚な形容 | slop | stable |
| P09 | `EMPTY_VERB` | 作業を宣言するだけの動詞 | slop | stable |
| P10 | `DRAMATIC_CLOSER` | 演出的な決め文 | slop | experimental |
| P11 | `COUNT_DECLARATION` | プレゼン的な数の予告 | slop | experimental |
| P12 | `TRANSLATIONESE` | 翻訳調 | slop | stable |
| P13 | `INANIMATE_SUBJECT` | 無生物主語と他動詞 | slop | stable |
| P14 | `EM_DASH` | ダッシュの挿入句 | slop | experimental |
| P15 | `KANJI_RUN` | 連続漢字 | readability | stable |
| P16 | `NO_CHAIN` | 「の」の連鎖 | readability | stable |
| P17 | `DOUBLE_NEGATIVE` | 二重否定 | readability | stable |
| P18 | `CHAT_RESIDUE` | 「ご質問ありがとうございます」などチャット応答の名残 | slop | experimental |
| P19 | `HYPE` | 根拠のない誇張表現 | slop | experimental |
| P20 | `COLON_CONTINUATION` | 「以下の通りです：」のような述語とコロンでの列挙の導入 | slop | experimental |
| P21 | `VAGUE_ATTRIBUTION` | 「専門家は〜と指摘しています」などの出典をぼかした権威付け | slop | experimental |
| R01 | `LOW_BURSTINESS` | 文長の単調さ | slop | stable |
| R02 | `REPETITIVE_ENDING` | 文末の反復 | readability | experimental |
| R03 | `LONG_SENTENCE` | 長すぎる一文 | readability | stable |
| R04 | `BURIED_LIST` | 一文に埋もれた列挙 | readability | experimental |
| R05 | `ANTITHESIS_REPETITION` | 否定→肯定の対比の反復 | slop | stable |
| R06 | `NO_NOMINAL_ENDING` | 長い文書での体言止めの欠如 | slop | stable |
| R07 | `UNIFORM_PARAGRAPHS` | 段落の文数がそろいすぎている | slop | stable |
| R08 | `REPEATED_SENTENCE_LEAD` | 文頭の型の反復 | slop | experimental |
| R09 | `PARAGRAPH_LEAD_CONJUNCTION` | 段落頭の接続詞 | slop | experimental |
| R10 | `CLEFT_BECAUSE` | 「それは〜。なぜなら〜」構文 | slop | experimental |
| R11 | `SELF_ANSWER` | 自分で立てた問いに自分で答える | slop | experimental |
| R12 | `OVERCORRECTION` | 直しすぎの均一さ（長短の機械的な交互・体言止めの過多） | slop | experimental |
| R13 | `COMMA_PROFILE` | 読点を打つ癖（1 文あたりの読点の多さ） | slop | experimental |
| R14 | `DUPLICATE_PASSAGE` | 同じ文・段落の再登場（初出の行・列も表示） | readability | experimental |
| R15 | `FORMULAIC_FUTURE_CLOSER` | 文書末尾の「課題は残る → 今後に期待する」という結び | slop | experimental |
| R16 | `REPEATED_EVALUATIVE_TRIAD` | 短い評価語・抽象語の三項列挙の反復 | slop | experimental |
| S01 | `BOLD_DENSITY` | 太字の多用 | slop | experimental |
| S02 | `BULLET_RATIO` | 箇条書きへの偏り | slop | experimental |
| S03 | `BOILERPLATE_HEADING` | 「まとめ」「おわりに」などの定型見出し | slop | experimental |
| S04 | `NUMBERED_PHASES` | 「フェーズ1」などの番号付きの段階 | slop | experimental |
| S05 | `EMOJI_DENSITY` | 絵文字・装飾記号の多用 | slop | experimental |
| S06 | `BOLD_LABEL_LIST` | 「**項目**: 説明」の定型 | slop | experimental |
| S07 | `HEADING_TEMPLATE` | 見出しの型（「X: Y」・問い・番号）の反復 | slop | experimental |
| S08 | `HEADING_EMPHASIS` | 見出しの中の太字・絵文字 | slop | experimental |
| S09 | `STRUCTURE_DENSITY` | 見出しと箇条書きの密度 | slop | experimental |
| S10 | `LABEL_STYLE` | 段落や項目を絵文字・「ラベル：」で書き出す | slop | experimental |

最新の一覧は `noslop rules`、各ルールの詳細は `noslop explain <ID>` で確認できます。全ルールの説明 (何を見るか・なぜ問題か・直し方・例・根拠) は [docs/rules.md](docs/rules.md) にまとめてあります。ルールの ID は公開後に意味を変えません。

語句パターン系ルールが既定で見るのは地の文（段落）だけです。校正を地の文で行ったためで、リスト・表・引用も見るには設定の `[scope]` を変えます。リズム・統計系ルールは常に地の文だけで集計します。

## 設定

設定ファイルは 2 つあります。どちらも書き方は同じです。

| ファイル | 場所 | 向いている設定 |
|---|---|---|
| ユーザーの設定 | `~/.config/noslop/config.toml`（Windows は `%USERPROFILE%\.config\noslop\config.toml`） | 手元のマシンでだけ使うもの（辞書の選び方など）。どのディレクトリで実行しても読む |
| プロジェクトの設定 | `noslop.toml`（または `.noslop.toml`） | 文書に合わせるもの（ジャンル・ルール・独自ルールなど）。カレントディレクトリから親へ向かって探し、最初に見つかった 1 つを使う |

両方あるときは、ユーザーの設定にプロジェクトの設定を重ねます。優先するのは次の順で、下ほど強くなります。

1. 既定値
2. ユーザーの設定
3. プロジェクトの設定
4. コマンドラインのオプション

重ね方は次のとおりです。

- 上書きするのは、書いた項目だけです。プロジェクトの設定に書いていない項目は、ユーザーの設定の値が残ります。配列（`[files] extensions`・`exclude`、`[code] extensions`）は丸ごと置き換えます
- ルールの有効・無効（`[rules]` の `enable`・`disable` と `[rules.<ID>] enabled`）は、ルールごとに決めます。ID と名前のどちらで書いても同じルールとして扱います。プロジェクトの設定で触れたルールはそちらに従い、触れていないルールはユーザーの設定に従います。同じファイルの中では無効が優先です。ユーザーの設定で止めたルールをプロジェクトで動かすには、`enable` か `enabled = true` を書きます
- `[rules.<ID>]` の `severity` と閾値は、項目ごとにプロジェクトの設定が上書きします
- 独自ルール（`[[custom]]`）は両方を使います。同じ ID（大文字小文字は区別しない）があれば、プロジェクトの定義で丸ごと置き換えます
- `[files] exclude` の基準は、プロジェクトの設定なら設定ファイルのあるディレクトリ、ユーザーの設定なら `noslop check` に渡したディレクトリ（フック（`noslop hook`）ではエージェントの作業ディレクトリ）です。プロジェクトの設定に `exclude` があれば、ユーザーの設定の `exclude` は使いません（`exclude = []` で打ち消せます）
- `[morphology] dictionary` の相対パスは、書いたファイルのあるディレクトリが基準です（`~/` はホームディレクトリ）

`--config <PATH>` はプロジェクトの設定だけを差し替え、ユーザーの設定は重ねたまま読みます。`--no-config` はどちらも読みません。どちらかのファイルの書式が正しくなければ、設定の誤りとして終了コード 2 で止まります。ユーザーの設定は自動では作りません。`noslop init --user` でひな形を作れます（`--user` を付けない `noslop init` は、プロジェクトの `noslop.toml` を作ります）。

ユーザーの設定の例です。

```toml
# ~/.config/noslop/config.toml
[morphology]
dictionary = "share:ipadic-neologd-sudachi"   # 手元に取得した辞書を必ず使う

[rules]
disable = ["R03"]                              # 長い一文の指摘は出さない
```

プロジェクトの設定の例です。

```toml
genre = "tech"
fail_on = "never"

[files]
exclude = ["CHANGELOG.md", "vendor/**"]

[scope]
lists = true            # 箇条書きの項目にも語句パターン系ルールを当てる

[morphology]
mode = "off"            # 形態素解析の辞書を使わない (auto / required / off)

[rules.P04]
enabled = true          # 実験的ルールを個別に有効にする

[rules.R03]
severity = "warning"    # 重大度を変える
max_chars = 100         # 閾値を変える

[[custom]]              # チーム独自の禁止語
id = "X01"
name = "BANNED_HONORIFIC"
pattern = "ユーザー様"
message = "「ユーザー様」ではなく「利用者」と書く"
severity = "warning"
```

すべての項目とその説明は [examples/noslop.toml](examples/noslop.toml) にあります。閾値は校正の前提を崩すので、変えるときは理由を残してください。

## コードのコメント

コードのファイルでは、コメントだけを文章として検査します（文字列リテラルは読みません）。

```bash
# 直接渡したコードのファイルは、拡張子から言語を決めてコメントを検査する
noslop check src/lib.rs

# ディレクトリをたどるときは、設定の [code] extensions に書いた拡張子のファイルも集める
noslop check src/
```

```toml
# noslop.toml か ~/.config/noslop/config.toml
[code]
extensions = ["rs", "ts", "tsx", "py", "go", "sh"]
```

- **読める言語**: Rust・C・C++・Python・JavaScript・TypeScript・TSX・Go・PHP・Java・Kotlin・Swift・C#・Bash・Ruby・Lua・HTML・CSS・YAML・TOML です。どのバイナリ（Homebrew・Releases・ソースからのビルド）でも、すべての言語を読めます。`[code] extensions` にコードの拡張子でないものを書くと、設定の誤りになります
- **取り出し方**: tree-sitter でコメントを読むので、文字列の中の `//`、正規表現、ヒアドキュメントをコメントと取り違えません。隣り合う行の同じ種類のコメント（同じ記号・同じ列）は 1 つにまとめ、コードの後ろのコメントは単独で読みます。記号（`//`・`#`・`/*`・行頭の `*` など）を外した本文を段落にし、指摘の位置は元のファイルの行・列で示します。コメントは句点を打たずに 1 行に 1 つのことを書くことが多いので、行の終わりは文の区切りとして読みます。前の行が読点・助詞・開き括弧で終わるときと、次の行が閉じ括弧・読点で始まるときだけ、1 つの文につなぎます
- **ドキュメントのコメント**: `///`・`/** */`・Python の docstring は Markdown として読み、インラインコードやコードブロックは検査しません。Javadoc と C# の XML ドキュメントは、タグを外して読みます
- **検査しないコメント**: 日本語を含まないもの、shebang、ツールへの指示（`eslint-disable`・`noqa` など）、著作権・ライセンスの表記
- **当てるルール**: コメントは互いに独立した短い断片なので、1 文ずつ判定するルール（語句ルール・独自ルール・R03・R04）だけを当てます。文書・段落をまたいで数えるルール（R01 など）と構造のルール（S）は当てません。ルールはひと続きの文章で校正したもので、コメントでの誤検知率は確かめていません
- **抑制**: コメントの中身全体を `noslop-disable-next-line P01 -- 理由` の形（`<!-- -->` で囲んでもかまいません）にすると、抑制コメントとして読みます。文の途中に書いた記法は抑制になりません
- **出力**: JSON の `format` は `"code"` になります。`--stdin-filename` と MCP の `check` の `filename` でも、拡張子から言語を決めます

```rust
// noslop-disable-next-line P01 -- 利用者に見せる案内の定型文のため
/// まとめると、ユーザーの設定にプロジェクトの設定を重ねたものを返します。
```

## 抑制コメント

指摘を読んだうえで「この文脈では直さない」と決めたら、その判断を文書に書き残します。Markdown とテキストのどちらでも HTML コメントとして書けます。

| 書き方 | 効く範囲 |
|-------|---------|
| `<!-- noslop-disable-next-line P01 -- 理由 -->` | コメントの次の行 |
| `<!-- noslop-disable-line R03 -- 理由 -->` | コメントのある行 |
| `<!-- noslop-disable P05 -- 理由 -->` 〜 `<!-- noslop-enable P05 -->` | 2 つのコメントの間（`enable` がなければ文書末まで） |
| `<!-- noslop-disable-file R01 -- 理由 -->` | 文書全体 |

- ルールは ID か名前で、カンマか空白で区切って複数書けます。省略すると全ルールが対象です。
- `--` のあとに理由を書きます。理由は JSON 出力の `suppressed.reason` に残ります。
- 存在しないルールを書くと警告を出します。
- コードブロックの中のコメントは抑制として扱いません（記法の説明を書けるように）。

```markdown
<!-- noslop-disable-next-line P01 -- 引用した発言なので原文のまま残す -->
> 「結論として、この方式が最適と言えるでしょう」と担当者は述べた。
```

理由のない抑制は判断を放棄したのと同じです。「固有名詞の一部」「引用」「ジャンル上自然」のように、短くても理由を書いてください。

## 出力形式

### text（既定）

```text
📄 docs/meeting.md
  AI 臭さの指摘 2 件
  3:1  情報  [AI 臭さ]  P03 AI_CONJUNCTION
    「このように」は前の内容を機械的に束ねる接続です（人間の文章にもよく出るため、弱い手掛かりとして扱っています）
    │ このように、定例会議を減らしたことはチーム全体にとって良い変化だったと言えるだろう。
    │ ^^^^^^^^^^
    💡 接続語に頼らず、前の内容の具体的な事実や疑問から次の話へつないでください
  3:35  警告  [AI 臭さ]  P01 AI_CONCLUSION
    「と言えるだろう」は結論を定型句で押し付ける締めです
    │ このように、定例会議を減らしたことはチーム全体にとって良い変化だったと言えるだろう。
    │                                                                     ^^^^^^^^^^^^^^
    💡 定型句を外して言い切るか、結論を支える事実や数値を書いてください

✖ AI 臭さの指摘 2 件 (警告 1・情報 1) — 1 ファイルを検査、辞書あり (同梱の ipadic)
```

行・列はどちらも 1 始まりで、列は文字数で数えます。

- 指摘はファイルごとに、レーンの節（AI 臭さ → 独自ルール → 読みやすさ）に分けて並びます。節の中は文書の順です。指摘のないレーンの節は出しません
- 各指摘の重大度の隣に、レーン名（`[AI 臭さ]`・`[独自ルール]`・`[読みやすさ]`）が付きます。実験的な項目の指摘には、末尾に `[実験的]` が付きます
- 最後の要約は、節の見出しと同じ呼び名で、レーンごとに件数と重大度の内訳を数えます。AI 臭さは 0 件でも出し、独自ルールと読みやすさは指摘があるときだけ出します
- 要約の先頭の ✖ は、AI 臭さか独自ルールの指摘があること、または `--fail-on` に当たったことを示します。読みやすさの指摘だけなら ✔ です
- 文書全体の点数は出しません（[文書全体の点数を出さない理由](#文書全体の点数を出さない理由)）

読みやすさの指摘があると、AI 臭さの節のあとに読みやすさの節が続きます。次の例では、読みやすさの指摘（3 行目）のほうが文書の中では先にありますが、節は AI 臭さを先に並べます。

```text
📄 docs/plan.md
  AI 臭さの指摘 1 件
  5:26  警告  [AI 臭さ]  P01 AI_CONCLUSION
    「と言えるだろう」は結論を定型句で押し付ける締めです
    │ 会議を減らしたことは、チームにとって良い変化だったと言えるだろう。
    │                                                   ^^^^^^^^^^^^^^
    💡 定型句を外して言い切るか、結論を支える事実や数値を書いてください

  読みやすさの指摘 1 件
  3:4  情報  [読みやすさ]  P15 KANJI_RUN
    漢字が 8 字続いています (「全社業務改善計画」)
    │ 来期は全社業務改善計画に沿って、会議の数を半分にします。
    │       ^^^^^^^^^^^^^^^^
    💡 語の切れ目が読み取れるか確かめ、助詞や動詞を補って開いてください

✖ AI 臭さの指摘 1 件 (警告 1)、読みやすさの指摘 1 件 (情報 1) — 1 ファイルを検査、辞書あり (同梱の ipadic)
```

### json

機械処理向けの安定したスキーマで出力します。要点は次のとおりです（下の例は、上の text 出力と同じ文書の JSON から P01 の指摘だけを抜き出したものです）。

- トップレベルに `schemaVersion`・`tool`（`name`・`version`）・`columnUnit`・`settings`（`genre`・`experimental`・`failOn`・`morphology`）・`files`・`summary`・`errors`
- `settings.morphology` は判定の方式で、`requested`（`auto` / `required` / `off`）・`method`（`dictionary` / `surface`）・`dictionary`（使った辞書の `name`・`source`（`bundled`: 同梱 / `file`: ファイルの辞書。指定したファイルと、`auto` で share ディレクトリから選んだ辞書）・`path`（同梱なら `null`）。辞書なしなら `null`）・`reason`（辞書を使わなかった理由: `disabled` / `not-found` / `not-needed`）を持つ
- 各ファイルに `path`・`format`（`markdown` / `text`）・`characters`・`sentences`・`counts`（抑制していない指摘を、レーン（`slop`・`readability`・`custom`）ごと・重大度（`error`・`warning`・`info`）ごとに数えたもの。0 件も常に出る）・`diagnostics`・`warnings`
- スキーマの版（`schemaVersion`）は 2 です。版 1 にあった文書全体の点数（`score`）は、版 2 で外して `counts` に置き換えました（[文書全体の点数を出さない理由](#文書全体の点数を出さない理由)）
- 各指摘に `ruleId`・`ruleName`・`severity`・`lane`・`status`・`message`・`hint`・`range`・`context`・`excerpt`・`related`・`metrics`・`fingerprint`・`suppressed`
- `range` と `context` は `start` と `end` を持ち、それぞれ `line`・`column`（1 始まり。`columnUnit` のとおり Unicode のスカラー値で数える）と `offset`（UTF-8 のバイト位置）を持つ
- `metrics` にはルールごとの値が入る。語句ルールは、一致した辞書の項目を `item` (正規表現の項目は `/パターン/` の形) に、一致した文字列を `matched` に入れる
- `fingerprint` は行番号に依存しない 16 桁の識別子で、前回の結果との突き合わせに使える。同じ文が繰り返されると同じ値になるので、突き合わせは多重集合で行う
- 抑制した指摘も消さずに出し、`suppressed` に `reason`（理由）と `line`（抑制コメントの行）を残す
- 読めなかったファイルは `errors` に `path` と `message` で入る
- 配列はパス・位置・ルール ID の順に並ぶ

```json
{
  "schemaVersion": 2,
  "tool": { "name": "noslop", "version": "26.9.100" },
  "columnUnit": "unicode-scalar",
  "settings": { "genre": "general", "experimental": false, "failOn": "never" },
  "files": [
    {
      "path": "docs/meeting.md",
      "format": "markdown",
      "characters": 56,
      "sentences": 2,
      "counts": {
        "slop": { "error": 0, "warning": 1, "info": 1 },
        "readability": { "error": 0, "warning": 0, "info": 0 },
        "custom": { "error": 0, "warning": 0, "info": 0 }
      },
      "diagnostics": [
        {
          "ruleId": "P01",
          "ruleName": "AI_CONCLUSION",
          "severity": "warning",
          "lane": "slop",
          "status": "stable",
          "message": "「と言えるだろう」は結論を定型句で押し付ける締めです",
          "hint": "定型句を外して言い切るか、結論を支える事実や数値を書いてください",
          "range": {
            "start": { "line": 3, "column": 35, "offset": 133 },
            "end": { "line": 3, "column": 42, "offset": 154 }
          },
          "context": {
            "start": { "line": 3, "column": 1, "offset": 31 },
            "end": { "line": 3, "column": 43, "offset": 157 }
          },
          "excerpt": "このように、定例会議を減らしたことはチーム全体にとって良い変化だったと言えるだろう。",
          "related": [],
          "metrics": { "item": "と言えるだろう", "matched": "と言えるだろう" },
          "fingerprint": "bd78baa3ef438ee5",
          "suppressed": null
        }
      ],
      "warnings": []
    }
  ],
  "summary": {
    "files": 1,
    "filesWithDiagnostics": 1,
    "diagnostics": 2,
    "bySeverity": { "error": 0, "warning": 1, "info": 1 },
    "byLane": { "slop": 2, "readability": 0, "custom": 0 },
    "suppressed": 0,
    "errors": 0
  },
  "errors": []
}
```

### github

GitHub Actions のワークフローコマンドとして出力し、プルリクエストの差分に注釈を付けます。重大度は `error`→`error`、`warning`→`warning`、`info`→`notice` に対応します。注釈のタイトルには、ルールの ID と名前の前にレーン名（AI 臭さ・読みやすさ・独自ルール）が入ります。抑制した指摘は出しません。

```text
::warning file=docs/meeting.md,line=3,col=35,endLine=3,endColumn=42,title=[AI 臭さ] P01 AI_CONCLUSION::「と言えるだろう」は結論を定型句で押し付ける締めです%0A💡 定型句を外して言い切るか、結論を支える事実や数値を書いてください
```

### brief

AI エージェントや編集者に渡す改稿指示を Markdown で出します。先頭に改稿のルール（主張・数字・固有名詞を変えない、原文にない事実を足さない、同じ直しを一律に当てない、指摘は残してよい、件数を減らすことを目的にしない、再実行は 1 回だけ）を置き、ルールごとに「なぜ疑わしいか」「直し方の方向」「該当箇所」をまとめます。語句の置き換え方は指示しません。

```markdown
## docs/meeting.md

- 指摘: AI 臭さ (校正済み) 2 件 / AI 臭さ (実験的) 0 件 / 独自ルール 0 件 / 読みやすさ 0 件

### 優先して見る箇所

#### 1. P01 AI_CONCLUSION — 結論の押し付け・まとめ口調 (AI 臭さ・警告 1 件)

- なぜ疑わしいか: 要約の定型として大量に学習された言い回しで、生成された文章ほど段落の終わりに現れます。…
- 直し方の方向: 定型句を外して言い切るか、結論を支える事実や数値を書いてください
- 該当箇所:
  - L3: 「と言えるだろう」は結論を定型句で押し付ける締めです
    - 原文: このように、定例会議を減らしたことはチーム全体にとって良い変化だったと言えるだろう。
```

出力の全体の構成は [docs/integrations.md](docs/integrations.md) にあります。

### 改稿指示の JSON / TOON（`--report brief --format json|toon`）

改稿指示と同じ内容を、プログラムで扱う JSON と、LLM に少ないトークンで渡す [TOON](https://github.com/toon-format/spec)（Token-Oriented Object Notation）で出します。どちらも同じデータで、ファイルごとに 2 つの表を持ちます。

- `rules` — 指摘のあったルール。優先して見る順に並び、`ruleId`・`ruleName`・`title`・`lane`・`maxSeverity`・`experimentalOnly`・`count`・`omittedCount`（`--brief-limit` を超えて載せなかった件数）・`why`（なぜ疑わしいか）・`hint`（直し方の方向）を持つ
- `occurrences` — 該当箇所。`ruleId` で `rules` を参照し、`line`・`column`（1 始まり、列は文字数）・`message`・`excerpt`（指摘を含む文の抜粋。文を持たない指摘は `null`）を持つ

ほかに `settings`（`genre`・`experimental`・`method`（判定の方式。`dictionary` / `surface`）・`dictionary`（使った辞書の名前。手元のパスは載せない））・`revisionRules`（改稿のルール）・`editorialQuestions`（編集の問い）・`note`（指摘がないときの断り書き）・`cleanFiles`・`warnings`・`errors` があります。metrics・fingerprint・抑制した指摘は入れません（指摘の追跡には全指摘のレポートを使います）。スキーマの版は `schemaVersion`、種類は `kind: brief` です。

```toon
files[1]:
  - path: docs/meeting.md
    counts:
      stableSlop: 2
      experimentalSlop: 0
      custom: 0
      readability: 0
    rules[2]{ruleId,ruleName,title,lane,maxSeverity,experimentalOnly,count,omittedCount,why,hint}:
      P01,AI_CONCLUSION,結論の押し付け・まとめ口調,slop,warning,false,1,0,要約の定型として…,定型句を外して言い切るか、結論を支える事実や数値を書いてください
      P03,AI_CONJUNCTION,空疎な接続,slop,info,false,1,0,前の段落で何が述べられたかに…,接続語に頼らず、前の内容の具体的な事実や疑問から次の話へつないでください
    occurrences[2]{ruleId,line,column,message,excerpt}:
      P01,3,35,「と言えるだろう」は結論を定型句で押し付ける締めです,このように、定例会議を減らしたことは…と言えるだろう。
      P03,3,1,「このように」は前の内容を機械的に束ねる接続です…,このように、定例会議を減らしたことは…と言えるだろう。
```

TOON は、ルールと該当箇所を 1 行 1 要素の表にするぶん短くなります。同梱の [examples/ai-smelly.md](examples/ai-smelly.md) の改稿指示では、トークン数（`o200k_base`）が TOON 2,770・Markdown 2,980・JSON 3,137（改行なし）/ 3,802（整形）でした。

### toon

全指摘のレポート（`--format json` と同じデータ）と `noslop diff` の結果を TOON で出します（`--format toon`）。整形した JSON より 2〜3 割短くなりますが、指摘ごとに `metrics` の項目が違うなど表にできない部分が多いため、改行なしの JSON よりは長くなります。LLM に指摘を渡すなら、改稿指示の TOON（`--report brief --format toon`）を使ってください。

TOON の符号化は公式の Rust 実装 [toon-format](https://github.com/toon-format/toon-rust)（仕様 v3.0）で行い、末尾に改行は付けません。出力は、仕様 v4.1 のリファレンス実装（`@toon-format/toon` 4.1.1）の strict モードで JSON と同じデータに戻ることを確かめています。

## GitHub Actions で使う

既定では注釈を付けるだけで、ジョブは成功します。指摘でジョブを落としたい場合だけ `--fail-on warning` などを付けてください。

```yaml
name: noslop

on:
  pull_request:

permissions:
  contents: read

jobs:
  noslop:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v7
      - name: Install noslop
        env:
          NOSLOP_VERSION: v26.9.106   # 使うリリースのタグに置き換える
        run: |
          base="https://github.com/owayo/noslop/releases/download/${NOSLOP_VERSION}"
          asset="noslop-x86_64-unknown-linux-gnu.tar.gz"
          curl -fsSL -o "${asset}" "${base}/${asset}"
          curl -fsSL -o SHA256SUMS "${base}/SHA256SUMS"
          grep " ${asset}\$" SHA256SUMS | sha256sum --check --strict -
          mkdir -p "$HOME/.local/bin"
          tar -xzf "${asset}" -C "$HOME/.local/bin" noslop
          echo "$HOME/.local/bin" >> "$GITHUB_PATH"
      - name: Lint Japanese prose
        run: noslop check docs --format github
```

リリースのタグを固定し、チェックサムを確かめてから使ってください。`releases/latest` から取る形にすると、使う版が知らないうちに変わります。v26.9.105 までのリリースの添付は、生のバイナリ（`noslop-linux-amd64` など）です。

## AI エージェントと使う

文章を書いた AI エージェントに、noslop の指摘をそのまま返せます。どの方法でも渡すのは「どこを、なぜ見直すか」と改稿の制約で、件数を減らすこと自体を目的にしないよう断っています。

| 方法 | 使いどころ |
|------|-----------|
| `noslop check --format brief` | 検査結果を AI や編集者に貼り付けて直してもらう（JSON・TOON なら `--report brief --format json\|toon`） |
| `noslop skill-install <claude\|codex>` | エージェントに、日本語の文章を書いた・直した後に noslop で見直す手順（スキル）を覚えさせる |
| `noslop mcp` | Claude Code・Codex CLI などのエージェントが、自分で検査（`check`）・改稿の前後の比較（`diff`）・ルールの説明（`explain`）・一覧（`rules`）を呼ぶ。`check` は改稿指示を Markdown（既定）・JSON・TOON で返す |
| `noslop hook claude-code` | Claude Code がファイルを書いた直後（変わった行）、gws で Google ドキュメント・スプレッドシートに書き込む前（書き込む値）、応答を終えたとき（リポジトリのコミットしていない変更）に、指摘を自動で渡す |
| `noslop hook file <PATH>`・`noslop hook git-diff` | フックの入力を渡せない仕組み（claw-hooks の extension_hooks・stop_hooks など）から、同じ指摘をテキストで渡す |

```bash
# スキルを入れる（~/.claude/skills/noslop/SKILL.md、~/.codex/skills/noslop/SKILL.md）
noslop skill-install claude
noslop skill-install codex

# MCP サーバーを登録する
claude mcp add noslop -- noslop mcp
codex mcp add noslop -- noslop mcp
```

スキルを入れると、「この文章を自然な日本語に推敲して」「AI っぽさを抜いて」のような依頼で、エージェントが `noslop check --report brief --format toon` で直す箇所を受け取り、直した後に `noslop diff` で確かめるようになります。スキルの本文は [skills/SKILL.md](skills/SKILL.md) です。

フックは `.claude/settings.json` などに書きます（[examples/claude-code-settings.json](examples/claude-code-settings.json)）。

```json
{
  "hooks": {
    "PostToolUse": [
      {
        "matcher": "Write|Edit|MultiEdit",
        "hooks": [{ "type": "command", "command": "noslop hook claude-code", "timeout": 30 }]
      }
    ],
    "PreToolUse": [
      {
        "matcher": "Bash",
        "hooks": [{ "type": "command", "command": "noslop hook claude-code", "timeout": 30 }]
      }
    ],
    "Stop": [
      { "hooks": [{ "type": "command", "command": "noslop hook claude-code", "timeout": 60 }] }
    ]
  }
}
```

- **PostToolUse**: 書き換えたファイルのうち、今回変わった行に重なる指摘だけを返します
- **PreToolUse (Bash)**: gws で Google ドキュメント・スプレッドシートに書き込む値を、書き込む前に検査します。ドキュメントの本文に指摘があれば 1 度だけ書き込みを止め、直すか、残すと決めて同じコマンドを打ち直すかを Claude に任せます。セルのような短い値は止めずに、指摘を添えます
- **Stop**: 作業ディレクトリを含む git の作業ツリーの、コミットしていない変更（HEAD との差分と追跡していないファイル）の変わった行に重なる指摘を返します。Bash で書き換えたファイルのように、PostToolUse を通らなかった変更も拾えます

対応する MCP の版、フックが返す範囲と上限、claw-hooks から呼ぶ設定などの詳細は [docs/integrations.md](docs/integrations.md) にあります。

## 改稿を比べる

`noslop diff <BEFORE> <AFTER>` は、同じ設定で改稿の前後を検査し、次の 3 つを確認事項として並べます。改稿を合否で判定するものではなく、指摘の件数を減らすための書き直しが別の問題を生んでいないかを、書き手が見直すための一覧です。前後の件数は、レーンごと（「AI 臭さの指摘 3 → 1 件」）に並べます。

- **指摘の変化** — 改稿後に新しく出た指摘、文を書き換えても同じ語句で残った指摘、継続・解消した指摘、抑制コメントを付けて残した指摘。行番号に依存しない `fingerprint` で突き合わせます
- **事実の変化** — 数字・日付・固有名詞らしい語・引用・URL の消失と追加。改稿前になかった数値には、出典のない数字を足していないか確かめるよう注意を付けます
- **改稿の偏り** — 読点の数や体言止めの比率の大きな変化、見出しの型の統一、箇条書きと地の文の一括の入れ替え、長短の機械的な交互など、同じ変換を文書全体に一律に当てた形跡

JSON（`--format json`）と TOON（`--format toon`）では、確認事項があるかを `hasConcerns` で返します。終了コードは確認事項があっても 0 です。

## 文書全体の点数を出さない理由

noslop は、指摘を 1 件ずつ並べ、レーンと重大度ごとの件数を数えるだけで、文書全体の点数（「自然度 80/100」のような値）は出しません。以前は、AI 臭さの指摘の件数を文字数で割り戻した 0〜100 の「自然度スコア」を出していましたが、次の理由で外しました。

- ルールごとの校正（人の文書での誤検知率）は、1 件ずつの指摘が当たっているかを確かめるもので、指摘を足し合わせた値が文書の出どころを見分けることまでは保証しない
- 実際に測ると見分けていなかった。出どころの分かる人の文書 29 本は 92〜100 点、AI が書いた文書 20 本は 98〜100 点（うち 17 本が 100 点）で、人の書籍のほうが AI のレポートより指摘の密度が高かった
- 長い文書は指摘があってもほぼ 100 点になり、「自然」という判定の札が安心材料として誤解を招いた

直すかどうかは、件数ではなく、1 件ずつの指摘を文脈に照らして決めてください。文書どうしを比べたいときは、JSON の `counts` を使います。

## ジャンル

ジャンルによって、人間の書き手が正当に使う慣習が違います。`--genre` か設定の `genre` で指定すると、閾値が切り替わり、慣習と衝突するルールが止まります。

| ジャンル | 別名 | 主な違い |
|---------|------|---------|
| `general` | | 既定。どのジャンルにも偏らない保守的な閾値 |
| `tech` | | 対比の反復を `error` にする比率を 3% から 4.5% に緩める。体言止めの欠如は 3000 字以上で判定。文頭の反復は 7 回以上 |
| `business` | `minutes` | 太字・箇条書き・定型見出し・番号付きの段階・見出しの型・構造の密度・ラベルでの書き出し（S01〜S04・S07・S09・S10）を止める。体言止めの欠如は 3000 字以上。文頭の反復は 7 回以上 |
| `essay` | `blog` | 長すぎる一文の目安を 90 字から 110 字に緩める。体言止めの欠如は 1500 字以上で判定。文頭の反復は 5 回以上 |

## 形態素解析の辞書

noslop は形態素解析器 [hasami](https://github.com/owayo/hasami) の IPAdic の辞書（`dict/ipadic.hsd`）をバイナリに同梱していて、何も指定しなくても、品詞で数えるルールを辞書で判定します。hasami の share ディレクトリに配布辞書を取得してあれば、そちらを使います（[使い方の指定](#使い方の指定)）。同梱の辞書で判定すれば、元の校正と同じ条件になります。

| ルール | 辞書ありの判定（既定） | 辞書なしの判定（`--no-dict`） |
|---|---|---|
| P16「の」の連鎖 | 連体の「の」を数え、隣り合う「の」の間が 2 語以内なら続いているとみなす。辞書が細かく分ける語（「必要性」「話し方」のような接尾辞の付いた語・数・カタカナ語）は 1 語にまとめて数える | 漢字・カタカナ・英数字の語に挟まれた「の」だけを数える。ひらがなを含む語をはさむ連鎖（「の家の大きな犬の」）や、端の語がひらがなの連鎖（「魂の安静のため」）は拾えない |
| P15 連続漢字 | 固有名詞を含む連なり（年号・人名など）を品詞で除く | 定番の接尾辞（〜委員会・株式会社など）で終わる連なりだけを除く |

形態素解析で数える元の検出器と、手元の文書 373 本で比べた結果です。

| | 辞書なし | 辞書あり（ipadic） |
|---|---:|---:|
| P16 元が指した箇所を拾えた割合 | 38% | 93% |
| P15 指した箇所の一致率 | 0.76 | 0.90 |

食い違った箇所を 1 件ずつルールの定義に照らして確かめると、辞書ありの判定の精度（指摘のうち正しいものの割合）と再現率（両方の正しい指摘のうち拾えた割合）は次のとおりでした。

| | noslop（辞書あり） | 元の検出器 |
|---|---|---|
| P16 精度・再現率 | 0.99・1.00 | 0.95・0.83 |
| P15 精度・再現率 | 0.88・0.96 | 0.86・0.97 |

ほかのルールは辞書の有無で変わりません。文分割も辞書を使いません。体言止め（R06）は、文書単位の判定が辞書なし・ありで変わらなかったため、辞書なしの推定のままです。

同梱の辞書はバイナリを約 18MB 大きくします（バイナリは、コードのコメントを読む 20 言語の文法も含めて約 53MB）。辞書はバイナリに埋め込んだまま複製せずに読むので、読み込みにはほとんど時間がかからず、メモリに載るのも解析で触れた部分だけです。手元の計測（Apple Silicon の macOS）で辞書なし（`--no-dict`）と比べると、短い文書 1 本の検査で増えたのは実時間 1ms 未満・最大 RSS 1MB 未満でした。約 50KB の文書では、実時間が約 4ms、最大 RSS が約 11MB 増えました。辞書を使うルールが動かない実行（`--no-readability` や、`--only-rules` で P15・P16 を外したとき）では、辞書を探しも読みもしません。share ディレクトリやファイルの辞書は mmap で読むので、200MB を超える辞書でも、メモリに載るのは解析で触れた部分だけです。

### 使い方の指定

使う辞書は `[morphology] dictionary`（または `--dict`）で、辞書を使うかどうかは `[morphology] mode`（または `--no-dict`）で決めます。

| 指定 | 動き |
|---|---|
| なにも指定しない（`dictionary = "auto"`） | `HASAMI_DICT` で指定した辞書 → share ディレクトリの配布辞書（hasami の推奨順。`ipadic-neologd-sudachi` → `ipadic-neologd` → `ipadic`）→ 同梱の辞書、の順に探し、最初に見つかったものを使う |
| `dictionary = "bundled"` / `--dict bundled` | 同梱の IPAdic を使う（元の校正と同じ条件）。`HASAMI_DICT` と share ディレクトリの辞書は見ない |
| `dictionary = "share:<名前>"` / `--dict share:<名前>` | hasami の share ディレクトリ（既定は `~/.local/share/hasami`）の `<名前>.hsd` を使う。`noslop dict download` で取得した辞書を指す（`HASAMI_DICT` には書けない） |
| `dictionary = "<パス>"` / `--dict <パス>` | この辞書（hasami の `.hsd`）を使う。設定ファイルの相対パスは設定ファイルのディレクトリが基準。`auto`・`bundled` という名前のファイルは `./auto` のように書く |
| `mode = "off"` / `--no-dict` | 辞書を使わず、辞書なしの近似で判定する |
| `mode = "required"` | 辞書を必ず使う。同梱しないビルドで辞書が見つからなければ設定の誤り（終了コード 2）。`--dict` は `required` を兼ねる |

見つかった辞書が読めない（壊れている・形式の版が違う）ときは、次の候補や同梱の辞書に黙って切り替えず、設定の誤りにします（終了コード 2）。`auto` で share ディレクトリから選んだ辞書も同じです。`noslop dict download <名前> --force` で取り直すか、`dictionary` で別の辞書を指定してください。

`auto` で使う辞書は、手元の share ディレクトリに何を取得したかで変わります。CI のように、どこで実行しても同じ結果にしたい場面では、`dictionary = "bundled"` か `share:<名前>` で辞書を固定してください。品詞で数えるルール（P15・P16）は同梱の IPAdic で校正したので、校正した条件で判定するなら `bundled` です。

```toml
[morphology]
mode = "auto"
# 使う辞書: auto（既定）/ bundled / share:<名前> / ファイルのパス
dictionary = "auto"
```

使った方式は、text の集計の行（「辞書あり (同梱の ipadic)」など）、JSON の `settings.morphology`、改稿指示の `settings.method` と `settings.dictionary` に出ます。

### 別の辞書を使う

hasami は、ほかにもビルド済みの辞書を配布しています。NEologd の語彙を含む `ipadic-neologd` と `ipadic-neologd-sudachi` は、収録語が多い分だけ大きく、どちらも 220MB を超えるため同梱していません。`noslop dict download` で hasami の share ディレクトリに取得できます。share ディレクトリは hasami と同じ規則で決まり、`HASAMI_DATA_DIR` が設定されていればそこ、次に `$XDG_DATA_HOME/hasami`、Windows では `%LOCALAPPDATA%\hasami`、どれもなければ `~/.local/share/hasami` です。

| 名前 | 大きさ | 取得の量（圧縮版） | 中身 |
|---|---:|---:|---|
| `ipadic` | 約 18MB | 約 5.9MB | IPAdic（同梱の辞書と同じ mecab-ipadic から作ったもの） |
| `ipadic-neologd` | 約 222MB | 約 68MB | IPAdic + NEologd |
| `ipadic-neologd-sudachi` | 約 238MB | 約 73MB | IPAdic + NEologd + SudachiDict（hasami の推奨。語彙が最も多い） |

```bash
# share ディレクトリ（既定は ~/.local/share/hasami）に取得する（名前を省くと ipadic-neologd-sudachi）
noslop dict download ipadic-neologd-sudachi

# 取得済みかと、辞書を指定しないときに使う辞書を確かめる（通信しない）
noslop dict list

# 辞書を指定しなければ（auto）、次の実行から取得した辞書を使う
noslop check docs/

# 辞書を名前で固定する
noslop check docs/ --dict share:ipadic-neologd-sudachi
```

取得した辞書は、辞書を指定しないとき（`auto`）に次の実行から使われます。いくつか取得してあれば、hasami の推奨順で選びます。どのマシンでも同じ辞書で判定したいときは、プロジェクトの設定の `[morphology]` に `dictionary = "share:ipadic-neologd-sudachi"` と書きます（取得していないマシンでは、取得のコマンドを案内して設定の誤りになります）。手元のマシンでだけ使うなら、ユーザーの設定（`~/.config/noslop/config.toml`）に書きます。

- 取得元は、noslop に組み込んだ hasami のリリースの目録（`noslop dict list` の 1 行目に出る版）にある、そのリリースの添付ファイルです。目録は noslop をリリースするたびに hasami の最新のリリースに合わせます
- 既定では、zstd で圧縮した版（`<名前>.hsd.zst`。3 分の 1 ほどの大きさ）を取り、受け取りながら展開します。受け取った圧縮版と展開した辞書の両方を目録の大きさと SHA-256 で確かめ、hasami の辞書として読めることも確かめてから置きます。途中で失敗しても、すでにあるファイルは消さず、壊しません。取得の処理は hasami のライブラリ（`hasami::download`）を使っています
- 置き場所に中身の違うファイル（hasami の別の版など）があるときは、`--force` を付けたときだけ置き換えます。`--force` は正しいファイルがあっても取り直します
- ミラーがあれば `--source <URL>` で取得元を切り替えられます（`<URL>/<名前>.hsd.zst` を取得します。どの取得元でも大きさと SHA-256 を確かめます）。圧縮版を置いていないミラーでは `--uncompressed` を付けると、展開前の `<URL>/<名前>.hsd` を取得します
- 品詞で数えるルール（P15・P16）の閾値は、同梱の IPAdic で校正しています。ほかの辞書では語の区切り方や品詞が変わるので、指摘の数や位置が変わることがあります。校正した条件で判定するなら `dictionary = "bundled"` を指定します

share ディレクトリの外に置いた辞書は、ファイルのパスで指定します（`--dict path/to/ipadic-neologd.hsd`）。

辞書を同梱しないバイナリは `cargo build --release --no-default-features` で作れます。このときの `auto` は、share ディレクトリに配布辞書がなければ、ほかの `*.hsd` を名前順に探します。それもなければ辞書なしの近似で判定します（`mode = "required"` なら設定の誤り）。`bundled` は使えず、設定の誤りになります。

## 校正の考え方

既定で有効にしているルールと閾値は、人間の文書と 7 種のモデルが生成した文書（人間 71〜103 本、AI 81〜381 本）で誤検知率を確かめたものです。主な判断は次のとおりです。

- **文長の単調さ（R01）** — burstiness `(σ−μ)/(σ+μ)` は変動係数の単調変換で、文字数で測ってもモーラで測っても判定力はほぼ同じでした。文書単位で `-0.38` 未満なら、人間の文書の誤検知は 1.4%、AI の文書の検出は約 58% です。`-0.24` では人間の文書の 32% に発火したため採りません。段落単位では文が少なく統計が安定しないため、文書全体（地の文が 20 文以上）で判定します。10〜19 文の文書は、より厳しい `-0.45` 未満のときだけ情報として出します。
- **読点の数** — 1 文に読点 4 つ以上を指す方式は、実文書の 62% に発火し、中身のほとんどが同格の列挙でした。そこで読点の数は数えず、埋もれた列挙を指摘するルール（R04）に置き換えています。
- **長い一文（R03）** — 長文は AI の証拠になりません（AI の文書の検出は約 1%）。ただし読みにくい箇所の指摘としては役に立つので、読みやすさのレーンで 90 字（コーパスの約 91 パーセンタイル）を目安にしています。
- **冗長表現の辞書（P04・P05）** — 冗長表現を網羅的に拾う辞書は、人間の良文の 25.5% に発火しました。実験的ルールに留めています。
- **定型句** — 人間のほうが多く使っていた語（「最後に」「まさに」）は外し、人間にも一定数ある語（「重要なのは」「このように」など）は重大度を情報に下げています。
- **対比の反復（R05）** — 回数だけで重大度を決めると、長い文書で薄い頻度でも強く出ます。総文数に対する比率で、2% 未満は情報、2〜3% は警告、3% 以上は重大にしています。
- **体言止め（R06）** — 「体言止めが多いと AI 臭い」という前提はデータと逆でした（人間のほうが使う）。長い文書に体言止めが 1 つもないことを、情報として指します。
- **構造の癖（S01〜S10）** — 定量校正が済んでいないため、すべて実験的ルールです。
- **後から足したルール（P18〜P21・R11〜R16）** — チャット応答の名残・誇張・出典をぼかした権威付け・問いと自答・直しすぎの均一さ・読点の癖・文や段落の重複・定型の結び・三項列挙の反復は、検出条件が未校正のため実験的ルールです。R14 は文字列の重複だけを読みやすさの指摘として扱い、意味の近さは判定しません。P21 は出典の確認を促すもので、根拠の欠如や虚偽を断定しません。

辞書を使わない近似（体言止めの推定、列挙の判定など）は、元の校正条件と同じではありません。近似で判定するルールは、再校正が済むまで実験的か情報の扱いにしています。「の」の連鎖（P16）と連続漢字（P15）は、同梱の形態素解析の辞書で、元の校正と同じ品詞の条件で数えます。

手元の文書で測り直すには `noslop calibrate --human <人の文書> --ai <生成文書>` を使います。ルールごとの誤検知率・検出率、閾値を動かしたときの率、校正済みに上げる候補と見直しが必要な校正済みルールを出します（閾値や状態は書き換えません）。コーパスの集め方・生成文書の作り方（[tools/corpus](tools/corpus)）・昇格の条件は [docs/calibration.md](docs/calibration.md) にまとめてあります。

## 開発

[mise](https://mise.jdx.dev/) が必要です。ツールチェーンの版は `mise.toml` で固定しています。Makefile はツールを `mise exec` 経由で呼ぶので、シェルで `mise activate` を済ませていなくても `mise.toml` の版で動きます。

```bash
make setup   # mise.toml のツールチェーンを入れ（mise install）、依存を取得する
make ci      # CI と同じ検査
```

ターゲットの一覧は `make`（引数なし）でも表示できます。下の表の説明は、その出力と同じです。

| コマンド | 説明 |
|---|---|
| `make setup` | ツールチェーン (mise.toml) を入れ、依存を取得する |
| `make build` | デバッグ版をビルドする |
| `make release` | リリース版をビルドする |
| `make run` | デバッグ版を実行する (引数は ARGS="...") |
| `make install` | リリース版をビルドし、バイナリとスキル (claude・codex) を入れる |
| `make uninstall` | 入れたバイナリを取り除く (スキルは残す) |
| `make test` | テストを実行する |
| `make test-no-default-features` | 辞書を同梱しないビルドでテストする (--no-default-features) |
| `make lint` | clippy を実行する (警告はエラー) |
| `make clippy` | lint の別名 |
| `make fmt` | コードを整形する (書き換える) |
| `make fmt-check` | 整形済みかを確かめる (書き換えない) |
| `make check` | 整形の確認と clippy (書き換えない) |
| `make docs` | 組み込みのルールから docs/rules.md を作り直す |
| `make docs-check` | docs/rules.md が最新かを確かめる (書き換えない) |
| `make ci` | CI と同じ検査 (整形・clippy・テスト・docs/rules.md) |
| `make clean` | ビルドの成果物を消す |
| `make dict-catalog` | 配布辞書の目録 (dict/catalog.json) を hasami のリリースに合わせる (TAG=... で版を指定、省くと最新) |
| `make dict-check` | 目録の辞書を実際に取得し、大きさ・SHA-256・読めることを確かめる (通信が要る) |
| `make help` | このヘルプを表示する |

`make ci` は、フォーマットの確認、clippy（警告はエラー）、テスト、`docs/rules.md` が最新かの確認、辞書を同梱しないビルドのテストを実行します。CI は Linux と macOS で `make setup` と `make ci` を実行し、Windows では make を使わず、`docs/rules.md` の確認以外の検査を cargo で直接実行します。ルールの定義や説明文を変えたら、`make docs` で `docs/rules.md` を作り直してください。忘れると `make ci` が失敗します。

cargo のコマンドには `--locked` を付け、`Cargo.lock` のとおりに依存を解決します。依存を足した直後など、`Cargo.lock` を更新したいときは `CARGO_FLAGS=` を付けます。mise を使わない場合は `SYSTEM_TOOLS=1` を付けると、`PATH` 上のツールで動きます（ツールの版はそろいません）。

## リリース

GitHub の Actions タブで Release ワークフローを選び、Run workflow で実行します。版は `YY.M.COUNTER` の形（`26.9.100` など）で、その月の最初のリリースは COUNTER を 100 から始め、同じ月の 2 回目以降は 1 ずつ上げます。`dry_run` を有効にすると、次の版を計算して `Cargo.toml` の変更を表示するだけで、コミット・タグ・ビルド・公開はしません。リリースには Linux x86_64 / arm64、macOS x86_64 / arm64、Windows x86_64 のアーカイブ（`noslop-<ターゲット>.tar.gz`、Windows は `.zip`。中身はバイナリと `LICENSE`・`THIRD_PARTY_NOTICES.md`）と、`SHA256SUMS` が付きます。

公開の後、Homebrew の tap（[owayo/homebrew-noslop](https://github.com/owayo/homebrew-noslop)）の formula を、新しい版の URL と SHA-256 に書き換えて push します。tap への push には GitHub App のトークンを使うので、リポジトリの Variables に `APP_CLIENT_ID`（App の Client ID）、Secrets に `PRIVATE_KEY`（App の秘密鍵）が要ります。どちらかがなければ、警告を出して tap の更新だけを飛ばします。

Release ワークフローは、`noslop dict download` が使う配布辞書の目録（`dict/catalog.json`）も hasami の最新のリリースに合わせます。目録が変わったときは、`make ci` と、3 つの辞書を実際に取得して確かめる `make dict-check` を通してから、版の更新と同じコミットに入れます。

## ロードマップ

- LSP（エディタ上でのリアルタイム表示）
- 形態素解析の辞書を使う判定を、埋もれた列挙（R04）と文頭の反復（R08）にも広げる
- `noslop calibrate` で集めたコーパスで辞書なしの近似を再校正し、実験的ルールを stable に上げる
- CI で前回の JSON 結果と比べ、新しく出た指摘だけを出す（ベースライン）

## ライセンス

[MIT](LICENSE)

noslop のルール体系・語句カタログ・閾値の一部は、MIT ライセンスで公開されている日本語の文章作法プロジェクトに由来します。著作権表示とライセンス全文は [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) にあります。

文分割には、日本語の形態素解析器 [hasami](https://github.com/owayo/hasami)（MIT）の辞書を使わない文分割を使っています。hasami が組み込む例外表（文末記号を含む語の一覧）は辞書データ（SudachiDict ほか）から抽出したもので、その出典と著作権表示も [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) にあります。

既定のバイナリ（feature `bundled-dict`）には、hasami が mecab-ipadic から作った形態素解析の辞書を同梱しています。mecab-ipadic のライセンス（NAIST-2003）の条文は [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) に、辞書の出所は [dict/README.md](dict/README.md) にあります。

見る観点の一部は、[textlint-rule-preset-ai-writing](https://github.com/textlint-ja/textlint-rule-preset-ai-writing)（MIT）も参考にしています。コードと語句の一覧は含みません。
