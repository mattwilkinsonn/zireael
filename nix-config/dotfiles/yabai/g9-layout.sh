#!/usr/bin/env bash
# G9 protect-primary: BSP gives a depth-1 window at ~50% screen width on a
# wide monitor. We track that as "primary". If a new window opens with primary
# focused, primary gets split — we detect that and warp the intruder into the
# non-primary subtree so primary's 50% slot is restored.

set -u
export PATH="/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin:$PATH"

G9_UUID="566C01EF-4847-4B1B-8169-CE2612386D4D"
STATE_DIR="$HOME/.cache/yabai-g9"
LOCK="/tmp/yabai-g9-layout.lock"

mkdir -p "$STATE_DIR"
if ! mkdir "$LOCK" 2>/dev/null; then
	exit 0
fi
trap 'rmdir "$LOCK" 2>/dev/null' EXIT

space=$(yabai -m query --spaces --space 2>/dev/null) || exit 0
display_idx=$(printf '%s' "$space" | jq -r '.display')
display_uuid=$(yabai -m query --displays --display "$display_idx" 2>/dev/null | jq -r '.uuid')
[[ $display_uuid != "$G9_UUID" ]] && exit 0

space_idx=$(printf '%s' "$space" | jq -r '.index')
STATE_FILE="$STATE_DIR/space-${space_idx}-primary"

windows=$(yabai -m query --windows --space | jq '[.[] | select(
  ."is-minimized"==false
  and ."is-visible"==true
  and ."is-floating"==false
  and (.subrole=="AXStandardWindow" or .subrole=="AXDialog")
)]')
count=$(printf '%s' "$windows" | jq 'length')

case "$count" in
0)
	rm -f "$STATE_FILE"
	exit 0
	;;
1)
	printf '%s' "$windows" | jq -r '.[0].id' >"$STATE_FILE"
	exit 0
	;;
esac

screen_w=$(yabai -m query --displays --display "$display_idx" | jq '.frame.w')

# Is there currently a window at ~50% screen width? That's the de-facto primary.
half_id=$(printf '%s' "$windows" | jq -r --argjson sw "$screen_w" '
  [.[] | select((.frame.w / $sw) >= 0.45 and (.frame.w / $sw) <= 0.55)]
  | .[0].id // empty')

if [[ -n $half_id && $half_id != "null" ]]; then
	# Layout has a clear primary. Tie-break to the saved id if it qualifies, to
	# avoid flipping primary between sibling 50% windows on every event.
	saved=$(cat "$STATE_FILE" 2>/dev/null || echo "")
	saved_qualifies=$(printf '%s' "$windows" | jq --argjson s "${saved:-0}" --argjson sw "$screen_w" '
    [.[] | select(.id == $s and (.frame.w / $sw) >= 0.45 and (.frame.w / $sw) <= 0.55)] | length')
	if [[ ${saved_qualifies:-0} -ge 1 ]]; then
		: # keep saved
	else
		echo "$half_id" >"$STATE_FILE"
	fi
	exit 0
fi

# No ~50% window. With count=2 this means a manual resize — leave it alone.
# With count>=3 it means primary just got split.
[[ $count -le 2 ]] && exit 0

saved=$(cat "$STATE_FILE" 2>/dev/null || echo "")
[[ -z $saved ]] && exit 0

saved_exists=$(printf '%s' "$windows" | jq --argjson s "$saved" '[.[] | select(.id == $s)] | length')
if [[ ${saved_exists:-0} -eq 0 ]]; then
	rm -f "$STATE_FILE"
	exit 0
fi

# Intruder = focused window (BSP focuses newly-created windows by default).
focused=$(yabai -m query --windows --window 2>/dev/null | jq '.id // empty')
[[ -z $focused || $focused == "$saved" ]] && exit 0

# Warp target: widest non-saved, non-focused window — that's the one in the
# "other" subtree we want the intruder to join.
target=$(printf '%s' "$windows" | jq -r --argjson s "$saved" --argjson f "$focused" '
  [.[] | select(.id != $s and .id != $f)]
  | sort_by(-.frame.w) | .[0].id // empty')

if [[ -n $target && $target != "null" ]]; then
	yabai -m window "$focused" --warp "$target" 2>/dev/null || true
fi
