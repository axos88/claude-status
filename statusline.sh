#!/usr/bin/env bash
# Claude Code status line — reference implementation, kept in lockstep with the
# Rust port (src/main.rs) as the differential-test baseline (tests/).
# Layout:  🤖 <family>  📁 <folder>  🧠 <ctx>  ⏳ <5h>  📅 <7d>
# Each usage metric: icon  reset-countdown  progress-bar  pct%, all one color
# (green <=75, yellow 76-90, red >90). The countdown shows two units in the red
# zone (one below), pulses red<->dark-red at >=95%, and at >=100% the bar is
# dropped, leaving "⛔ <pct>% <countdown>". When ANY limit is red (>90), the
# context bar collapses to just its %.

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
# per-second countdown pulse: red <-> dark red, when >= 95%
dred=$'\033[38;5;88m'

# --- helpers ------------------------------------------------------------------
# threshold color for a percentage: green <= 75, yellow 76-90, red > 90
color_for() {
  local pct=$1
  if   (( pct > 90 )); then printf '%s' "$red"
  elif (( pct > 75 )); then printf '%s' "$yel"
  else                      printf '%s' "$grn"
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

# usage metric:  icon  countdown  bar  pct%
#   color: green <=75, yellow 76-90, red >90 (bar, %, countdown all share it)
#   countdown: two units in the red zone (>90), one unit below it
#   flashing:  countdown pulses red<->dark-red at >=95%
#   at >=100%: the bar and % are dropped — countdown + ⛔ only
metric() {
  local label=$1 pct=$2 resets_at=$3
  local color cd_color cd="" txt
  color=$(color_for "$pct")
  # countdown color: pulse each second once near the ceiling
  cd_color=$color
  if (( pct >= 95 )); then
    (( now % 2 == 0 )) && cd_color=$red || cd_color=$dred
  fi
  # countdown text: two units in the red zone, one unit below it
  if (( resets_at > 0 )); then
    if (( pct > 90 )); then txt=$(fmt_left $(( resets_at - now )))
    else                    txt=$(fmt_top  $(( resets_at - now )))
    fi
    cd="${cd_color}${txt}${reset}"
  fi
  if (( pct >= 100 )); then
    # over the limit: no bar — ⛔ flag, percentage, then the countdown
    local tail=""
    [ -n "$cd" ] && tail=" ${cd}"
    printf '%s%s%s ⛔ %s%d%%%s%s' "$dim" "$label" "$reset" "$color" "$pct" "$reset" "$tail"
  else
    # countdown sits between the icon and the bar, sharing its color
    local lead=""
    [ -n "$cd" ] && lead="${cd} "
    printf '%s%s%s %s%s' "$dim" "$label" "$reset" "$lead" "$(bar "$pct")"
  fi
}

sep="  "

# --- assemble -----------------------------------------------------------------
OUT="${bold}${mag}🤖 ${MODEL}${reset}${sep}${cyan}📁 ${FOLDER}${reset}${sep}"

# context: bar normally, but only the % if any limit is in the red zone
if (( F_INT > 90 || S_INT > 90 )); then
  OUT+="🧠 $(color_for "$CTX_INT")${CTX_INT}%${reset}"
else
  OUT+="🧠 $(bar "$CTX_INT")"
fi

(( F_INT >= 0 )) && OUT+="${sep}$(metric "⏳" "$F_INT" "$FIVE_RESET")"
(( S_INT >= 0 )) && OUT+="${sep}$(metric "📅" "$S_INT" "$SEVEN_RESET")"

printf '%s' "$OUT"
