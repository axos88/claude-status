#!/usr/bin/env bash
# Claude Code status line — shared by the default `claude` and `claude-akos` configs.
# Layout:  🤖 <family>  │  📁 <folder>  │  🧠 <ctx>  │  5h <…>  │  7d <…>
# Normal:  each usage metric shows a colored progress bar + %.
# Near/hit (>=90%): that metric's bar is replaced by its next reset clock time
#                   (yellow approaching, red once hit). When ANY limit is near,
#                   the context bar is dropped too — only its % remains.

input=$(cat)

# --- pull fields from the status-line JSON on stdin ---------------------------
IFS=$'\t' read -r MODEL CWD CTX FIVE_H FIVE_RESET SEVEN_D SEVEN_RESET < <(
  printf '%s' "$input" | jq -r '
    [ (.model.display_name // "?"),
      (.workspace.current_dir // .cwd // "?"),
      (.context_window.used_percentage // 0),
      (.rate_limits.five_hour.used_percentage // -1),
      (.rate_limits.five_hour.resets_at // 0),
      (.rate_limits.seven_day.used_percentage // -1),
      (.rate_limits.seven_day.resets_at // 0)
    ] | @tsv'
)

FOLDER=$(basename "$CWD")
CTX_INT=${CTX%.*}; F_INT=${FIVE_H%.*}; S_INT=${SEVEN_D%.*}

# model family only — drop version / context-length suffix (e.g. "Opus 4.8" -> "Opus")
case "$MODEL" in
  *Opus*)   MODEL="Opus"   ;;
  *Sonnet*) MODEL="Sonnet" ;;
  *Haiku*)  MODEL="Haiku"  ;;
  *Fable*)  MODEL="Fable"  ;;
  *)        MODEL="${MODEL%% *}" ;;
esac

dim=$'\033[2m'; reset=$'\033[0m'; bold=$'\033[1m'; cyan=$'\033[36m'; mag=$'\033[35m'
red=$'\033[31m'; yel=$'\033[33m'; grn=$'\033[32m'
# per-second countdown pulse palettes
byel=$'\033[93m'; brown=$'\033[38;5;130m'   # near mode: bright yellow <-> brown
dred=$'\033[38;5;88m'                        # hit mode:  red <-> dark red

# --- helpers ------------------------------------------------------------------
# threshold color for a percentage: green < 60, yellow < 85, red otherwise
color_for() {
  local pct=$1
  if   (( pct >= 85 )); then printf '%s' "$red"
  elif (( pct >= 60 )); then printf '%s' "$yel"
  else                       printf '%s' "$grn"
  fi
}

# colored progress bar: bar <percent> -> "[████░░░░░░] 42%"
bar() {
  local pct=${1%.*} width=10 filled i out="" color
  (( pct < 0 )) && pct=0
  (( pct > 100 )) && pct=100
  color=$(color_for "$pct")
  out="${color}"
  filled=$(( (pct * width + 50) / 100 ))
  for (( i = 0; i < filled; i++ )); do out+="█"; done
  out+="${dim}"
  for (( i = filled; i < width; i++ )); do out+="░"; done
  printf '%s[%s%s] %s%3d%%%s' "$reset" "$out" "$reset" "$color" "$pct" "$reset"
}

now=$(date +%s)

# time left, single most-significant unit: "5d" / "2h" / "50m" / "9s"
fmt_top() {
  local s=$1
  (( s < 0 )) && s=0
  local d=$(( s/86400 )) h=$(( (s%86400)/3600 )) m=$(( (s%3600)/60 )) sec=$(( s%60 ))
  if   (( d > 0 )); then printf '%dd' "$d"
  elif (( h > 0 )); then printf '%dh' "$h"
  elif (( m > 0 )); then printf '%dm' "$m"
  else                   printf '%ds' "$sec"
  fi
}

# time left, two most-significant units: "5d 2h" / "2h 1m" / "50m 13s" / "9s"
fmt_left() {
  local s=$1
  (( s < 0 )) && s=0
  local d=$(( s/86400 )) h=$(( (s%86400)/3600 )) m=$(( (s%3600)/60 )) sec=$(( s%60 ))
  if   (( d > 0 )); then printf '%dd %dh' "$d" "$h"
  elif (( h > 0 )); then printf '%dh %dm' "$h" "$m"
  elif (( m > 0 )); then printf '%dm %ds' "$m" "$sec"
  else                   printf '%ds' "$sec"
  fi
}

# usage metric:
#   normal (<90%) -> colored progress bar
#   near (90-99%) -> percentage + live countdown until reset
#   hit  (100%)   -> red error emoji + countdown (no percentage)
metric() {
  local label=$1 pct=$2 resets_at=$3 cd="" clock=""
  if (( resets_at > 0 )); then
    cd=$(fmt_left $(( resets_at - now )))
    clock=$(date -d "@$resets_at" +%H:%M 2>/dev/null)
  fi
  # alternate colors each second so feedback is perceived even when the
  # displayed digits don't change (e.g. in the "1h 23m" range)
  if (( pct >= 100 )); then
    local blink tail=""
    (( now % 2 == 0 )) && blink=$red || blink=$dred
    [ -n "$cd" ] && tail=" ${blink}${cd}${reset}"
    [ -n "$clock" ] && tail+=" ${dim}@ ${clock}${reset}"
    printf '%s%s%s ⛔%s' "$dim" "$label" "$reset" "$tail"
  elif (( pct >= 90 )); then
    local blink tail=""
    (( now % 2 == 0 )) && blink=$byel || blink=$brown
    [ -n "$cd" ] && tail=" ${blink}${cd}${reset}"
    printf '%s%s%s %s%d%%%s%s' "$dim" "$label" "$reset" "$yel" "$pct" "$reset" "$tail"
  else
    # normal mode: bar + always-on reset countdown at its single
    # highest-magnitude unit (dim, since it's ancillary here)
    local tail=""
    (( resets_at > 0 )) && tail=" ${dim}$(fmt_top $(( resets_at - now )))${reset}"
    printf '%s%s%s %s%s' "$dim" "$label" "$reset" "$(bar "$pct")" "$tail"
  fi
}

sep="  "

# --- assemble -----------------------------------------------------------------
OUT="${bold}${mag}🤖 ${MODEL}${reset}${sep}${cyan}📁 ${FOLDER}${reset}${sep}"

# context: bar normally, but only the % if any limit is near
if (( F_INT >= 90 || S_INT >= 90 )); then
  OUT+="🧠 $(color_for "$CTX_INT")${CTX_INT}%${reset}"
else
  OUT+="🧠 $(bar "$CTX_INT")"
fi

(( F_INT >= 0 )) && OUT+="${sep}$(metric "⏳" "$F_INT" "$FIVE_RESET")"
(( S_INT >= 0 )) && OUT+="${sep}$(metric "📅" "$S_INT" "$SEVEN_RESET")"

printf '%s' "$OUT"
