# 同梱の形態素解析の辞書

`ipadic.hsd` は、形態素解析器 [hasami](https://github.com/owayo/hasami) の辞書です。noslop は既定の feature `bundled-dict` でこれをバイナリに埋め込み、品詞で数えるルール (P15・P16) に使います。

| 項目 | 値 |
|---|---|
| 出所 | hasami v26.9.105 のリリースに添付された `ipadic.hsd` |
| 元のデータ | mecab-ipadic 2.7.0-20070801 (辞書のメタデータ `sources=ipadic@61b90ba6e669`) |
| 語数 | 390,849 |
| 大きさ | 18,125,804 バイト |
| SHA-256 | `1ca13555b1fc6ec12dd4b830aec97262b4481b70b7cfec6d2a9bea912e1d6277` |
| ライセンス | NAIST-2003 (条文は [THIRD_PARTY_NOTICES.md](../THIRD_PARTY_NOTICES.md)) |

NEologd の語彙を含む辞書 (ipadic-neologd など) は 220MB を超えるので同梱しません。`noslop dict download <名前>` で hasami の share ディレクトリに取得すると、辞書を指定しないとき (`dictionary = "auto"`) に、この同梱の辞書より先に使われます (いくつかあれば hasami の推奨順)。名前で固定するなら `--dict share:<名前>` か設定の `dictionary` で指定し、同梱の辞書に固定するなら `bundled` を指定します。`noslop dict download ipadic` で取れる `ipadic.hsd` は、取得に使う目録の版のもので、この同梱の辞書とは版が違うことがあります。

取得に使う目録 (`dict/catalog.json`) は、Release のジョブが hasami の最新のリリースに合わせて自動で更新します (手で更新するなら `make dict-catalog`)。同梱の辞書は判定の校正の前提なので、目録とは別に、次の手順で上げます。

## 同梱の辞書を上げる手順

1. hasami の新しいタグのリリースから、`ipadic.hsd` と `dictionaries.json` を作業用のディレクトリに取得する (`gh release download <タグ> -R owayo/hasami -p ipadic.hsd -p dictionaries.json -D <ディレクトリ>`)
2. このディレクトリの `ipadic.hsd` を置き換え、上の表を書き換える。SHA-256 は `shasum -a 256 dict/ipadic.hsd` で求め、`dictionaries.json` の ipadic の `sha256` と一致することを確かめる (大きさは `size`、出典は `sources` にある)。語数は、hasami の clone で `cargo run --release -- info -d <ipadic.hsd のパス>` を実行すると表示される。表の「大きさ」と「SHA-256」の行は、テスト (`the_bundled_dictionary_matches_dict_readme`) が `dict/ipadic.hsd` と照らすので、書式 (`| 大きさ | 18,125,804 バイト |`・``| SHA-256 | `<16 進>` |``) を変えない
3. `src/morph.rs` のテストに固定した辞書の大きさ・語数・出典を書き換える。依存の hasami (`Cargo.toml` のタグ) も上げるときは、AGENTS.md の技術スタックの手順に従う (目録は書き換えなくてよい)
4. 手元の文書で P15・P16 の指摘の差分を確かめてから `make ci` を通す
