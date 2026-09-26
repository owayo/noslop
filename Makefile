# noslop の開発用タスク。引数なしの `make` でターゲット一覧を表示する。
#
# ツールの版は mise.toml が正。mise があればコマンドを `mise exec --` 経由で呼ぶので、
# シェルで mise を activate していなくても (IDE や GUI から make を呼んでも)
# mise.toml の版で動く。mise を使わず PATH 上のツールで動かすなら SYSTEM_TOOLS=1 を
# 付ける (その場合、版の再現性は保証しない)。
#
# CI (.github/workflows/ci.yml) の Linux と macOS のジョブは make setup と make ci だけを
# 呼ぶ。検査を足すときは ci に足し、workflow に検査のコマンドを並べない。Windows の build ジョブ
# だけはランナーの make (mingw32-make) を避けて、ci と同じコマンドを直接呼んでいる。
#
# ターゲットの説明 (## の後ろ) は make help がそのまま表示し、README の「開発」の表にも同じ文を
# 載せている。説明を変えたら README の表もそろえる。
#
# macOS 標準の GNU Make 3.81 で動く書き方に限っている
# (.ONESHELL / .SHELLFLAGS / $(file ...) / != は使わない)。

.PHONY: help setup build release run install uninstall test test-no-default-features lint clippy fmt fmt-check check docs docs-check ci clean dict-catalog dict-check

# 引数なしの make はヘルプを表示する
.DEFAULT_GOAL := help

# 変数
BINARY_NAME := noslop
INSTALL_PATH ?= /usr/local/bin
# Cargo.lock をコミットしているので、依存の解決結果を CI・リリースとそろえる。
# Cargo.lock を更新したいとき (依存を足した直後など) は CARGO_FLAGS= で外す
CARGO_FLAGS ?= --locked
# make install でスキルを入れる AI エージェント (空にすると入れない: make install SKILL_TARGETS=)
SKILL_TARGETS ?= claude codex
# docs と docs-check が生成したルールの一覧を置く場所。生成に失敗したときに
# docs/rules.md を空にしないよう、いったんここに書く
RULES_MD_TMP := target/rules.md
# make dict-catalog で取り込む hasami のリリースのタグ (例: TAG=v26.9.110。空なら最新のリリース)
TAG ?=

# ---- ツールチェーン -----------------------------------------------------------
# mise は PATH、よくある導入先の順に探す。GUI から起動した make はシェルの PATH を
# 引き継がないことがあるため。make MISE=/path/to/mise で明示もできる。
# mise が無い環境の振る舞いを試すときは MISE_CANDIDATES= で探す先を空にする。
MISE_CANDIDATES ?= $(HOME)/.local/bin/mise /opt/homebrew/bin/mise /usr/local/bin/mise
ifeq ($(SYSTEM_TOOLS),1)
RUN :=
else
ifndef MISE
MISE := $(firstword $(shell command -v mise 2>/dev/null) $(wildcard $(MISE_CANDIDATES)))
endif
ifeq ($(MISE),)
ifneq ($(filter-out help,$(or $(MAKECMDGOALS),help)),)
$(error mise が見つかりません。https://mise.jdx.dev で入れるか、PATH 上のツールで動かすなら SYSTEM_TOOLS=1 を付けてください)
endif
endif
RUN := $(if $(MISE),$(MISE) exec --,)
endif

## セットアップ

setup: ## ツールチェーン (mise) と依存を取得する
	@if [ -n "$(MISE)" ]; then "$(MISE)" install; fi
	$(RUN) cargo fetch $(CARGO_FLAGS)

## ビルド

build: ## デバッグ版をビルドする
	$(RUN) cargo build $(CARGO_FLAGS)

release: ## リリース版をビルドする
	$(RUN) cargo build --release $(CARGO_FLAGS)

run: ## デバッグ版を実行する (引数は ARGS="...")
	$(RUN) cargo run $(CARGO_FLAGS) -- $(ARGS)

## インストール

# 上書きコピーではなく一時ファイル + rename で置き換える。
# macOS はコード署名の検証結果をパス/inode 単位でキャッシュするため、実行中または
# 直前に実行されたバイナリへ cp で上書きすると、キャッシュ済みの CDHash と中身が
# 食い違って新しいバイナリが起動直後に SIGKILL される (exit 137)。
# 一時ファイルは rename が inode の差し替えになるよう、同じディレクトリに置く。
# スキル (skills/SKILL.md) は入れたばかりのバイナリで書き出すので、バイナリと版がそろう。
install: release ## リリース版を INSTALL_PATH (既定 /usr/local/bin) に入れる (スキルも入れる)
	@mkdir -p "$(INSTALL_PATH)"
	cp "target/release/$(BINARY_NAME)" "$(INSTALL_PATH)/$(BINARY_NAME).new"
	mv -f "$(INSTALL_PATH)/$(BINARY_NAME).new" "$(INSTALL_PATH)/$(BINARY_NAME)"
	@for target in $(SKILL_TARGETS); do \
		"$(INSTALL_PATH)/$(BINARY_NAME)" skill-install "$$target" || exit 1; \
	done

# バイナリだけを消し、スキルは消さない。スキルの置き場所はエージェントごとに違い、
# 別の版で入れたものまで消してしまうため
uninstall: ## INSTALL_PATH から取り除く (スキルは残す)
	rm -f "$(INSTALL_PATH)/$(BINARY_NAME)"

## 開発

test: ## テストを実行する
	$(RUN) cargo test $(CARGO_FLAGS)

# 辞書を同梱しないビルド (#[cfg(not(feature = "bundled-dict"))] の側) をコンパイルしてテストする。
# cargo は feature を切り替えるたびに target/debug/noslop を置き直すので、これを単独で
# 回した後の target/debug/noslop は辞書を同梱しない版になる (次に既定の feature で
# build・run・test すると、作り直さずに既定の版へ戻る)
test-no-default-features: ## 辞書を同梱しないビルドでテストする (--no-default-features)
	$(RUN) cargo test $(CARGO_FLAGS) --no-default-features

lint: ## clippy を警告ゼロで通す
	$(RUN) cargo clippy $(CARGO_FLAGS) --all-targets -- -D warnings

clippy: lint ## lint の別名

fmt: ## コードを整形する (書き換える)
	$(RUN) cargo fmt --all

fmt-check: ## 整形済みかを確かめる (書き換えない)
	$(RUN) cargo fmt --all -- --check

check: fmt-check lint ## 整形と静的検査 (書き換えない)

# 手元の noslop.toml の有効・無効が一覧に混ざらないよう、設定ファイルは読まない
docs: ## 組み込みのルールから docs/rules.md を作り直す
	@mkdir -p target docs
	$(RUN) cargo run --quiet $(CARGO_FLAGS) -- rules --format markdown --no-config > $(RULES_MD_TMP)
	mv -f $(RULES_MD_TMP) docs/rules.md

# ルールの定義や説明文を変えたのに make docs を忘れた変更を落とす (docs/rules.md は書き換えない)
docs-check: ## docs/rules.md が最新かを確かめる (書き換えない)
	@mkdir -p target
	$(RUN) cargo run --quiet $(CARGO_FLAGS) -- rules --format markdown --no-config > $(RULES_MD_TMP)
	@diff -u docs/rules.md $(RULES_MD_TMP) || { echo "docs/rules.md が古くなっています。make docs で作り直してください" >&2; exit 1; }

# CI の Linux と macOS のジョブはこれを呼ぶ。書き換えを含めない。
# 同梱しないビルドのテストを先に回し、既定の feature の test と docs-check を後に置く。
# こうすると make ci の後の target/debug/noslop が既定の版 (辞書を同梱) になる
ci: check test-no-default-features test docs-check ## CI と同じ検査 (書き換えない)

clean: ## ビルド成果物を消す
	$(RUN) cargo clean

## 配布辞書の目録

# dict/catalog.json は hasami のリリースに添付された dictionaries.json をそのまま置いたもの。
# build.rs がこれから取得元のタグ・URL・大きさ・SHA-256 を作る。Release のワークフローも
# リリースの前にこれを呼び、変わっていれば make ci と dict-check を通してから一緒にコミットする。
# gh (GitHub CLI) を使う
dict-catalog: ## 配布辞書の目録 (dict/catalog.json) を hasami のリリースに合わせる (TAG=... で版を指定、省くと最新)
	tools/dict-catalog.sh $(TAG)

# 通信が要り重い (3 つの辞書の圧縮版で約 146MB と、展開前の ipadic の約 18MB を取得する) ので、ci には入れない
dict-check: ## 目録の辞書を実際に取得し (圧縮版を展開する)、大きさ・SHA-256・読めることを確かめる (通信が要る)
	$(RUN) cargo test $(CARGO_FLAGS) --lib dictionaries::tests:: -- --ignored

## ヘルプ

help: ## このヘルプを表示する
	@echo "$(BINARY_NAME) の開発用タスク"
	@echo ""
	@echo "使い方: make <ターゲット>"
	@echo ""
	@echo "ターゲット:"
	@grep -E '^[a-zA-Z0-9_-]+:.*?## .*$$' $(MAKEFILE_LIST) | awk 'BEGIN {FS = ":.*?## "}; {printf "  \033[36m%-26s\033[0m %s\n", $$1, $$2}'
	@echo ""
	@echo "ツールチェーン:"
	@echo "  版は mise.toml で固定している。初回は make setup"
	@echo "  コマンドは mise exec 経由で動く。PATH 上のツールを使うなら SYSTEM_TOOLS=1 を付ける"
	@echo ""
	@echo "リリース:"
	@echo "  GitHub Actions > Release > Run workflow"
