#!/usr/bin/env bash
# Per-app display/space rules. Display and space indexes shift around when
# monitors connect/disconnect or macOS reorders things, so we resolve them
# dynamically at runtime (by display resolution) and re-add every rule on
# every invocation. Run at yabai startup and on display_added/removed/moved.
#
# All rule labels are prefixed `auto:` so we can identify and clean up the
# rules this script manages without touching anything added by hand.

set -u
export PATH="/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin:$PATH"

# 1) Remove any rules we previously added (label starts with "auto:").
yabai -m rule --list 2>/dev/null |
	jq -r '.[] | select((.label // "") | startswith("auto:")) | .label' |
	while read -r lbl; do
		[[ -n $lbl ]] && yabai -m rule --remove "$lbl" 2>/dev/null || true
	done

displays=$(yabai -m query --displays 2>/dev/null) || exit 0

# Resolve display index by resolution (stable per monitor).
idx_for_width() {
	printf '%s' "$displays" | jq -r --argjson w "$1" \
		'[.[] | select(.frame.w == $w)] | .[0].index // empty'
}

g9=$(idx_for_width 5120)
aw=$(idx_for_width 3440)

# Laptop's first two space indexes (first is stack layout, second is BSP).
laptop_stack_space=$(printf '%s' "$displays" |
	jq -r '[.[] | select(.frame.w==1728)] | .[0].spaces[0] // empty')
laptop_bsp_space=$(printf '%s' "$displays" |
	jq -r '[.[] | select(.frame.w==1728)] | .[0].spaces[1] // empty')

# add_rule <label-suffix> <yabai rule args...>
add_rule() {
	local suffix=$1
	shift
	yabai -m rule --add label="auto:${suffix}" "$@" 2>/dev/null || true
}

# --- Default: laptop stack space ----------------------------------------
# Any window without a more specific rule below lands on the laptop stack
# space. Yabai applies rules in insertion order and later rules override
# earlier ones for shared properties (display/space), so the catch-all
# goes FIRST and the specific rules below override it.
if [[ -n $laptop_stack_space ]]; then
	add_rule default-laptop-stack app="^.*$" space="$laptop_stack_space"
fi

# --- G9 -----------------------------------------------------------------
if [[ -n $g9 ]]; then
	add_rule code app="^Code$" display="$g9"
	add_rule zed app="^Zed$" display="$g9"
	add_rule cmux app="^cmux$" display="$g9"
fi

# --- Alienware ----------------------------------------------------------
if [[ -n $aw ]]; then
	add_rule akiflow app="^Akiflow$" display="$aw"
	add_rule linear app="^Linear$" display="$aw"
	add_rule arc app="^Arc$" display="$aw"
fi

# --- Laptop BSP space (Messages, Qalculate, System Settings) -----------
# These are the only laptop-screen apps that should NOT land on the
# stack space — they go on the secondary BSP space instead. The default
# catch-all above already routes everything else to the stack space, so
# Discord, Obsidian, Claude, Activity Monitor, Spotify, 1Password,
# Circleback, Slack, etc. don't need explicit entries.
if [[ -n $laptop_bsp_space ]]; then
	add_rule messages app="^Messages$" space="$laptop_bsp_space"
	add_rule qalculate app="^Qalculate$" space="$laptop_bsp_space"
	add_rule systemsettings app="^System Settings$" space="$laptop_bsp_space"
fi

# --- Always float -------------------------------------------------------
# Finder is landed on the laptop stack by the catch-all above, then
# unmanaged here. Floating windows still appear on whatever space the
# catch-all assigned, but yabai doesn't tile them.
add_rule finder app="^Finder$" manage=off

# Re-apply all rules to currently-open windows.
yabai -m rule --apply 2>/dev/null || true
