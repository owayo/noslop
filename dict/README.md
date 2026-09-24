# 同梱の形態素解析の辞書

`ipadic.hsd` は、形態素解析器 [hasami](https://github.com/owayo/hasami) の辞書です。noslop は既定の feature `bundled-dict` でこれをバイナリに埋め込み、品詞で数えるルール (P15・P16) に使います。

| 項目 | 値 |
|---|---|
| 出所 | hasami v26.9.103 の `dict/ipadic.hsd` (Git LFS) |
| 元のデータ | mecab-ipadic 2.7.0-20070801 (辞書のメタデータ `sources=ipadic@61b90ba6e669`) |
| 語数 | 390,849 |
| 大きさ | 18,125,804 バイト |
| SHA-256 | `e917bcdcdb45893fb4dd9b2de88ccb11dba2ecad2471dd0f62bd674a7f89ed73` |
| ライセンス | NAIST-2003 (条文は [THIRD_PARTY_NOTICES.md](../THIRD_PARTY_NOTICES.md)) |

NEologd の語彙を含む辞書 (ipadic-neologd など) は 220MB を超えるので同梱しません。使いたい場合は `noslop dict download <名前>` で hasami の share ディレクトリに取得し、`--dict share:<名前>` か設定の `dictionary` で指定します。これと同じ `ipadic.hsd` も `noslop dict download ipadic` で取得できます。

## 更新の手順

1. hasami の新しいタグで `dict/ipadic.hsd` を取り出す (`git lfs install` のうえで clone する)
2. このディレクトリの `ipadic.hsd` を置き換え、上の表を書き換える。SHA-256 は `shasum -a 256 dict/ipadic.hsd` で求め、hasami のタグの LFS ポインタ (`git show <タグ>:dict/ipadic.hsd` の `oid`) と一致することを確かめる。語数と出典 (`sources`) は、hasami の clone で `cargo run --release -- info -d dict/ipadic.hsd` を実行すると表示される
3. `src/morph.rs` のテストに固定した辞書の大きさ・語数・出典を書き換える。`src/dictionaries.rs` の `HASAMI_TAG`・`DEFAULT_SOURCE`・`DICTIONARIES` (3 つの配布辞書の大きさと SHA-256。タグの LFS ポインタの `size` と `oid`) も書き換える (テストが、この `ipadic.hsd` と表の ipadic の一致を確かめる)
4. 手元の文書で P15・P16 の指摘の差分を確かめてから `mise exec -- make ci` を通す
