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
FAR=$((NOW + 400000))     # ~4.6 days out -> "4d" / "4d 15h", stable across a 1s skew

strip() { sed 's/\x1b\[[0-9;]*m//g'; }

pass=0 fail=0
# check:  strip ANSI before diffing — for near/hit fixtures whose countdown
#         blink (now % 2) is time-dependent. Verifies layout, not color.
# checkc: diff WITH ANSI intact — for normal-mode fixtures, whose color is
#         fully deterministic. This is what actually verifies color_for()
#         thresholds and bar fill (e.g. boundary-60 differs only in hue).
_diff() {
  local name=$1 json=$2 filter=$3 b r
  b=$(printf '%s' "$json" | bash "$BASH_SL" | $filter)
  r=$(printf '%s' "$json" | "$RUST_SL"     | $filter)
  if [ "$b" = "$r" ]; then
    pass=$((pass + 1)); printf '  ok   %s\n' "$name"
  else
    fail=$((fail + 1))
    printf '  FAIL %s\n     bash: %q\n     rust: %q\n' "$name" "$b" "$r"
  fi
}
check()  { _diff "$1" "$2" strip; }
checkc() { _diff "$1" "$2" cat; }

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

# color + unit thresholds — static below 95%, so diff WITH ANSI intact.
# (five is the varied metric; seven is a fixed green control.)
checkc "green-30"          "$(j 50 30 "$FAR" 5 "$FAR")"   # green, 1-unit
checkc "green-75"          "$(j 50 75 "$FAR" 5 "$FAR")"   # 75 -> still green
checkc "yellow-76"         "$(j 50 76 "$FAR" 5 "$FAR")"   # 76 -> yellow
checkc "yellow-80"         "$(j 50 80 "$FAR" 5 "$FAR")"
checkc "yellow-90"         "$(j 50 90 "$FAR" 5 "$FAR")"   # 90 -> still yellow, 1-unit
checkc "red-91-2unit"      "$(j 50 91 "$FAR" 5 "$FAR")"   # 91 -> red, 2-unit, static
checkc "red-94-2unit"      "$(j 50 94 "$FAR" 5 "$FAR")"
checkc "context-collapse"  "$(j 96 92 "$FAR" 5 "$FAR")"   # a red limit collapses ctx to %

# flashing zone (>=95) — blink is time-dependent, so strip ANSI (layout only)
check  "flash-95"          "$(j 50 95 "$FAR" 5 "$FAR")"   # red, flashing, bar still shown
check  "flash-99"          "$(j 50 99 "$FAR" 5 "$FAR")"
check  "over-100"          "$(j 50 100 "$FAR" 5 "$FAR")"  # bar dropped -> countdown + ⛔
check  "over-135"          "$(j 50 135 "$FAR" 5 "$FAR")"
check  "over-both"         "$(j 88 100 "$FAR" 105 "$FAR")"

# missing metrics / fallbacks — static, so diff WITH ANSI intact
checkc "missing-five"      "$(j 50 "" "" 50 "$FAR")"
checkc "missing-seven"     "$(j 50 40 "$FAR" "" "")"
checkc "missing-both-rl"   '{"model":{"display_name":"Sonnet 5"},"workspace":{"current_dir":"/x/y"},"context_window":{"used_percentage":33}}'
checkc "missing-model-cwd" '{"context_window":{"used_percentage":10}}'
checkc "cwd-toplevel-only" '{"cwd":"/var/log/app","context_window":{"used_percentage":5},"rate_limits":{"five_hour":{"used_percentage":20,"resets_at":0}}}'
checkc "haiku-family"      "$(j 5 5 "$FAR" 5 "$FAR" 'Haiku 4.5')"

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
