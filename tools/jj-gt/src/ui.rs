//! User-facing progress + result rendering for jj-gt.
//!
//! Every CLI command (`submit`, `fetch`, `status`) runs through a
//! sequence of subprocess calls — `jj git fetch`, `gt track`,
//! `gt submit`, `jj rebase`, etc. — each of which is chatty in its
//! own way. Letting all of that stderr stream straight through to
//! the user is what `jj-gt` did at first and the result was an
//! unreadable wall of mixed-tool output.
//!
//! This module wraps each subprocess invocation in a [`Step`]:
//!
//! - On entry: print a one-line header with a leading glyph
//!   (`⠋`-like marker before completion, replaced on completion)
//!   and the human name of the phase.
//! - During: subprocess stderr/stdout get captured into a buffer
//!   instead of streaming to the user's terminal.
//! - On exit:
//!   - `Step::success("3 new commits")` → replace marker with `✓`,
//!     append summary in dim text, drop the captured buffer (unless
//!     verbose).
//!   - `Step::skip("nothing to do")` → `◦` marker + dim text.
//!   - `Step::warn("drift detected")` → `⚠` marker + yellow text.
//!   - `Step::fail("hook rejected")` → `✗` marker + red text,
//!     ALWAYS dump the captured buffer so the user can debug.
//!
//! Verbose mode (`-v` / `--verbose`) makes every step dump its
//! captured buffer regardless of outcome — useful when something
//! is going subtly wrong inside one of the subprocess invocations
//! and the structured summary is hiding the real signal.
//!
//! Color/glyph rendering uses [`owo-colors`] with its
//! `supports-colors` feature, which auto-detects TTY + respects
//! `NO_COLOR`. Glyphs are unicode unconditionally — every modern
//! terminal handles them.

use std::io::Write;

use owo_colors::OwoColorize;
use owo_colors::Stream;

/// Verbosity level for [`Step`] output. The global CLI flag flips
/// `Verbose` on; otherwise `Quiet` is the default.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verbosity {
    /// Show only the structured step headers + summaries. Captured
    /// subprocess output is suppressed on success and only dumped
    /// on failure.
    Quiet,
    /// Show structured step headers + the full captured subprocess
    /// output on every step, regardless of outcome.
    Verbose,
}

impl Verbosity {
    pub fn from_flag(verbose: bool) -> Self {
        if verbose { Self::Verbose } else { Self::Quiet }
    }
}

/// A single step in a multi-phase CLI command. Construct via
/// [`Step::start`], drop or call one of the terminal methods
/// (`success` / `skip` / `warn` / `fail`) when the phase ends.
///
/// Each Step is responsible for rendering exactly one line of
/// output. Captured subprocess output is owned by the caller and
/// passed to the terminal method — keeps the Step itself
/// allocation-free for the no-output case.
pub struct Step {
    name: String,
    verbosity: Verbosity,
}

impl Step {
    /// Begin a step. Prints the in-progress marker + step name to
    /// stderr immediately so the user sees forward motion before
    /// the subprocess returns.
    pub fn start(name: &str, verbosity: Verbosity) -> Self {
        let name = name.to_owned();
        let glyph = "⠋".if_supports_color(Stream::Stderr, |s| s.dimmed());
        eprintln!("{glyph}  {name}");
        Self { name, verbosity }
    }

    /// Terminal: phase completed successfully. `summary` is a
    /// short note that ends up in dim text after the step name
    /// (e.g. `"3 new commits"`, `"PR #340 already tracked"`).
    /// Pass an empty string for no summary.
    pub fn success(self, summary: &str, captured: Option<&str>) {
        let glyph = "✓".if_supports_color(Stream::Stderr, |s| s.green());
        let summary_part = if summary.is_empty() {
            String::new()
        } else {
            let dimmed = summary.if_supports_color(Stream::Stderr, |s| s.dimmed());
            format!("    {dimmed}")
        };
        // Move the cursor up one line to overwrite the in-progress
        // marker we printed on `start`. `\x1b[1A\r\x1b[K` is
        // up-one + carriage-return + erase-to-end-of-line.
        rewrite_line(&format!("{glyph}  {name}{summary_part}", name = self.name));
        self.maybe_dump_captured(captured);
    }

    /// Terminal: phase was a no-op (nothing to do, already in the
    /// desired state). Marker is `◦` to differentiate from `✓` —
    /// makes it easy to skim a long output for "did anything
    /// happen?" at a glance.
    pub fn skip(self, reason: &str, captured: Option<&str>) {
        let glyph = "◦".if_supports_color(Stream::Stderr, |s| s.dimmed());
        let reason_part = if reason.is_empty() {
            String::new()
        } else {
            let dimmed = reason.if_supports_color(Stream::Stderr, |s| s.dimmed());
            format!("    {dimmed}")
        };
        rewrite_line(&format!("{glyph}  {name}{reason_part}", name = self.name));
        self.maybe_dump_captured(captured);
    }

    /// Terminal: phase completed but with a warning the user
    /// should know about (e.g. SHA drift detected, fixup commit
    /// produced). Marker is `⚠` in yellow.
    pub fn warn(self, message: &str, captured: Option<&str>) {
        let glyph = "⚠".if_supports_color(Stream::Stderr, |s| s.yellow());
        let yellow_message = message.if_supports_color(Stream::Stderr, |s| s.yellow());
        rewrite_line(&format!(
            "{glyph}  {name}    {yellow_message}",
            name = self.name
        ));
        // Warnings dump captured output even in quiet mode if the
        // caller supplied any — usually relevant context.
        if let Some(captured) = captured {
            dump_captured(captured, /*always=*/ true);
        }
    }

    /// Terminal: phase failed. Marker is `✗` in red, message in
    /// red, and the captured buffer is ALWAYS dumped (regardless
    /// of quiet/verbose) so the user has the full subprocess
    /// output to debug from.
    pub fn fail(self, error: &str, captured: Option<&str>) {
        let glyph = "✗".if_supports_color(Stream::Stderr, |s| s.red());
        let red_error = error.if_supports_color(Stream::Stderr, |s| s.red());
        rewrite_line(&format!("{glyph}  {name}    {red_error}", name = self.name));
        if let Some(captured) = captured {
            dump_captured(captured, /*always=*/ true);
        }
    }

    fn maybe_dump_captured(&self, captured: Option<&str>) {
        if matches!(self.verbosity, Verbosity::Verbose)
            && let Some(captured) = captured
        {
            dump_captured(captured, /*always=*/ true);
        }
    }
}

/// Overwrite the line we wrote on `Step::start` with the final
/// rendering. Uses ANSI cursor-up + clear-line escape sequences.
/// When stderr isn't a TTY (piped to a file, captured by a test)
/// the escape sequences would garble the output, so we fall back
/// to a plain print + newline.
fn rewrite_line(line: &str) {
    use std::io::IsTerminal;
    let mut err = std::io::stderr();
    if err.is_terminal() {
        // \x1b[1A = cursor up 1, \r = column 0, \x1b[2K = clear
        // line. Then write the new content + newline.
        let _ = write!(err, "\x1b[1A\r\x1b[2K{line}\n");
        let _ = err.flush();
    } else {
        // No TTY → don't even try to overwrite. Just print the
        // final line. The in-progress marker will already have
        // been printed; the result is one extra line per step,
        // but readable in pipe/log capture contexts.
        let _ = writeln!(err, "{line}");
    }
}

/// Dump a captured subprocess output buffer to stderr with each
/// line prefixed by `│ ` so it's visually distinguishable from
/// the step headers. Suppressed when `captured` is empty.
fn dump_captured(captured: &str, _always: bool) {
    if captured.trim().is_empty() {
        return;
    }
    for line in captured.lines() {
        let prefixed = format!("│ {line}");
        eprintln!(
            "{}",
            prefixed.if_supports_color(Stream::Stderr, |s| s.dimmed())
        );
    }
}

/// Render a section header — used between top-level phases when
/// rendering long pipelines (e.g. `jj-gt fetch` has Fetch → Track
/// → Sync → Cleanup sections). Prints a blank line above and a
/// bolded heading.
pub fn section(title: &str) {
    eprintln!();
    let styled = title.if_supports_color(Stream::Stderr, |s| s.bold());
    eprintln!("{styled}");
}

/// Print a single action-row in the final result table that both
/// `submit` and `fetch` emit. The leading glyph (`●` by default) +
/// bookmark name + dim summary keeps everything visually aligned
/// with the step rows above.
pub fn action_row(bookmark: &str, status: ActionStatus, message: &str) {
    let (glyph, painted_msg) = match status {
        ActionStatus::Ok => (
            "●"
                .if_supports_color(Stream::Stderr, |s| s.green())
                .to_string(),
            message
                .if_supports_color(Stream::Stderr, |s| s.dimmed())
                .to_string(),
        ),
        ActionStatus::Warn => (
            "●"
                .if_supports_color(Stream::Stderr, |s| s.yellow())
                .to_string(),
            message
                .if_supports_color(Stream::Stderr, |s| s.yellow())
                .to_string(),
        ),
        ActionStatus::Skipped => (
            "◦"
                .if_supports_color(Stream::Stderr, |s| s.dimmed())
                .to_string(),
            message
                .if_supports_color(Stream::Stderr, |s| s.dimmed())
                .to_string(),
        ),
        ActionStatus::Error => (
            "●"
                .if_supports_color(Stream::Stderr, |s| s.red())
                .to_string(),
            message
                .if_supports_color(Stream::Stderr, |s| s.red())
                .to_string(),
        ),
    };
    eprintln!("  {glyph}  {bookmark:<48}{painted_msg}");
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionStatus {
    Ok,
    Warn,
    Skipped,
    Error,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verbosity_from_flag_round_trips() {
        assert_eq!(Verbosity::from_flag(false), Verbosity::Quiet);
        assert_eq!(Verbosity::from_flag(true), Verbosity::Verbose);
    }

    #[test]
    fn action_status_variants_distinct() {
        // Sanity check that the enum has the expected variants and
        // they're not accidentally aliased — guards against a
        // future refactor where someone collapses them.
        assert_ne!(ActionStatus::Ok, ActionStatus::Warn);
        assert_ne!(ActionStatus::Skipped, ActionStatus::Error);
    }
}
