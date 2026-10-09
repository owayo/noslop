# 同梱の形態素解析の辞書

`ipadic.hsd` は、形態素解析器 [hasami](https://github.com/owayo/hasami) の辞書です。noslop は既定の feature `bundled-dict` でこれをバイナリに埋め込み、品詞で数えるルール (P15・P16) に使います。

本体は Git に置きません。ビルド前に `dict/bundled.json` に固定した版・大きさ・SHA-256 で取得・照合します。`make setup`・`make release` が準備し、`make dict-bundled` (make のない環境では `cargo run --locked -p noslop-xtask -- prepare`) でも準備できます。取得済みなら照合だけで、追跡ファイルは書き換えません。Cargo のビルドでも本体を再照合し、欠落・不一致なら止めます。

| 項目 | 値 |
|---|---|
| 出所 | hasami v26.10.101 のリリースに添付された `ipadic.hsd` |
| 形式 | HSD v5 |
| 元のデータ | mecab-ipadic 2.7.0-20070801 (辞書のメタデータ `sources=ipadic@61b90ba6e669`) |
| 語数 | 390,860 |
| SHA-256 | `2f8d87dc4fff0d4daeb7beb9e90f64f19f560191e15bd8d387c16c744d002409` |
| ライセンス | NAIST-2003 (条文は [THIRD_PARTY_NOTICES.md](../THIRD_PARTY_NOTICES.md)) |

NEologd の語彙を含む辞書 (ipadic-neologd など) は同梱しません。`noslop dict download <名前>` で hasami の share ディレクトリに取得すると、辞書を指定しないとき (`dictionary = "auto"`) に、この同梱の辞書より先に使われます (いくつかあれば hasami の推奨順)。名前で固定するなら `--dict share:<名前>` か設定の `dictionary` で指定し、同梱の辞書に固定するなら `bundled` を指定します。`noslop dict download ipadic` で取れる `ipadic.hsd` は、取得に使う目録の版のもので、この同梱の辞書とは版が違うことがあります。

取得に使う目録 (`dict/catalog.json`) は、Release のジョブが hasami の最新のリリースに合わせて自動で更新します (手で更新するなら `make dict-catalog`)。同梱の辞書は目録とは別に、依存の hasami (`Cargo.toml` のタグ) と常に同じリリースのものにします。hasami を上げたら、同梱の辞書も必ず上げます。版が合っていないと `make ci` のテストが落ちます。

## 同梱の辞書を上げる手順

1. `make hasami-update TAG=<タグ>` (省くと最新のリリース) を実行する。`Cargo.toml` の `[workspace.dependencies]` にある hasami のタグと `Cargo.lock` を上げ、同じリリースの `ipadic.hsd` に置き換えて、固定情報 (`dict/bundled.json`)・上の表・THIRD_PARTY_NOTICES.md の版を書き換える。依存の更新ツールなどでタグだけが上がったときは、`make dict-bundled-update` で固定情報と表示を追いつかせる
   - 辞書は作業用のディレクトリに取得し、大きさと SHA-256 をリリースの `dictionaries.json` と照らす。依存の hasami で読み、作った hasami の版がタグと合うことも確かめてから置き換える (`tools/dict_bundled.rs`)。どれかが合わなければ、何も書き換えない
   - 例外表の NOTICE が変わっていれば、THIRD_PARTY_NOTICES.md の写しを直すよう知らせて止まる。辞書のメタデータの `ipadic_patch` が変わったときは、THIRD_PARTY_NOTICES.md の mecab-ipadic からの変更点の記述を確かめるよう知らせる。hasami の `rust-version` が上がったときも知らせるので、`Cargo.toml` の `rust-version` をそろえる
2. `make ci` を通す。テストが、同梱の辞書の版と依存の hasami の版 (`the_bundled_dictionary_is_ipadic_from_hasami`)、固定情報・上の表・THIRD_PARTY_NOTICES.md の版と辞書の中身 (`the_bundled_dictionary_matches_dict_readme`)、workspace のタグと固定情報 (`the_hasami_dependencies_point_to_the_linked_release`) を照らす。表の出所・形式・元のデータ・語数・SHA-256 の行は、スクリプトが書き換えてテストが読むので、書式を変えない。例外表の版を固定したテスト (`segment.rs`) が落ちたら、文分割の差分を確かめてから値を書き換える
3. 手元の文書で、品詞で数えるルール (P15・P16・R18・R20) の指摘を、上げる前と後のバイナリで比べる (`--dict bundled` を指定する)。差は上げるかどうかの判断には使わず、記録として残す。差があれば、ルールの説明文の「根拠」を見直す
