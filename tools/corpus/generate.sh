#!/usr/bin/env bash
# 校正用の生成文書を、お題 1 件につき 1 ファイル作る (noslop calibrate の --ai に渡す)。
#
# 使い方:
#   tools/corpus/generate.sh <model> <out-dir> <topics-file>
#
#   model        claude | claude:<モデル名> | codex | codex:<モデル名>
#                (claude -p か codex exec で生成する。モデル名を省くと各 CLI の既定)
#   out-dir      出力先。お題ごとに <slug>.md を書く。中身のあるファイルは作り直さない
#   topics-file  お題の一覧。1 行に「slug: お題」。空行と # で始まる行は読み飛ばす
#
# 環境変数:
#   GENRE   general / tech / business / essay (既定 general)。文章の種類の指示に使う
#   LENGTH  目安の字数 (既定 2000)。人の文書の字数の中央値に合わせる
#
# 例:
#   GENRE=tech LENGTH=2500 tools/corpus/generate.sh claude:sonnet corpus/tech/ai-claude corpus/tech/topics.txt
#
# お題 1 件ごとに LLM を呼ぶため、件数によっては数十分から数時間かかる。手元の端末で
# 実行すること。途中で止めても、同じコマンドをもう一度実行すれば続きから作る。
#
# 利用者の設定 (CLAUDE.md・スキル・フック・AGENTS.md など) で文体が変わると、校正の前提
# (素の生成文書) が崩れる。claude は --safe-mode、codex は利用者の設定を読まずに動かす。
# 生成した文書はリポジトリにコミットしない (docs/calibration.md)。

set -euo pipefail

usage() {
  sed -n '2,24p' "$0" | sed 's/^# \{0,1\}//' >&2
  exit 2
}

die() {
  echo "generate.sh: $*" >&2
  exit 2
}

trim() {
  local s=$1
  s=${s#"${s%%[![:space:]]*}"}
  s=${s%"${s##*[![:space:]]}"}
  printf '%s' "$s"
}

[[ $# -eq 3 ]] || usage
model_spec=$1
out_dir=$2
topics=$3
genre=${GENRE:-general}
length=${LENGTH:-2000}

case $genre in
  general) kind="一般の読者に向けた記事" ;;
  tech) kind="技術ブログの記事" ;;
  business) kind="社内向けの報告書" ;;
  essay) kind="エッセイ" ;;
  *) die "GENRE は general / tech / business / essay のどれかにしてください: $genre" ;;
esac
[[ $length =~ ^[1-9][0-9]*$ ]] || die "LENGTH には字数 (正の整数) を指定してください: $length"
[[ -f $topics ]] || die "お題の一覧が見つかりません: $topics"

tool=${model_spec%%:*}
model=""
if [[ $model_spec == *:* ]]; then
  model=${model_spec#*:}
  [[ -n $model ]] || die "モデル名が空です: $model_spec"
fi
case $tool in
  claude | codex) command -v "$tool" >/dev/null 2>&1 || die "$tool コマンドが見つかりません" ;;
  *) die "model は claude / claude:<モデル名> / codex / codex:<モデル名> のどれかにしてください: $model_spec" ;;
esac

mkdir -p "$out_dir"
out_abs=$(cd "$out_dir" && pwd)

# お題 1 件を生成して $2 に書く。
generate() {
  local prompt=$1 part=$2
  case $tool in
    claude)
      local args=(-p --safe-mode --tools "" --no-session-persistence --output-format text)
      if [[ -n $model ]]; then
        args+=(--model "$model")
      fi
      (cd "$out_abs" && claude "${args[@]}" "$prompt") </dev/null >"$part"
      ;;
    codex)
      # 最後の応答だけを -o で受け取る。作業ディレクトリの AGENTS.md も読ませない
      local args=(exec --skip-git-repo-check --ephemeral --sandbox read-only
        --ignore-user-config --ignore-rules -c project_doc_max_bytes=0
        -C "$out_abs" -o "$part")
      if [[ -n $model ]]; then
        args+=(-m "$model")
      fi
      codex "${args[@]}" "$prompt" </dev/null >/dev/null
      ;;
  esac
}

total=$(grep -cvE '^[[:space:]]*(#|$)' "$topics" || true)
echo "お題 ${total} 件を ${model_spec} で生成します (${genre}、${length} 字前後)。1 件に数十秒から数分かかります" >&2

made=0
skipped=0
failed=0
count=0
# 生成のコマンドが標準入力を読まないよう、お題の一覧は別の記述子から読む
while IFS= read -r line <&3 || [[ -n $line ]]; do
  line=${line%$'\r'}
  trimmed=$(trim "$line")
  [[ -z $trimmed || $trimmed == \#* ]] && continue
  count=$((count + 1))
  slug=$(trim "${trimmed%%:*}")
  topic=$(trim "${trimmed#*:}")
  if [[ $trimmed != *:* || ! $slug =~ ^[A-Za-z0-9][A-Za-z0-9._-]*$ || -z $topic ]]; then
    echo "[$count/$total] 読み飛ばします (「slug: お題」の形ではありません): $line" >&2
    failed=$((failed + 1))
    continue
  fi
  target="$out_abs/$slug.md"
  if [[ -s $target ]]; then
    skipped=$((skipped + 1))
    continue
  fi
  prompt="次のお題で、${kind}を日本語で書いてください。

お題: ${topic}
長さ: ${length} 字前後

本文だけを Markdown で出力してください。前置きや、書き終えたあとの説明は要りません。"
  # 書きかけのファイルは隠しファイルにしておき、書き終えてから名前を変える
  # (noslop は隠しファイルを読まないので、途中で止めても校正に混ざらない)
  part="$out_abs/.$slug.md.part"
  echo "[$count/$total] $slug" >&2
  if generate "$prompt" "$part" && [[ -s $part ]]; then
    mv -f "$part" "$target"
    made=$((made + 1))
  else
    echo "[$count/$total] 生成に失敗しました: $slug" >&2
    failed=$((failed + 1))
  fi
done 3<"$topics"

echo "作成 ${made} 件、既存のため省略 ${skipped} 件、失敗 ${failed} 件 (${out_dir})" >&2
[[ $failed -eq 0 ]]
