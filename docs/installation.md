# インストールの補足

[README に戻る](../README.md)

## インストール

### Homebrew (macOS・Linux)

```bash
brew install owayo/noslop/noslop
```

tap ([owayo/homebrew-noslop](https://github.com/owayo/homebrew-noslop)) を足して、Releases のビルド済みのバイナリを入れます。更新は `brew upgrade noslop` です。Claude Code・Codex CLI のスキルは入らないので、使うなら `noslop skill-install claude` (Codex CLI なら `codex`) を実行します ([AI エージェントと使う](agent-setup.md#ai-エージェントと使う))。

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

`make install` は、バイナリを入れたあと、そのバイナリで Claude Code と Codex CLI のスキル（`~/.claude/skills/noslop/SKILL.md`・`~/.codex/skills/noslop/SKILL.md`）も入れます（[AI エージェントと使う](agent-setup.md#ai-エージェントと使う)）。入れる先は `SKILL_TARGETS` で選べます（`make install SKILL_TARGETS=claude`、入れないなら `make install SKILL_TARGETS=`）。`make uninstall` はバイナリだけを取り除き、スキルは残します。
