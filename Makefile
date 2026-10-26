# ikari-ime の入口（薄い包み。中身は scripts/ と macos/scripts/ のシェルスクリプト）。
# 使い方は `make help`。署名は SIGNING=adhoc（既定。Apple の開発者アカウントは要らない）か SIGNING=team。
# 他の人が自分の名前で作るときは IKARI_ID_PREFIX=io.github.<名前> TEAM_ID=<チームID> を付ける（ADR 0036 §3.3）。

SIGNING ?= adhoc

.DEFAULT_GOAL := help
.PHONY: help setup setup-lm-off build install check clean

help: ## この一覧を出す
	@awk -F':.*## ' '/^[a-z-]+:.*## /{printf "  make %-14s %s\n", $$1, $$2}' $(MAKEFILE_LIST)
	@echo "  SIGNING=adhoc|team  署名（既定: adhoc）"

setup: ## 取得 → 辞書 → データ → アプリのビルドと入れ替え → smoke を通す（LM あり。初回は通信と数 GB が要る）
	scripts/setup.sh --fetch --signing $(SIGNING)

setup-lm-off: ## 同上、LM を使わない構成（cmake とモデルが要らない）
	scripts/setup.sh --fetch --no-lm --signing $(SIGNING)

build: ## アプリのビルドと署名だけ（入れ替えはしない）
	macos/scripts/build.sh --signing $(SIGNING) --release

install: ## ビルドして ~/Library/Input Methods に入れ替える
	macos/scripts/build.sh --signing $(SIGNING) --release
	macos/scripts/install.sh --signing $(SIGNING) --release

check: ## fmt、clippy、テスト、文書のリンク（コミットの前に）
	scripts/check.sh

clean: ## ビルドの生成物を消す（data/ は消さない）
	cargo clean
	rm -rf macos/.build
