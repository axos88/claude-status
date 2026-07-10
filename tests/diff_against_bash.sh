#!/usr/bin/env bash
# Differential test: the Rust binary must render identically to the original
# statusline.sh for every branch. We strip ANSI codes before diffing because
# the per-second blink (now % 2) and countdown seconds are time-dependent;
# resets are placed far in the future so the two-unit clock stays stable.
set -u

HERE=$(cd "$(dirname "$0")/.." && pwd)
BASH_SL="$HERE/statusline.sh"
RUST_SL="$HERE/target/release/claude-status"

[ -x "$RUST_SL" ] || { echo "build first: cargo build --release"; exit 2; }

NOW=$(date +%s)
FAR=$((NOW + 400000))     # ~4.6 days out -> "4d Nh", stable across a 1s skew
SOON=$((NOW + 7000))      # ~1h57m       -> "1h 56m"

strip() { sed 's/\x1b\[[0-9;]*m//g'; }

pass=0 fail=0
check() {
  local name=$1 json=$2
  local b r
  b=$(printf '%s' "$json" | bash "$BASH_SL" | strip)
  r=$(printf '%s' "$json" | "$RUST_SL"     | strip)
  if [ "$b" = "$r" ]; then
    pass=$((pass + 1)); printf '  ok   %s\n' "$name"
  else
    fail=$((fail + 1))
    printf '  FAIL %s\n     bash: %s\n     rust: %s\n' "$name" "$b" "$r"
  fi
}

j() {  # build a status JSON with named knobs; omit a metric by passing ""
  local ctx=$1 five=$2 five_r=$3 seven=$4 seven_r=$5
  local model=${6-'Opus 4.8 (1M context)'} cwd=${7-'/home/akos/projects/lvgl'}
  printf '{"model":{"display_name":"%s"},"workspace":{"current_dir":"%s"},"context_window":{"used_percentage":%s}' "$model" "$cwd" "$ctx"
  printf ',"rate_limits":{'
  local sep=""
  [ -n "$five" ]  && { printf '"five_hour":{"used_percentage":%s,"resets_at":%s}' "$five" "$five_r"; sep=","; }
  [ -n "$seven" ] && { printf '%s"seven_day":{"used_percentage":%s,"resets_at":%s}' "$sep" "$seven" "$seven_r"; }
  printf '}}'
}

check "normal-both"        "$(j 42 30 "$FAR" 55 "$FAR")"
check "near-five-only"     "$(j 42 93 "$SOON" 55 "$FAR")"
check "near-both"          "$(j 77 95 "$SOON" 92 "$FAR")"
check "hit-five"           "$(j 60 100 "$SOON" 70 "$FAR")"
check "hit-both"           "$(j 88 100 "$SOON" 100 "$FAR")"
check "missing-five"       "$(j 42 "" "" 50 "$FAR")"
check "missing-seven"      "$(j 42 40 "$FAR" "" "")"
check "missing-both-rl"    '{"model":{"display_name":"Sonnet 5"},"workspace":{"current_dir":"/x/y"},"context_window":{"used_percentage":33}}'
check "missing-model-cwd"  '{"context_window":{"used_percentage":10}}'
check "cwd-toplevel-only"  '{"cwd":"/var/log/app","context_window":{"used_percentage":5},"rate_limits":{"five_hour":{"used_percentage":20,"resets_at":0}}}'
check "boundary-60"        "$(j 60 60 "$FAR" 59 "$FAR")"
check "boundary-85"        "$(j 85 84 "$FAR" 85 "$FAR")"
check "haiku-family"       "$(j 5 5 "$FAR" 5 "$FAR" 'Haiku 4.5')"

# --- malformed-input robustness (intentional divergence from bash) -----------
# On empty/garbage input the bash script coerces "" -> 0 and renders phantom 0%
# rate-limit bars with a blank model. The Rust port instead degrades to
# documented defaults: "?" model/folder, and omits absent rate-limit metrics.
expect() {
  local name=$1 json=$2 want=$3 got
  got=$(printf '%s' "$json" | "$RUST_SL" | strip)
  if [ "$got" = "$want" ]; then
    pass=$((pass + 1)); printf '  ok   %s\n' "$name"
  else
    fail=$((fail + 1))
    printf '  FAIL %s\n     want: %s\n     got:  %s\n' "$name" "$want" "$got"
  fi
}
expect "empty-input"   ''       '🤖 ?  📁 ?  🧠 [░░░░░░░░░░]   0%'
expect "garbage-input" 'not json' '🤖 ?  📁 ?  🧠 [░░░░░░░░░░]   0%'

echo "---"
echo "pass=$pass fail=$fail"
[ "$fail" -eq 0 ]
