# コマンドとオプション

[README に戻る](../README.md)

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
| `noslop hook claude-code` | Claude Code のフックとして、書き換えたファイル（PostToolUse）・gws で書き込む値（PreToolUse）・コミットしていない変更（Stop）の指摘を返す |
| `noslop hook command` | claw-hooks のコマンドフックの判定器として、gws で Google ドキュメント・スプレッドシートに書き込む値を書き込む前に検査する |
| `noslop hook file <PATH>` | 編集したファイルのパスだけを渡すフックの仕組み（claw-hooks など）から呼び、コミットしていない変更に重なる指摘をテキストで返す |
| `noslop hook git-diff` | フックの入力を渡せない Stop の仕組み（claw-hooks など）から呼び、リポジトリのコミットしていない変更の指摘をテキストで返す |
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
| `--dict <DICT>` | | 形態素解析の辞書を指定し、必ず使う。`auto`（share ディレクトリの一番良い辞書、なければ同梱の辞書）/ `bundled`（同梱の IPAdic）/ `share:<名前>` / ファイルのパス（hasami の `.hsd`）。詳細は[形態素解析の辞書](dictionaries.md#形態素解析の辞書) |
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
| `noslop mcp` | `--config <PATH>` / `--no-config` | 設定ファイルの指定。ツールと登録の仕方は [docs/integrations.md](../docs/integrations.md) |
| `noslop hook claude-code` | `--brief-limit <N>` / `--include-readability` / `--experimental` / `--genre <GENRE>` / `--whole-file` | 返す箇所の上限（既定 3）、読みやすさの指摘を含めるか、変わった行に限らずファイル全体を見るか。詳細は [docs/integrations.md](../docs/integrations.md) |
| `noslop hook command` | `hook claude-code` と同じもの / `--max-chars <N>` | 出力の文字数の上限（既定 9000。超える分は行の単位で省く）。claw-hooks の出力の上限（既定 1000 文字）に合わせるなら 900。詳細は [docs/integrations.md](../docs/integrations.md) |
| `noslop hook file <PATH>` | `hook claude-code` と同じもの / `--max-chars <N>` | 出力の文字数の上限（既定 9000。超える分は行の単位で省く）。変わった行は git の HEAD との差分から求める。詳細は [docs/integrations.md](../docs/integrations.md) |
| `noslop hook git-diff` | `hook claude-code` と同じもの / `--max-chars <N>` | 出力の文字数の上限（既定 9000）。指摘があれば終了コード 1。詳細は [docs/integrations.md](../docs/integrations.md) |
| `noslop skill-install <claude\|codex>` | `--dir <DIR>` | スキルの置き場（既定は `~/.claude/skills` か `~/.codex/skills`。プロジェクトに置くなら `.claude/skills` など）。`noslop/SKILL.md` を書き、すでにあれば上書きする |
| `noslop dict download [NAME]` | `--dir <DIR>` / `--source <URL>` / `--uncompressed` | NAME は `ipadic` / `ipadic-neologd` / `ipadic-neologd-sudachi`（既定）。保存先（既定は hasami の share ディレクトリ）、取得元の URL（ミラー用）、圧縮版を使わずに非圧縮版を取るかを指定する。既存の辞書は毎回取得し直し、検証に成功してから置き換える。詳細は[別の辞書を使う](dictionaries.md#別の辞書を使う) |
| `noslop dict list` | `--dir <DIR>` | 取得済みかを確かめる場所（既定は share ディレクトリ） |
| `noslop calibrate` | `--human <PATH>` / `--ai <PATH>`（必須・繰り返し可）、`--genre`、`--target-fp`、`--holdout`、`--min-detection`、`--no-experimental`、`-f, --format <text\|json\|markdown>` | コーパスでの測り方。手順は [docs/calibration.md](../docs/calibration.md) |

### 終了コード

| コード | 意味 |
|-------|------|
| `0` | 検査が終わった（指摘の有無は問わない。`diff` は確認事項があっても 0） |
| `1` | `--fail-on` で指定した重大度以上の指摘があった（`check` のみ） |
| `2` | 引数・設定・入出力のエラー（読めないファイルがあった場合も、読めたファイルの結果を出したうえで 2） |

既定の `--fail-on never` では、指摘があっても終了コードは 0 です。noslop は疑いを示す道具で、件数でビルドを止める設計にはしていません。

フック（`noslop hook ...`）の終了コードは、呼び出す側の約束に合わせてあり、上の表と違います。Claude Code と claw-hooks は 2 を「止める」合図として読むので、フックの誤りは引数の誤りも含めて 1 です。2 を返すのは、`hook command` が書き込みを止めるときと、`hook git-diff` の誤りのときだけです（`hook git-diff` は指摘があれば 1）。詳細は [docs/integrations.md](../docs/integrations.md) にあります。
