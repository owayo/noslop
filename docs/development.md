# 開発とリリース

[README に戻る](../README.md)

## 開発

[mise](https://mise.jdx.dev/) が必要です。ツールチェーンの版は `mise.toml` で固定しています。Makefile はツールを `mise exec` 経由で呼ぶので、シェルで `mise activate` を済ませていなくても `mise.toml` の版で動きます。

```bash
make setup   # mise.toml のツールチェーンを入れ（mise install）、依存を取得する
make ci      # CI と同じ検査
```

ターゲットの一覧は `make`（引数なし）でも表示できます。下の表の説明は、その出力と同じです。

| コマンド | 説明 |
|---|---|
| `make setup` | ツールチェーン (mise) と依存を取得する |
| `make build` | デバッグ版をビルドする |
| `make release` | リリース版をビルドする |
| `make run` | デバッグ版を実行する (引数は ARGS="...") |
| `make install` | リリース版を INSTALL_PATH (既定 /usr/local/bin) に入れる (スキルも入れる) |
| `make uninstall` | INSTALL_PATH から取り除く (スキルは残す) |
| `make test` | テストを実行する |
| `make test-no-default-features` | 辞書を同梱しないビルドでテストする (--no-default-features) |
| `make lint` | clippy を警告ゼロで通す |
| `make clippy` | lint の別名 |
| `make fmt` | コードを整形する (書き換える) |
| `make fmt-check` | 整形済みかを確かめる (書き換えない) |
| `make check` | 整形と静的検査 (書き換えない) |
| `make docs` | 組み込みのルールから docs/rules.md を作り直す |
| `make docs-check` | docs/rules.md が最新かを確かめる (書き換えない) |
| `make ci` | CI と同じ検査 (書き換えない) |
| `make clean` | ビルド成果物を消す |
| `make dict-catalog` | 配布辞書の目録 (dict/catalog.json) を hasami のリリースに合わせる (TAG=... で版を指定、省くと最新) |
| `make dict-check` | 目録の辞書を実際に取得し (圧縮版を展開する)、大きさ・SHA-256・読めることを確かめる (通信が要る) |
| `make help` | このヘルプを表示する |

`make ci` は、フォーマットの確認、clippy（警告はエラー）、テスト、`docs/rules.md` が最新かの確認、辞書を同梱しないビルドのテストを実行します。CI は Linux と macOS で `make setup` と `make ci` を実行し、Windows では make を使わず、`docs/rules.md` の確認以外の検査を cargo で直接実行します。ルールの定義や説明文を変えたら、`make docs` で `docs/rules.md` を作り直してください。忘れると `make ci` が失敗します。

cargo のコマンドには `--locked` を付け、`Cargo.lock` のとおりに依存を解決します。依存を足した直後など、`Cargo.lock` を更新したいときは `CARGO_FLAGS=` を付けます。mise を使わない場合は `SYSTEM_TOOLS=1` を付けると、`PATH` 上のツールで動きます（ツールの版はそろいません）。

## リリース

GitHub の Actions タブで Release ワークフローを選び、Run workflow で実行します。版は日本時間で `YY.M.COUNTER` の形（`26.9.100` など）で、その月の最初のリリースは COUNTER を 100 から始め、同じ月の 2 回目以降は 1 ずつ上げます。`dry_run` を有効にすると、版と辞書の目録を確認し、目録が変われば検査も回しますが、コミット・タグ・ビルド・公開はしません。リリースには Linux x86_64 / arm64、macOS x86_64 / arm64、Windows x86_64 のアーカイブ（`noslop-<ターゲット>.tar.gz`、Windows は `.zip`。中身はバイナリと `LICENSE`・`THIRD_PARTY_NOTICES.md`）と、`SHA256SUMS` が付きます。

公開の後、Homebrew の tap（[owayo/homebrew-noslop](https://github.com/owayo/homebrew-noslop)）の formula を、新しい版の URL と SHA-256 に書き換えて push します。tap への push には GitHub App のトークンを使います。設定する項目は [エージェント向けガイド](../AGENTS.md#リリース) を参照してください。設定が足りなければ、警告を出して tap の更新だけを飛ばします。

Release ワークフローは、`noslop dict download` が使う配布辞書の目録（`dict/catalog.json`）も hasami の最新のリリースに合わせます。目録が変わったときは、`make ci` と、3 つの辞書を実際に取得して確かめる `make dict-check` を通してから、版の更新と同じコミットに入れます。

## ロードマップ

- LSP（エディタ上でのリアルタイム表示）
- 形態素解析の辞書を使う判定を、埋もれた列挙（R04）と文頭の反復（R08）にも広げる
- `noslop calibrate` で集めたコーパスで辞書なしの近似を再校正し、実験的ルールを stable に上げる
- CI で前回の JSON 結果と比べ、新しく出た指摘だけを出す（ベースライン）
