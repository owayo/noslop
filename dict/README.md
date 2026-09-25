# 同梱の形態素解析の辞書

`ipadic.hsd` は、形態素解析器 [hasami](https://github.com/owayo/hasami) の辞書です。noslop は既定の feature `bundled-dict` でこれをバイナリに埋め込み、品詞で数えるルール (P15・P16) に使います。

| 項目 | 値 |
|---|---|
| 出所 | hasami v26.9.103 のリリースに添付された `ipadic.hsd` |
| 元のデータ | mecab-ipadic 2.7.0-20070801 (辞書のメタデータ `sources=ipadic@61b90ba6e669`) |
| 語数 | 390,849 |
| 大きさ | 18,125,804 バイト |
| SHA-256 | `e917bcdcdb45893fb4dd9b2de88ccb11dba2ecad2471dd0f62bd674a7f89ed73` |
| ライセンス | NAIST-2003 (条文は [THIRD_PARTY_NOTICES.md](../THIRD_PARTY_NOTICES.md)) |

NEologd の語彙を含む辞書 (ipadic-neologd など) は 220MB を超えるので同梱しません。使いたい場合は `noslop dict download <名前>` で hasami の share ディレクトリに取得し、`--dict share:<名前>` か設定の `dictionary` で指定します。これと同じ `ipadic.hsd` も `noslop dict download ipadic` で取得できます。

## 更新の手順

1. hasami の新しいタグのリリースから、`ipadic.hsd` と `dictionaries.json` を作業用のディレクトリに取得する (`gh release download <タグ> -R owayo/hasami -p ipadic.hsd -p dictionaries.json -D <ディレクトリ>`)
2. このディレクトリの `ipadic.hsd` を置き換え、上の表を書き換える。SHA-256 は `shasum -a 256 dict/ipadic.hsd` で求め、`dictionaries.json` の ipadic の `sha256` と一致することを確かめる (大きさは `size`、出典は `sources` にある)。語数は、hasami の clone で `cargo run --release -- info -d <ipadic.hsd のパス>` を実行すると表示される
3. `src/morph.rs` のテストに固定した辞書の大きさ・語数・出典を書き換える。`src/dictionaries.rs` の `HASAMI_TAG`・`DEFAULT_SOURCE` (タグのリリースの添付ファイルの URL)・`DICTIONARIES` (3 つの配布辞書の大きさと SHA-256。`dictionaries.json` の `size` と `sha256`) も書き換える (テストが、この `ipadic.hsd` と表の ipadic の一致を確かめる)
4. 手元の文書で P15・P16 の指摘の差分を確かめてから `make ci` を通す
