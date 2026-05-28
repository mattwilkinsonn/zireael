#!/bin/bash

# Claude Code Status Line Script - Gruvbox Dark Theme
# Format: [Model Name] | [Progress Bar] | [tokens_used/total_tokens] | [directory] | [git branch]

# Read JSON input
input=$(cat)

# Extract values. total_input/output_tokens were used by an earlier
# version that summed them directly; we now compute tokens_used from
# used_percentage * context_window_size below for accuracy when the
# cumulative totals drift from the current context.
model_name=$(echo "$input" | jq -r '.model.display_name')
context_window_size=$(echo "$input" | jq -r '.context_window.context_window_size // 200000')
used_percentage=$(echo "$input" | jq -r '.context_window.used_percentage // 0')
cwd=$(echo "$input" | jq -r '.workspace.current_dir')

# Gruvbox Dark color palette
MODEL_COLOR="\033[38;2;86;182;194m"       # Model name - #56B6C2 (bright teal)
BRACKET_COLOR="\033[38;2;102;92;84m"      # Progress bar brackets/separators - Gruvbox gray #665C54
EMPTY_BAR_COLOR="\033[38;2;50;48;47m"     # Empty bar segments - Near-background #32302F
FILLED_BAR_COLOR="\033[38;2;142;192;124m" # Filled bar segments - Gruvbox Aqua #8EC07C
PERCENTAGE_COLOR="\033[38;2;251;241;199m" # Percentage number - Bright foreground #FBF1C7 (emphasized)
TOKEN_COLOR="\033[38;2;224;175;104m"      # Token budget - #E0AF68
GIT_COLOR="\033[38;2;143;175;209m"        # Git branch - #8FAFD1
DIR_COLOR="\033[38;2;152;195;121m"        # Directory - #98C379
RESET="\033[0m"

# Calculate tokens used from percentage and context window size
# This ensures accuracy when the cumulative totals don't match the current context
if [ "$used_percentage" != "null" ] && [ "$used_percentage" != "0" ]; then
	tokens_used=$(echo "scale=0; $used_percentage * $context_window_size / 100" | bc)
else
	tokens_used=0
fi

# Build status line
status_line=""

# 1. Model name
if [ -n "$model_name" ]; then
	status_line+=$(printf "${MODEL_COLOR}✦ %s${RESET}" "$model_name")
fi

# 2. Progress bar for token usage (20-segment lightweight bar)
if [ -n "$used_percentage" ] && [ "$used_percentage" != "null" ]; then
	# DESIGN OPTIONS (glyph choices):
	# Option 1: ▰▱ (horizontal rectangles) - clean, proportional, good contrast
	# Option 2: ━─ (box drawing lines) - minimal, very lightweight
	# Option 3: ■□ (large squares) - bigger, more visible
	#
	# IMPLEMENTED: ■□ (large squares, more visible)
	# 20 segments, each representing 5% of context window

	# Build 12-segment progress bar
	bar="${BRACKET_COLOR}[${RESET}"
	segment_size=5 # 100% / 20 segments

	for segment in {1..20}; do
		segment_threshold=$(echo "scale=2; $segment * $segment_size" | bc)

		if (($(echo "$used_percentage >= $segment_threshold" | bc -l))); then
			# Fully filled segment
			bar+="${FILLED_BAR_COLOR}■${RESET}"
		else
			# Empty segment
			bar+="${EMPTY_BAR_COLOR}□${RESET}"
		fi
	done

	bar+="${BRACKET_COLOR}]${RESET}"

	# Format percentage as integer with emphasized styling
	percentage_int=$(printf "%.0f" "$used_percentage")

	status_line+=$(printf " | %s ${PERCENTAGE_COLOR}%s%%${RESET}" "$bar" "$percentage_int")
fi

# 3. Token count (tokens_used/total_tokens)
if [ "$tokens_used" -gt 0 ]; then
	# Format tokens in thousands with 'k' suffix
	tokens_used_k=$(echo "scale=0; $tokens_used / 1000" | bc)
	context_window_k=$(echo "scale=0; $context_window_size / 1000" | bc)
	status_line+=$(printf " | ${TOKEN_COLOR}↯ %sk/%sk${RESET}" "$tokens_used_k" "$context_window_k")
fi

# 4. Directory (formatted as ./directory_name)
if [ -n "$cwd" ]; then
	dir_name=$(basename "$cwd")
	status_line+=$(printf " | ${DIR_COLOR}◆ %s${RESET}" "$dir_name")
fi

# 5. Git branch (with branch icon)
if [ -n "$cwd" ] && [ -d "$cwd" ]; then
	# `cd` failure here is a no-op for branch detection — the
	# `git rev-parse` below stays in the current dir and returns
	# non-zero, the surrounding `if` skips the segment, and the
	# rest of the status line still renders.
	if cd "$cwd" 2>/dev/null && git rev-parse --git-dir >/dev/null 2>&1; then
		branch=$(git --no-optional-locks branch --show-current 2>/dev/null)
		if [ -n "$branch" ]; then
			status_line+=$(printf " | ${GIT_COLOR}⎇ %s${RESET}" "$branch")
		fi
	fi
fi

# Output the status line
echo -e "$status_line"
