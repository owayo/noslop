# AI エージェントへの導入

[README に戻る](../README.md)

## AI エージェントと使う

文章を書いた AI エージェントに、noslop の指摘をそのまま返せます。どの方法でも渡すのは「どこを、なぜ見直すか」と改稿の制約で、件数を減らすこと自体を目的にしないよう断っています。

| 方法 | 使いどころ |
|------|-----------|
| `noslop check --format brief` | 検査結果を AI や編集者に貼り付けて直してもらう（JSON・TOON なら `--report brief --format json\|toon`） |
| `noslop skill-install <claude\|codex>` | エージェントに、日本語の文章を書いた・直した後に noslop で見直す手順（スキル）を覚えさせる |
| `noslop mcp` | Claude Code・Codex CLI などのエージェントが、自分で検査（`check`）・改稿の前後の比較（`diff`）・ルールの説明（`explain`）・一覧（`rules`）を呼ぶ。`check` は改稿指示を Markdown（既定）・JSON・TOON で返す |
| `noslop hook claude-code` | Claude Code がファイルを書いた直後（変わった行）、gws で Google ドキュメント・スプレッドシートに書き込む前（書き込む値）、応答を終えたとき（リポジトリのコミットしていない変更）に、指摘を自動で渡す |
| `noslop hook command` | claw-hooks のコマンドフックから、gws で書き込む値を書き込む前に検査する。claw-hooks が解析するので、`bash -c`・`sudo` の中の gws も読め、Codex CLI などほかのエージェントでも止められる |
| `noslop hook file <PATH>`・`noslop hook git-diff` | フックの入力を渡せない仕組み（claw-hooks の extension_hooks・stop_hooks など）から、同じ指摘をテキストで渡す |

```bash
# スキルを入れる（~/.claude/skills/noslop/SKILL.md、~/.codex/skills/noslop/SKILL.md）
noslop skill-install claude
noslop skill-install codex

# MCP サーバーを登録する
claude mcp add noslop -- noslop mcp
codex mcp add noslop -- noslop mcp
```

スキルを入れると、「この文章を自然な日本語に推敲して」「AI っぽさを抜いて」のような依頼で、エージェントが `noslop check --report brief --format toon` で直す箇所を受け取り、直した後に `noslop diff` で確かめるようになります。スキルの本文は [skills/SKILL.md](../skills/SKILL.md) です。

フックは `.claude/settings.json` などに書きます（[examples/claude-code-settings.json](../examples/claude-code-settings.json)）。

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
- **PreToolUse (Bash)**: gws で Google ドキュメント・スプレッドシートに書き込む値を、書き込む前に検査します。ドキュメントの本文に指摘があれば 1 度だけ書き込みを止め、直すか、残すと決めて同じコマンドを打ち直すかを Claude に任せます。セルのような短い値は止めずに、指摘を添えます。[claw-hooks](https://github.com/owayo/claw-hooks) を使っているなら、この PreToolUse の代わりに claw-hooks のコマンドフック（`noslop hook command`）で同じ検査ができます（両方に登録すると同じ書き込みを 2 回検査するので、どちらか一方にします）
- **Stop**: 作業ディレクトリを含む git の作業ツリーの、コミットしていない変更（HEAD との差分と追跡していないファイル）の変わった行に重なる指摘を返します。Bash で書き換えたファイルのように、PostToolUse を通らなかった変更も拾えます

対応する MCP の版、フックが返す範囲と上限、claw-hooks から呼ぶ設定などの詳細は [docs/integrations.md](../docs/integrations.md) にあります。
