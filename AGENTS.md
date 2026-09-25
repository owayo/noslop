# noslop — エージェント向けガイド

このリポジトリで作業する AI エージェント (と人間の開発者) 向けの案内。利用者向けの説明は [README.md](README.md) にある (日本語だけで書く。英語版は置かない)。

## プロジェクト概要

noslop は、日本語の文章から「AI 臭さ」を機械的に拾う Rust 製の Linter。判定器ではなく、疑わしい箇所を決定的に並べ、直すかどうかは書き手に委ねる。

- 文字種・語句パターン・文長の統計で判定する。品詞で数えるルール (P15・P16) は、バイナリに同梱した hasami の IPAdic の辞書 (`dict/ipadic.hsd`) で、元の校正と同じ条件で判定する (`morph.rs`)。`--no-dict` や同梱しないビルドでは辞書なしの近似で動く
- 既定で有効にするのは、コーパスで誤検知率を確かめた語句と閾値だけ。未校正のものは experimental にする
- AI 臭さ (`slop`) と読みやすさ (`readability`) の 2 つのレーンを混ぜない
- 文書全体の点数は出さない。指摘を 1 件ずつ並べ、レーンと重大度ごとの件数を数えるだけにする (以前の「自然度スコア」は校正しておらず、人の文書と AI の文書を見分けていなかったので外した。README の「文書全体の点数を出さない理由」)

## 技術スタック

- Rust (edition 2024)。ツールチェーンの版は `mise.toml` が正
- Markdown: `pulldown-cmark`
- 文分割と形態素解析: [hasami](https://github.com/owayo/hasami)。文分割は `hasami::sentence` (辞書を使わない)、形態素解析は `analyzer` feature (辞書があるときだけ)。crates.io の同名クレートは別物なので、git の依存でリリースのタグ (`Cargo.toml` の `tag`) を指定して入れる。テストは `build` feature (dev-dependency) で小さな辞書を組み立てる。既定の feature `bundled-dict` で `dict/ipadic.hsd` (約 18MB) を `hasami::include_hsd!` でバイナリに埋め込み、`Dictionary::from_static` で複製せずに読む。辞書を差し替える手順は `dict/README.md`。上げるときはタグを書き換えて `cargo update -p hasami` を実行し (hasami の `rust-version` が上がっていれば、`Cargo.toml` の `rust-version` もそろえる。上げると clippy が MSRV で抑えていた指摘を出すことがある)、例外表の版を固定したテスト (`segment.rs`) が落ちたら分割の差分と THIRD_PARTY_NOTICES.md の NOTICE の写しを確かめる。`src/dictionaries.rs` の `HASAMI_TAG`・`DEFAULT_SOURCE` (タグのリリースの添付ファイルの URL)・`DICTIONARIES` (配布辞書の大きさと SHA-256。リリースに添付された `dictionaries.json` の `size` と `sha256`) も書き換える (テストが `Cargo.toml` のタグと、`dict/ipadic.hsd` と表の ipadic の一致を確かめる)
- 語句の照合: `aho-corasick` / `regex`
- ファイル探索: `ignore` (.gitignore を尊重)、並列化: `rayon`
- CLI: `clap`、設定: `toml` + `serde`、出力の色: `anstream` / `anstyle`
- 配布辞書の取得 (`noslop dict download`): `ureq` (TLS は rustls。社内の CA を入れた環境でも通るよう、OS の証明書ストアで検証する `platform-verifier`)、検証は `sha2`、置き換えは `tempfile`

## 構成

```mermaid
flowchart TD
    CLI[cli.rs<br/>引数・サブコマンド] --> CFG[config.rs<br/>noslop.toml の探索と統合]
    CLI --> WALK[walk.rs<br/>対象ファイルの列挙]
    WALK --> DOC[document.rs<br/>Document / Block / Sentence]
    DOC --> MD[markdown.rs<br/>Markdown → ブロック]
    DOC --> TXT[plaintext.rs<br/>テキスト → ブロック]
    MD --> DIR[directive.rs<br/>抑制コメントの読み取り]
    TXT --> DIR
    DOC --> SEG[segment.rs<br/>文分割]
    CFG --> ENG[engine.rs<br/>ルールの選択・実行]
    DOC --> ENG
    ENG --> RULES[rules/<br/>phrases・rhythm・structure・custom]
    ENG --> MORPH[morph.rs<br/>形態素解析 (辞書は同梱)]
    RULES --> MORPH
    ENG --> SUP[suppress.rs<br/>抑制の適用]
    ENG --> OUT[output/<br/>text・json・toon・github・brief]
    CLI --> DIFF[diff/<br/>改稿の前後の比較]
    CLI --> CAL[calibrate.rs<br/>コーパスでの校正]
    CLI --> MCP[mcp.rs<br/>MCP サーバー]
    CLI --> HOOK[hook.rs<br/>Claude Code のフック]
    CLI --> DICTS[dictionaries.rs<br/>配布辞書の取得]
    MORPH --> DICTS
    DIFF --> ENG
    CAL --> ENG
    MCP --> ENG
    HOOK --> ENG
    MCP --> OUT
    HOOK --> OUT
```

## モジュールの責務

| ファイル | 責務 |
|---------|------|
| `src/main.rs` | エントリポイント。終了コードを返す |
| `src/cli.rs` | clap の定義と、`check` / `diff` / `calibrate` / `rules` / `explain` / `init` / `mcp` / `hook` / `skill-install` / `dict` の実行。`check` と `diff` はルールの選び方の引数 (`EngineArgs`) を共有する。`check` は出す内容 (`--report full\|brief`) と形式 (`--format`) の組み合わせを最初に確かめる |
| `src/config.rs` | `noslop.toml` / `.noslop.toml` の探索 (カレントから親へ、最初の 1 つ) と、CLI の指定との統合 |
| `src/walk.rs` | 対象ファイルの列挙 (.gitignore・.ignore・.noslopignore・拡張子・設定の除外。除外は .gitignore と同じ書式で、設定ファイルのディレクトリが基準。直接指定したファイルは拡張子と除外を問わない) |
| `src/engine.rs` | ルールの選択 (stable / experimental / ジャンル / 明示の有効化・無効化)、設定値の適用、実行、重大度の上書き、fingerprint、並べ替え。辞書を使う有効なルール (`Rule::uses_morphology`) があるときだけ辞書を読み、文書ごとに `DocMorphology` を作ってルールに渡す |
| `src/morph.rs` | 形態素解析。`[morphology]`・`--dict`・`--no-dict` から辞書を決めて読み込む (`resolve`)。探す順は、明示のパス → `HASAMI_DICT` → 同梱の辞書 (share ディレクトリは探さない。同梱しないビルドでは hasami の既定の場所を探し、`auto` で見つからなければ辞書なし)。明示の指定が `share:<名前>` なら share ディレクトリの辞書に直す (`dictionaries::resolve_share`。`HASAMI_DICT` には当てない)。指定した辞書が読めなければエラーにし、同梱の辞書に切り替えない。同梱の辞書は埋め込んだバイト列を複製せずに読み (`Dictionary::from_static`)、組み立てはプロセスで 1 度だけにして共有する (`Morphology::bundled`)。文書ごとの解析器で、ルールが求めた文だけを解析して覚えておく。使った方式 (`MorphologyStatus`) は出力の `settings` に載る |
| `src/suppress.rs` | 抑制コメントを診断に当てる。未知のルール名は警告にする |
| `src/output/` | text (色付き。ファイルの中をレーンごとの節に分け、要約もレーンごとに数える)・json (安定スキーマ)・toon (JSON と同じデータを TOON で。符号化は `toon-format` クレート)・github (ワークフローコマンド、エスケープは出力器の責務)・brief (AI や編集者に渡す改稿指示。`Brief` のデータを組み立て、Markdown・JSON・TOON に描き分ける。データはルールの表と該当箇所の表に分け、TOON の表形式が効くようにしている) |
| `src/diff/` | `noslop diff`。指摘の突き合わせ (`mod.rs`、fingerprint を多重集合で)、事実の消失と追加 (`facts.rs`)、改稿の偏り (`shifts.rs`)、出力 (`render.rs`) |
| `src/calibrate.rs` | `noslop calibrate`。人の文書と生成文書で、ルールごとの誤検知率・検出率、`Rule::measure` による閾値の掃引、昇格候補と見直しを出す (手順は `docs/calibration.md`) |
| `src/mcp.rs` | `noslop mcp`。標準入出力の JSON-RPC で `check` / `diff` / `explain` / `rules` を提供する。`check` は `report` (既定 `brief`) と `format` (`markdown`・`json`・`toon`) で返すものを選ぶ (版の扱いは `docs/integrations.md`) |
| `src/hook.rs` | `noslop hook claude-code`。PostToolUse の入力から変わった行を求め、重なる指摘だけを brief で返す |
| `src/skill.rs`・`skills/SKILL.md` | `noslop skill-install`。`skills/SKILL.md` をバイナリに埋め込み、`~/.claude/skills/noslop/` か `~/.codex/skills/noslop/` に書く。`make install` もバイナリを入れたあとに両方へ入れる (`SKILL_TARGETS` で選ぶ)。CLI の使い方を変えたら SKILL.md も直す (本文に `$` の直後の数字や `$ARGUMENTS` を書かない。スキルの引数に置き換わる) |
| `src/dictionaries.rs` | `noslop dict download` / `list`。hasami の配布辞書の表 (`DICTIONARIES`。名前・大きさ・SHA-256) と、タグに固定した取得元 (`HASAMI_TAG`・`DEFAULT_SOURCE`。そのタグの GitHub のリリースの添付ファイル)。share ディレクトリ (`share_dir`。hasami の `hasami::analyzer::data_dir` をそのまま使う。`HASAMI_DATA_DIR` → `$XDG_DATA_HOME/hasami` → Windows は `%LOCALAPPDATA%\hasami` → `~/.local/share/hasami`) と、`share:<名前>` の解決 (`resolve_share`。名前にパスの区切り・`:`・`..` を書かせない)。取得は保存先と同じディレクトリの一時ファイル (`.<名前>.hsd.<乱数>.part`) に書き、大きさ・SHA-256・辞書として読めることを確かめてから rename で置く (失敗しても既存のファイルは消さず、壊さない。24 時間より古い一時ファイルは次の取得で消す)。取っただけでは使わない (辞書を指定しないときの結果を、手元に入れた辞書で変えないため)。HTTP (ureq) は TLS の provider と root_certs を明示する |
| `src/heading.rs` | 見出しの形 (コロン型・問い型・番号型) の分類。S07 と `noslop diff` で共有する |
| `src/document.rs` | 文書モデル。解析用テキストと原文の対応 (`TextMap`)、行・列 (`LineIndex`) |
| `src/markdown.rs` | Markdown をブロック (段落・リスト項目・見出し・表セル) に分け、コード・URL・装飾を解析用テキストから外す。front matter は文書の先頭のものだけを自前で見つけて解析から外す (pulldown-cmark のメタデータブロックの記法は文書の途中の `---` にも当たって本文を捨てるので、有効にしない)。段落の中の改行のうち、書式から文の区切りと分かるものは `Block::sentence_breaks` として記録する (`breaks_sentence`。1 行 1 項目の箇条書きやラベルの行を 1 文につながないため)。次の行が Markdown の記法にない箇条書きの記号・番号 (「・」「◯」「①」「(1)」) か「短い見出し＋全角コロン」で始まる、直前の行がコロンで終わる・【】で囲んだ見出し風の行・太字だけの行である、直前の行が日本語を含まない英文の文末で次の行が日本語で始まる、ハード改行の直前が文末記号・読点・ひらがなで終わらない、直前の行が丁寧体の文末 (「です」「ます」「ください」など) で終わり次の行が括弧で始まらない、のどれか。ひらがなや読点で終わる行の改行は本文の折り返しとみなしてつなぐ |
| `src/plaintext.rs` | テキストをブロックに分ける (空行・字下げ・箇条書き記号。空行がほとんどない文書は 1 行 1 段落とみなし、文末記号のない短い 1 行は見出しと推定する。コメントだけの行は段落を切らない) |
| `src/segment.rs` | 文分割。`hasami::sentence` への橋渡し (括弧の対応を取ってから、対応の取れた括弧の内側と例外表の語の内側では分割しない)。改行を文の区切りにするモードでは、`line_breaks` の位置を改行とみなして分割する (`split_with_breaks`)。書式から文の区切りと分かる改行 (`sentence_breaks`) は、改行の扱いの設定によらず切る。組み込みの例外表の版 (`BUILTIN_EXCEPTIONS_VERSION`) をテストで固定し、表が変わったら気付けるようにしている |
| `src/directive.rs` | `<!-- noslop-... -->` の読み取り |
| `src/text.rs` | 文字種の判定と文長の数え方。文末記号・括弧の判定は、文分割と字の集合がずれないよう `hasami::sentence` のものを再公開する。文末の記号を除いた本体と、名詞らしい終止 (体言止め) の推定もここに置き、R06・R12 と `noslop diff` で共有する |
| `src/genre.rs` | ジャンルと別名 |
| `src/diagnostic.rs` | 診断・重大度・レーン・ステータス・原文上の範囲 |
| `src/rules/mod.rs` | `Rule` trait・`RuleMeta`・`Scope`・`builtin_rules`、校正用の測定値 (`Measure`・`Fires`) と校正の基準の重大度 (`calibration_basis`) |
| `src/rules/phrases.rs` | 語句パターン系 (`P`)。`phrases/catalog.rs` が語句辞書で動く P01〜P12・P18・P19、`phrases/syntax.rs` が構文の型 (P13・P14・P20)、`phrases/reading.rs` が読みやすさのルール (P15〜P17)、`phrases/engine.rs` が照合の共通部品 |
| `src/rules/rhythm.rs` | リズム・統計系 (`R`)。ルールごとに `rhythm/` 配下のファイル (burstiness・endings・length・buried_list・antithesis・paragraphs・leads・cleft・self_answer・overcorrection・commas) |
| `src/rules/structure.rs` | 構造系 (`S01`〜`S10`) |
| `src/rules/custom.rs` | 設定ファイルの独自ルール (`[[custom]]`) |
| `src/rules/testing.rs` | ルールのテスト用の近道 (`run` / `run_with` / `matched`) と、`measure` と `check` の一致の確認 (`assert_measures_agree`) |

位置は内部で原文の UTF-8 バイト位置 (`Span`) に統一し、行・列は出力時に計算する。解析用テキストの位置は `Block::to_source` で原文に戻す。

## ルールを足す・変えるとき

1. 系統を決める。文の中の表現なら `P`、文や段落をまたぐ集計なら `R`、Markdown の体裁なら `S`
2. ID はその系統の次の番号にする。**公開した ID の意味は変えない**。廃止したルールの ID は再利用しない
3. `RuleMeta` を書く。`lane`・`status`・`default_severity`・`summary` と、下の書式の `explanation`
4. 語句ルールは語句ごとにステータスと重大度を持たせる。人間の文章にも一定数出る語は `info` にする
5. テストを書く。検出する例・検出しない例・スコープ (段落だけか)・experimental の有効化・原文上の位置 (`matched` で原文の文字列と一致すること) の 5 点は必ず押さえる
6. 閾値を持つルールは `Rule::measure` を実装し、`noslop calibrate` で閾値を掃引できるようにする。実装したら `rhythm.rs` / `structure.rs` のテストにある `MEASURED` に ID を足す。`testing::assert_measures_agree` が、測定値が閾値を越えることと `check` が指摘することの一致を、閾値を測定値の前後に動かして確かめる (指摘する例と、値は測れるが指摘しない例を 1 つ以上用意する)。重大度を切り替えるだけの閾値 (R05 の `error_above` など) は `Measure::at` で切り替え先の重大度を示す
7. 特定の重大度の率で校正したルールは、`Rule::calibration_basis` でその重大度を返す (既定は、既定の重大度が警告以上なら警告、情報なら情報。R05 は重大)。`noslop calibrate` は、見直しの判定をこの重大度以上の指摘で数える
8. 品詞で判定したほうが元の校正条件に近いルールは、`Rule::uses_morphology` を真にし、`ctx.morph` (辞書があるときだけ `Some`) の形態素で判定する。辞書がない・文を解析できないときは辞書なしの近似に戻す。テストは `morph::testing::morphology` で小さな辞書を組み立て、`testing::run_with_morphology` で当てる (辞書なしと辞書ありの結果が違う例を並べる)
9. README のルール一覧を更新し、`make docs` で `docs/rules.md` を作り直す

### `explanation` の書式

`noslop explain` と `docs/rules.md` にそのまま出る。見出しは次の 5 つで固定する。

```markdown
### 何を見るか
(検出の条件を具体的に。閾値があれば数値で)

### なぜ問題か
(なぜ AI 臭さ・読みにくさにつながるのか)

### 直し方
(どう直すか。機械的に全部直す必要はないことも書く)

### 例
- 直す前: ...
- 直した後: ...

### 根拠
(校正の結果、または未校正で experimental である理由)
```

例文は自分で作る。既存の資料の文面をそのまま写さない。

## 校正の原則

- **閾値はデータなしに変えない**。変えるなら、人間の文書での誤検知率と AI の文書での検出率を `noslop calibrate` で測り (手順は [docs/calibration.md](docs/calibration.md))、根拠を `explanation` の「根拠」に書く。コーパスはリポジトリに入れない
- **未校正のものは experimental にする**。辞書なしの近似で元の校正条件から外れるものも同じ
- **辞書ありの判定は元の校正条件 (品詞で数える) に合わせる**。辞書あり・なしで結果が変わるルールは、元の検出器と比べた両方の一致率を `explanation` の「根拠」に書く。辞書で精度が上がらないルール (R06 など) は辞書を使わない
- **文書単位の指標を足すなら、先に検証する**。ルールごとの誤検知率の校正は、指摘を足し合わせた値が文書を見分けることを保証しない。文書全体の点数や判定を出すなら、ジャンルと長さを分けた保留のデータで見分けられることを示してから、目的と名前を決める
- **統計系ルールは地の文だけで集計する**。校正を地の文 (見出し・リスト・引用・表・コードを除く) で行ったため
- **語句ルールの既定のスコープは段落だけ**。リスト・表・引用は設定で広げる
- 人間のほうが多く使うと分かった語は外す。人間にも一定数ある語は `info` に下げる

## テストとチェック

開発のコマンドは Makefile にまとめてある (一覧は `make help`)。make は `mise.toml` の版のツールを `mise exec` 経由で呼ぶので、シェルで mise を activate していなくてよい。

```bash
make setup                         # mise.toml の Rust を入れ、依存を取得する (初回と依存の更新後)
make ci                            # CI と同じ検査 (fmt・clippy -D warnings・test・docs-check・同梱しないビルドの test)
make test                          # テストだけ
mise exec -- cargo test segment::  # モジュールを絞る
make docs                          # docs/rules.md を作り直す
```

- push 前に `make ci` を通す。make が `mise exec` 経由で動くので、手元と CI で clippy の版がずれない。cargo のコマンドには `--locked` が付く (`Cargo.lock` を更新したいときは `CARGO_FLAGS=` で外す)
- CI は ubuntu / macos / windows で検査を回す。Linux と macOS のジョブは `make setup` と `make ci` だけを呼ぶので、検査を足すときは Makefile の `ci` に足し、`.github/workflows/ci.yml` に検査のコマンドを並べない。Windows のジョブはランナーの make (mingw32-make) を避け、`docs-check` 以外の同じ検査を cargo で直接呼ぶ。`ci` を変えたら、ci.yml の Windows のステップもそろえる。Windows では改行を LF のまま checkout している (テストの行・列・バイト位置は LF 前提。`.gitattributes` でも LF に固定している)
- `make ci` は `docs/rules.md` を生成し直して差分がないことも確かめる (`make docs-check`。ファイルは書き換えない)。ルールの定義や説明文を変えたら `make docs` を忘れない
- 統合テストは `tests/cli.rs` (サブコマンドの入出力・終了コード) と `tests/integrations.rs` (MCP サーバーとフック) にある。組み込みルールの増減で壊れないよう、件数は設定ファイルの独自ルールと `--only-rules` で確かめる
- テスト用の文章は、実在の文書や既存の資料を写さずに自分で書く
- 辞書ありの判定のテストは、既定の探索や同梱の辞書に頼らず、hasami の `DictBuilder` で小さな辞書を組み立てる (`morph::testing`、`tests/cli.rs` の `write_dictionary`)。ただし小さな辞書は hasami の既定の文字種で動くので、全角空白 (U+3000) がトークンにならないなど配布の IPAdic と違う点がある。全角空白のトークンに依る処理は形態素を手で組んで確かめる。手元の辞書に左右されないよう、既定の探索に頼るテストは `HASAMI_DICT` と `HASAMI_DATA_DIR` を外し、`XDG_DATA_HOME` を空のディレクトリにする (`HASAMI_DATA_DIR` は `XDG_DATA_HOME` より優先される)。同梱の辞書に依るテストは `cfg(feature = "bundled-dict")` で分ける。同梱しないビルドのテスト (`make test-no-default-features`。中身は `cargo test --no-default-features`) は `make ci` に入っている
- Rust の文字列の行継続 (`\`) は次の行の先頭の空白を消す。説明文を数字や `(` の前で折り返すときは、`\` の前に空白を入れる (「90 字台から 100 字」のように、数字の前後の空白が落ちるため)

## 利用者に見せる文言

- 出力・ヘルプ・ルールの説明文・ドキュメントでは、レーンを「AI 臭さ」「読みやすさ」「独自ルール」(`Lane::label_ja`) と呼ぶ。内部の ID (`slop` / `readability` / `custom`)、JSON のフィールド、オプション名 (`--no-readability` など) はそのまま使う
- 1 件ずつの指摘は、どのレーンでも「指摘」と呼ぶ。数えるときは「読みやすさの指摘 13 件」のように「〜の指摘」を付ける (「読みやすさ 13 件」だけでは、読みやすい点が 13 あるという良い評価にも読める)。レーンやルールそのものを指すときは「読みやすさのレーン」「読みやすさのルール」のように「〜の」でつなぐ
- 「指さし」「読解負荷」のような別の呼び方を作らない。出力ごとに呼び名が違うと、要約の件数と一覧の指摘が対応しなくなる

## 公開リポジトリとしての注意

- 社内の組織名・チーム名、実在のメールアドレス、ホームディレクトリの絶対パスを、コード・ドキュメント・テスト・コミットメッセージに書かない。例には `you@example.com` や `~` を使う
- ルールの知見の出典は [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) に記載している。出典の文面をそのまま持ち込まない
- 図は Mermaid で書く (ASCII アートは使わない)
- コメントとドキュメントは日本語で書く。利用者に見せる診断の文も日本語

## リリース

GitHub Actions の Release ワークフロー (workflow_dispatch) で行う。版は `YY.M.COUNTER` (例: `26.9.100`) で、同じ月の 2 回目以降は COUNTER を 1 ずつ上げる。`dry_run` で版の計算だけを確かめられる。成果物は Linux x86_64、macOS x86_64 / arm64、Windows x86_64 のバイナリと `SHA256SUMS`。

mise 自身の版は `ci.yml` と `release.yml` の `MISE_VERSION` で固定している (公開から 14 日以上たった版を選ぶ)。上げるときは 2 つのファイルを同時に書き換える。
