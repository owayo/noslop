# 形態素解析の辞書

[README に戻る](../README.md)

## 形態素解析の辞書

noslop は形態素解析器 [hasami](https://github.com/owayo/hasami) の IPAdic の辞書（`dict/ipadic.hsd`）をバイナリに同梱していて、何も指定しなくても、品詞で数えるルールを辞書で判定します。hasami の share ディレクトリに配布辞書を取得してあれば、そちらを使います（[使い方の指定](#使い方の指定)）。同梱の辞書で判定すれば、元の校正と同じ条件になります。

| ルール | 辞書ありの判定（既定） | 辞書なしの判定（`--no-dict`） |
|---|---|---|
| P16「の」の連鎖 | 連体の「の」を数え、隣り合う「の」の間が 2 語以内なら続いているとみなす。辞書が細かく分ける語（「必要性」「話し方」のような接尾辞の付いた語・数・カタカナ語）は 1 語にまとめて数える | 漢字・カタカナ・英数字の語に挟まれた「の」だけを数える。ひらがなを含む語をはさむ連鎖（「の家の大きな犬の」）や、端の語がひらがなの連鎖（「魂の安静のため」）は拾えない |
| P15 連続漢字 | 固有名詞を含む連なり（年号・人名など）を品詞で除く | 定番の接尾辞（〜委員会・株式会社など）で終わる連なりだけを除く |

ほかのルールは辞書の有無で変わりません。文分割も辞書を使いません。体言止め（R06）は、文書単位の判定が辞書なし・ありで変わらなかったため、辞書なしの推定のままです。

同梱の辞書は、バイナリに埋め込んだまま複製せずに読みます。辞書を使うルールが動かない実行（`--no-readability` や、`--only-rules` で P15・P16 を外したとき）では、辞書を探しも読みもしません。share ディレクトリやファイルの辞書は mmap で読むので、大きな辞書でも、メモリに載るのは解析で触れた部分だけです。

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

見つかった辞書が読めない（壊れている・形式の版が違う）ときは、次の候補や同梱の辞書に黙って切り替えず、設定の誤りにします（終了コード 2）。`auto` で share ディレクトリから選んだ辞書も同じです。`noslop dict download <名前>` で取り直すか、`dictionary` で別の辞書を指定してください。

現在の辞書は HSD v5 形式です。v4 以前の辞書は読めません。以前に取得した配布辞書は `noslop dict download <名前>` で取り直してください。自作の辞書は hasami v26.9.107 以降の対応版で作り直します。同梱の辞書だけを使う場合は、noslop の更新だけで移行できます。

`auto` で使う辞書は、手元の share ディレクトリに何を取得したかで変わります。CI のように、どこで実行しても同じ結果にしたい場面では、`dictionary = "bundled"` か `share:<名前>` で辞書を固定してください。品詞で数えるルール（P15・P16）は同梱の IPAdic で校正したので、校正した条件で判定するなら `bundled` です。

```toml
[morphology]
mode = "auto"
# 使う辞書: auto（既定）/ bundled / share:<名前> / ファイルのパス
dictionary = "auto"
```

使った方式は、text の集計の行（「辞書あり (同梱の ipadic)」など）、JSON の `settings.morphology`、改稿指示の `settings.method` と `settings.dictionary` に出ます。

### 別の辞書を使う

hasami は、ほかにもビルド済みの辞書を配布しています。NEologd の語彙を含む `ipadic-neologd` と `ipadic-neologd-sudachi` は、収録語が多い分だけ大きいため同梱していません。`noslop dict download` で hasami の share ディレクトリに取得できます。share ディレクトリは hasami と同じ規則で決まり、`HASAMI_DATA_DIR` が設定されていればそこ、次に `$XDG_DATA_HOME/hasami`、Windows では `%LOCALAPPDATA%\hasami`、どれもなければ `~/.local/share/hasami` です。

| 名前 | 中身 |
|---|---|
| `ipadic` | IPAdic（同梱の辞書と同じ mecab-ipadic から作ったもの） |
| `ipadic-neologd` | IPAdic + NEologd |
| `ipadic-neologd-sudachi` | IPAdic + NEologd + SudachiDict（hasami の推奨。語彙が最も多い） |

辞書の大きさは `noslop dict list` で確認できます。圧縮版の大きさは取得時に表示します。

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
- 既定では、zstd で圧縮した版（`<名前>.hsd.zst`）を取り、受け取りながら展開します。受け取った圧縮版と展開した辞書の両方を目録の大きさと SHA-256 で確かめ、hasami の辞書として読めることも確かめてから置きます。途中で失敗しても、すでにあるファイルは消さず、壊しません。取得の処理は hasami のライブラリ（`hasami::download`）を使っています
- 既存の辞書があっても、毎回取得し直して置き換えます。同じ版や壊れた辞書でも追加のオプションは不要です。取得・検証に失敗した場合は、既存の辞書を保持します
- ミラーがあれば `--source <URL>` で取得元を切り替えられます（`<URL>/<名前>.hsd.zst` を取得します。どの取得元でも大きさと SHA-256 を確かめます）。圧縮版が HTTP 404 のときだけ、非圧縮版の `<URL>/<名前>.hsd` へ自動で切り替えます。通信・検証・展開の失敗では切り替えません。`--uncompressed` を付けると、最初から非圧縮版を取得します
- 品詞で数えるルール（P15・P16）の閾値は、同梱の IPAdic で校正しています。ほかの辞書では語の区切り方や品詞が変わるので、指摘の数や位置が変わることがあります。校正した条件で判定するなら `dictionary = "bundled"` を指定します

share ディレクトリの外に置いた辞書は、ファイルのパスで指定します（`--dict path/to/ipadic-neologd.hsd`）。

辞書を同梱しないバイナリは `cargo build --release --no-default-features` で作れます。このときの `auto` は、share ディレクトリに配布辞書がなければ、ほかの `*.hsd` を名前順に探します。それもなければ辞書なしの近似で判定します（`mode = "required"` なら設定の誤り）。`bundled` は使えず、設定の誤りになります。
