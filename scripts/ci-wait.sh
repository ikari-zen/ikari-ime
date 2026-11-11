#!/bin/bash
# PR の CI（GitHub Actions）の終わりを待つ。人もエージェントも、待つ間は何も読まなくてよい（トークンを使わない）。
#
#   scripts/ci-wait.sh <PR 番号>                     終わるまで待つ。全部通れば終了コード 0、本物の失敗なら 1
#   scripts/ci-wait.sh <PR 番号> --max-reruns 5      ランナーを確保できなかったジョブを、最大 5 回まで再実行する（既定 3）
#   scripts/ci-wait.sh <PR 番号> --interval 60       確認の間隔（秒。既定 60）
#   scripts/ci-wait.sh <PR 番号> --timeout 5400      待つ時間の上限（秒。既定 5400 = 90 分）
#
# 再実行するのは、GitHub 側の都合で失敗したものだけ（ランナーを確保できない、ランナーが落ちた）。
# テストなどの本物の失敗は再実行せず、失敗したジョブの名前と URL を出して終わる。
# 終了コード: 0 = 全部通った、1 = 本物の失敗、2 = 再実行の上限、3 = 待ち時間の上限、64 = 使い方の誤り。
# 出力は、状態が変わったときと最後だけ（1 行ずつ）。
set -u

pr=""
max_reruns=3
interval=60
timeout=5400
while [ $# -gt 0 ]; do
  case "$1" in
    --max-reruns) max_reruns="${2:-}"; shift 2 ;;
    --interval) interval="${2:-}"; shift 2 ;;
    --timeout) timeout="${2:-}"; shift 2 ;;
    -h|--help) sed -n '2,13p' "$0"; exit 0 ;;
    -*) echo "知らないオプション: $1" >&2; exit 64 ;;
    *) pr="$1"; shift ;;
  esac
done
case "$pr" in ''|*[!0-9]*) echo "使い方: scripts/ci-wait.sh <PR 番号> [--max-reruns N] [--interval 秒] [--timeout 秒]" >&2; exit 64 ;; esac
for v in "$max_reruns" "$interval" "$timeout"; do
  case "$v" in ''|*[!0-9]*) echo "数値が要ります: $v" >&2; exit 64 ;; esac
done

# ジョブの失敗が、GitHub 側の都合（コードのせいではない）かどうか。run の ID を渡す。
is_infra_failure() {
  local text
  text=$(gh run view "$1" 2>&1) || return 1
  printf '%s' "$text" | grep -qE 'not acquired by Runner|runner has received a shutdown signal|lost communication with the server|hosted runner encountered an error'
}

start=$(date +%s)
reruns=0
last=""
while :; do
  head=$(gh pr view "$pr" --json headRefOid --jq '.headRefOid[0:7]' 2>/dev/null) || { echo "PR #$pr を読めない（gh の認証か番号）" >&2; exit 64; }
  # name、bucket（pass / fail / pending / skipping / cancel）、link を、タブ区切りで
  rows=$(gh pr checks "$pr" --json name,bucket,link --jq '.[] | [.name, .bucket, .link] | @tsv' 2>/dev/null || true)
  if [ -z "$rows" ]; then
    status="チェックがまだない"
  else
    pending=$(printf '%s\n' "$rows" | awk -F'\t' '$2=="pending"' | wc -l | tr -d ' ')
    failed=$(printf '%s\n' "$rows" | awk -F'\t' '$2=="fail" || $2=="cancel"')
    status="head=$head 待ち=$pending 失敗=$(printf '%s' "$failed" | grep -c . || true)"
  fi
  if [ "$status" != "$last" ]; then echo "[$(date +%H:%M)] #$pr $status"; last="$status"; fi

  if [ -n "$rows" ] && [ "$pending" = "0" ]; then
    if [ -z "$failed" ]; then
      # 再実行の直後は、実行中のジョブが一覧から消えていることがある。一覧ではなく、GitHub が決める
      # マージの状態（必須のチェックが全部通ると CLEAN。必須でない失敗だけなら UNSTABLE）で確かめる
      sleep 20
      state=$(gh pr view "$pr" --json mergeStateStatus --jq .mergeStateStatus 2>/dev/null || true)
      case "$state" in
        CLEAN|UNSTABLE|HAS_HOOKS)
          echo "全部通った（#$pr head=${head} 状態=$state）"
          exit 0 ;;
      esac
      continue
    fi
    # 失敗した各ジョブの run を調べる。全部が GitHub 側の都合なら再実行する
    all_infra=1
    runs=""
    while IFS=$'\t' read -r name _ link; do
      run=$(printf '%s' "$link" | sed -n 's#.*/actions/runs/\([0-9][0-9]*\).*#\1#p')
      if [ -z "$run" ] || ! is_infra_failure "$run"; then
        all_infra=0
        echo "本物の失敗: $name $link"
      else
        runs="$runs $run"
      fi
    done <<EOF
$failed
EOF
    if [ "$all_infra" = "0" ]; then exit 1; fi
    if [ "$reruns" -ge "$max_reruns" ]; then
      echo "ランナーの失敗が続いたので止める（再実行 $reruns 回）" >&2
      exit 2
    fi
    reruns=$((reruns + 1))
    for run in $(printf '%s\n' $runs | sort -u); do
      echo "[$(date +%H:%M)] GitHub 側の失敗。再実行する（$reruns/${max_reruns}）run=$run"
      gh run rerun "$run" --failed >/dev/null 2>&1 || true
    done
    last=""
    sleep 20
    continue
  fi

  if [ $(( $(date +%s) - start )) -ge "$timeout" ]; then
    echo "待ち時間の上限（${timeout} 秒）" >&2
    exit 3
  fi
  sleep "$interval"
done
