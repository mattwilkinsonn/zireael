//! Live per-bookmark progress tracker for parallel hook runs.
//!
//! `jj-gt submit` and `jj-gt reconcile` fan a `jj-hooks` invocation
//! out across N bookmarks in parallel. Hooks for a single bookmark
//! can take 30+ seconds (cold cargo cache, slow JS toolchain, etc.),
//! and the runner's output is captured per-bookmark — so without
//! something on the screen telling the user which bookmarks are
//! currently in flight, the terminal sits idle and the run looks
//! frozen. Existing per-completion `passed`/`FAILED` lines are still
//! printed (see [`Tracker::finished`]) but they only fire after work
//! is done; this module adds the missing "in progress" signal.
//!
//! # Behaviour
//!
//! - TTY: animated braille spinner (`⠋ ⠙ ⠹ ⠸ ⠼ ⠴ ⠦ ⠧ ⠇ ⠏`) at
//!   ~10 fps, one row per running bookmark, with elapsed time so
//!   the user can tell when a particular bookmark is the long pole.
//! - Non-TTY (CI logs, piped output): degrades to one `START` line
//!   per bookmark when it starts running. The completion lines that
//!   were already there give the rest of the picture.
//!
//! The active running set is held in a `Mutex<BTreeMap>`, sorted by
//! insertion order so the rendered list is stable. A background
//! thread re-renders every ~100ms, clearing the previous block of
//! lines with cursor-up + erase-line escapes before drawing the
//! current state. The thread is joined on `Tracker::finish` — owning
//! it as a `JoinHandle` ensures we always cancel cleanly even on a
//! panic / early return.

use std::io::{IsTerminal, Write};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use owo_colors::OwoColorize;
use owo_colors::Stream;

/// Frame rate for the spinner animation. 100ms is the sweet spot:
/// fast enough to read as motion, slow enough that the terminal
/// scroll buffer isn't being thrashed on each render.
const FRAME_DURATION: Duration = Duration::from_millis(100);

/// Braille spinner glyph sequence. Same set most CLI spinners use
/// (notably indicatif's default) — visually balanced and works in
/// every modern terminal font.
const SPINNER_GLYPHS: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// Shared mutable state for the renderer thread + the start /
/// finish callbacks. The mutex is held only while inserting,
/// removing, or snapshotting; the actual write to stderr happens
/// outside the lock so the renderer doesn't serialise the workers.
struct State {
    running: Vec<RunningEntry>,
    /// Number of lines the renderer drew on its last frame, so the
    /// next frame knows how many lines to erase. Starts at zero;
    /// resets to zero when the running set empties.
    lines_drawn: usize,
}

struct RunningEntry {
    /// Bookmark name as it should appear in the rendered row.
    label: String,
    /// When the start callback fired. Used for the elapsed-time
    /// column in each row.
    started_at: Instant,
}

/// Live tracker for parallel hook runs. Constructed with
/// [`Tracker::new`]; drop the returned `JoinHandle`-owning value via
/// [`Tracker::finish`] when the parallel run completes so the
/// renderer thread shuts down cleanly.
pub struct Tracker {
    state: Arc<Mutex<State>>,
    /// Sentinel the renderer thread polls to know when to exit.
    /// Set to `true` from [`Tracker::finish`] before joining.
    stop: Arc<AtomicBool>,
    /// `Some(_)` when the tracker is in TTY mode and a renderer
    /// thread is running. `None` when stderr isn't a TTY — start /
    /// finished callbacks then print plain lines synchronously and
    /// there's nothing to join.
    renderer: Option<JoinHandle<()>>,
}

impl Tracker {
    /// Spawn a tracker. The renderer thread starts immediately and
    /// will draw the (currently empty) running set every
    /// [`FRAME_DURATION`]. Workers call [`Tracker::started`] when a
    /// bookmark begins and [`Tracker::finished`] when it completes;
    /// the renderer reflects the snapshot on each frame.
    ///
    /// `total` and `partitions` are header context for the initial
    /// non-animated line — they don't affect the animation itself.
    pub fn new() -> Self {
        let state = Arc::new(Mutex::new(State {
            running: Vec::new(),
            lines_drawn: 0,
        }));
        let stop = Arc::new(AtomicBool::new(false));

        // TTY check is one-shot at construction. If stderr is
        // redirected mid-run we'd false-positive into spinner mode,
        // but that scenario doesn't happen in normal CLI use — the
        // user either has a terminal or they don't.
        if !std::io::stderr().is_terminal() {
            return Self {
                state,
                stop,
                renderer: None,
            };
        }

        let renderer_state = Arc::clone(&state);
        let renderer_stop = Arc::clone(&stop);
        let handle = std::thread::Builder::new()
            .name("jj-gt-hooks-progress".into())
            .spawn(move || render_loop(renderer_state, renderer_stop))
            .expect("spawn jj-gt-hooks-progress thread");

        Self {
            state,
            stop,
            renderer: Some(handle),
        }
    }

    /// Mark a bookmark as in-flight. Called from the worker thread
    /// the instant the hook scope is entered (worktree creation,
    /// setup steps, runner invocation are all "running" as far as
    /// the user is concerned). Falls back to a one-shot `START`
    /// line in non-TTY mode.
    pub fn started(&self, bookmark: &str) {
        // Non-TTY: print a stable "started" line synchronously so
        // CI logs still tell the story. The completion line
        // (printed by the hooks runner's progress callback) closes
        // the loop.
        if self.renderer.is_none() {
            eprintln!("  started    {bookmark}");
            return;
        }

        let mut state = self.state.lock().unwrap();
        state.running.push(RunningEntry {
            label: bookmark.to_owned(),
            started_at: Instant::now(),
        });
    }

    /// Mark a bookmark as finished. Removes it from the active set
    /// (so the next frame stops drawing its row), prints the final
    /// status line above the spinner, and — when `status` is
    /// `Failed` or `Fixup` and `captured` is non-empty — dumps the
    /// captured output between the status line and the still-
    /// running spinner block. The dump lands at exactly the moment
    /// of completion so the user gets actionable signal without
    /// waiting for sibling bookmarks to drain (which can be 30-60s
    /// on a cold cargo cache).
    ///
    /// Passing `None` for `captured` is the right call when:
    ///   - `status` is `Passed` / `Cancelled` (nothing to dump),
    ///   - status IS Failed / Fixup but the runner didn't produce
    ///     a captured buffer (e.g. a spawn error before the runner
    ///     started, or a sequential run that streamed live). The
    ///     completion line stands alone in that case.
    pub fn finished(&self, bookmark: &str, status: FinishedStatus, captured: Option<&str>) {
        let line = render_completion_line(bookmark, status);
        let kind = match status {
            FinishedStatus::Failed => Some("failure output"),
            FinishedStatus::Fixup => Some("autofix output"),
            FinishedStatus::Passed | FinishedStatus::Cancelled => None,
        };
        let dump_payload = kind.and_then(|k| {
            captured
                .filter(|c| !c.trim().is_empty())
                .map(|c| (k, c.to_owned()))
        });

        // Non-TTY: print line + dump synchronously, no clear-and-
        // redraw protocol needed (there's no spinner block to
        // erase).
        if self.renderer.is_none() {
            let mut err = std::io::stderr().lock();
            let _ = writeln!(err, "{line}");
            if let Some((k, c)) = dump_payload.as_ref() {
                write_failure_block(&mut err, bookmark, k, c);
            }
            return;
        }

        // TTY path. Two locks at play here:
        //   1. `stderr().lock()` — keeps the renderer thread (and
        //      any concurrent worker) from interleaving its own
        //      writes between this worker's clear_block, completion
        //      line, and (optionally) failure dump. Held for the
        //      whole atomic visual unit.
        //   2. `self.state` — guards `state.running` and
        //      `state.lines_drawn` against concurrent worker /
        //      renderer access. Held only long enough to remove
        //      this bookmark from the running set and reset
        //      `lines_drawn` to zero so the next render-tick
        //      doesn't try to erase the lines we just emitted.
        //
        // The state lock is released BEFORE the captured-output
        // dump even when one is present — captured buffers can be
        // tens of KB (compiler diagnostics, lint diffs) and we
        // don't want other workers' `finished` calls or the
        // renderer's state snapshot to wait behind that terminal
        // I/O. The stderr lock is held across the dump regardless,
        // because the visual contract is "completion line + dump
        // appear back-to-back, attached to each other"; a render-
        // tick or a sibling worker's completion line landing
        // between them would break that.
        let mut err = std::io::stderr().lock();
        {
            let mut state = self.state.lock().unwrap();
            if let Some(pos) = state.running.iter().position(|e| e.label == bookmark) {
                state.running.remove(pos);
            }
            clear_block(&mut err, state.lines_drawn);
            let _ = writeln!(err, "{line}");
            // Reset BEFORE releasing the state lock so any concurrent
            // render-tick that races in next sees an empty erase
            // budget rather than trying to erase the lines we just
            // emitted as the completion line + (about-to-be) dump.
            state.lines_drawn = 0;
        }
        if let Some((k, c)) = dump_payload.as_ref() {
            write_failure_block(&mut err, bookmark, k, c);
        } else {
            let _ = err.flush();
        }
    }

    /// Stop the renderer thread and wait for it to exit. Safe to
    /// call multiple times; subsequent calls are no-ops.
    pub fn finish(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        // Final wipe so the spinner block doesn't linger after the
        // run completes (the caller's next eprintln would land on
        // the spinner's last row otherwise).
        if self.renderer.is_some() {
            // stderr → state, matching `Tracker::finished` and the
            // render loop. Without this, shutdown() could deadlock
            // against a render-tick that holds stderr and is
            // waiting on state.
            let mut err = std::io::stderr().lock();
            let mut state = self.state.lock().unwrap();
            clear_block(&mut err, state.lines_drawn);
            state.lines_drawn = 0;
        }
        self.stop.store(true, Ordering::Release);
        if let Some(handle) = self.renderer.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for Tracker {
    fn drop(&mut self) {
        // Defensive: if the caller forgot to call `finish()` (panic
        // path, early return), shut the renderer thread down so it
        // doesn't outlive the tracker and keep painting onto the
        // user's terminal after the parent has moved on.
        self.shutdown();
    }
}

impl Default for Tracker {
    fn default() -> Self {
        Self::new()
    }
}

/// Outcome a worker reports when a bookmark finishes. Mirrors the
/// states `jj_gt::hooks::run_pre_push_stack` already classifies —
/// kept here so the tracker owns the line formatting in one place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FinishedStatus {
    Passed,
    Failed,
    /// Runner modified files (post-fix or autofix). Output is dumped
    /// in-band with an "autofix output" header so the user can see
    /// what was rewritten without waiting for the post-run summary.
    Fixup,
    Cancelled,
}

fn render_completion_line(bookmark: &str, status: FinishedStatus) -> String {
    match status {
        FinishedStatus::Passed => format!(
            "  {}     {bookmark}",
            "passed".if_supports_color(Stream::Stderr, |s| s.green())
        ),
        FinishedStatus::Failed => format!(
            "  {}     {bookmark}",
            "FAILED".if_supports_color(Stream::Stderr, |s| s.red())
        ),
        FinishedStatus::Fixup => format!(
            "  {}    {bookmark}",
            "AUTOFIX".if_supports_color(Stream::Stderr, |s| s.yellow())
        ),
        FinishedStatus::Cancelled => format!(
            "  {}  {bookmark} (sibling failed)",
            "cancelled".if_supports_color(Stream::Stderr, |s| s.dimmed())
        ),
    }
}

/// Drive the render loop until `stop` is set. Runs on the dedicated
/// renderer thread spawned in [`Tracker::new`].
fn render_loop(state: Arc<Mutex<State>>, stop: Arc<AtomicBool>) {
    let mut tick: usize = 0;
    while !stop.load(Ordering::Acquire) {
        // Acquire the stderr lock BEFORE the state snapshot so the
        // whole tick — snapshot, clear_block, spinner writes — is
        // serialized against `Tracker::finished`. Without this, a
        // worker's `finished()` could fire between our state-lock
        // release (line A below) and our subsequent `clear_block`,
        // leaving us with a stale `prev_lines` that doesn't match
        // what's actually on screen anymore — `clear_block` would
        // then erase rows that include the worker's freshly-written
        // completion line + failure dump.
        //
        // The lock ordering here (stderr → state → release state →
        // continue stderr) matches the order `Tracker::finished`
        // uses, so the two paths can't deadlock against each other.
        // Stderr is released before the inter-frame sleep so worker
        // threads' `started`/`finished` calls aren't blocked for
        // the full ~100ms frame.
        let mut err = std::io::stderr().lock();
        let (running_snapshot, prev_lines): (Vec<(String, Duration)>, usize) = {
            let mut s = state.lock().unwrap();
            let snap: Vec<(String, Duration)> = s
                .running
                .iter()
                .map(|e| (e.label.clone(), e.started_at.elapsed()))
                .collect();
            let prev = s.lines_drawn;
            // Stash the count we're about to draw so the next frame
            // (or `finished` interleave) erases the right number of
            // lines.
            s.lines_drawn = snap.len();
            (snap, prev)
            // (A) state lock released here.
        };

        clear_block(&mut err, prev_lines);

        let glyph = SPINNER_GLYPHS[tick % SPINNER_GLYPHS.len()];
        for (label, elapsed) in &running_snapshot {
            let secs = elapsed.as_secs();
            let _ = writeln!(
                err,
                "  {glyph}  {label}  {}",
                format_args!("({secs}s)")
                    .to_string()
                    .if_supports_color(Stream::Stderr, |s| s.dimmed())
            );
        }
        let _ = err.flush();
        // Drop stderr lock before the sleep so workers can land
        // their completion lines / failure dumps during the
        // inter-frame quiet window.
        drop(err);

        tick = tick.wrapping_add(1);
        std::thread::sleep(FRAME_DURATION);
    }

    // Final wipe on the way out so the loop exits with no leftover
    // spinner block on screen. [`Tracker::finish`] also wipes
    // before signalling stop, but a duplicate wipe is harmless
    // (writes 0 escapes when `lines_drawn` is 0).
    //
    // Same lock ordering as the main render-tick: stderr → state.
    let mut err = std::io::stderr().lock();
    let last = state.lock().unwrap().lines_drawn;
    clear_block(&mut err, last);
    state.lock().unwrap().lines_drawn = 0;
}

/// Erase `n` lines above the cursor by emitting `n` repeats of
/// "cursor-up + clear-line", then leave the cursor at the start of
/// the now-empty block. Bails on the first write failure (terminal
/// closed mid-run) — there's no recovery path so silently dropping
/// matches what `eprintln!` does on a write error.
fn clear_block(err: &mut dyn Write, n: usize) {
    for _ in 0..n {
        if write!(err, "\x1b[1A\r\x1b[2K").is_err() {
            return;
        }
    }
    let _ = err.flush();
}

/// Print a captured-output block for a failed or autofix bookmark
/// with a visible header/footer fence so the user can scan back
/// through the scrollback and find the start of a particular
/// bookmark's output quickly.
///
/// `kind` is rendered into the header fence and is used to tell
/// `failure output` from `autofix output` apart in the scrollback.
/// Writes to `out` (production passes a locked `stderr`); tests
/// route to a `Vec<u8>` so they can assert on the output without
/// spinning up a terminal.
///
/// The block format matches `render_failure_report`'s historical
/// shape so users who got used to the post-run replay see the
/// same fence in the mid-run dump. Once the mid-run dump is the
/// only place the captured output appears, the post-run report
/// is just a summary table (one line per bookmark).
fn write_failure_block(out: &mut dyn Write, bookmark: &str, kind: &str, captured: &str) {
    let _ = writeln!(out, "--- {bookmark} ({kind}) ---");
    let _ = out.write_all(captured.as_bytes());
    if !captured.ends_with('\n') {
        let _ = writeln!(out);
    }
    let _ = writeln!(out, "--- end {bookmark} ---");
    let _ = writeln!(out);
    let _ = out.flush();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finished_status_lines_format_predictably() {
        // Pin the format so a future tweak has to update this test.
        // Color codes are stripped because `if_supports_color`
        // returns the bare string when stderr isn't a TTY (test
        // harness redirects).
        assert!(render_completion_line("foo", FinishedStatus::Passed).contains("passed"));
        assert!(render_completion_line("foo", FinishedStatus::Passed).contains("foo"));
        assert!(render_completion_line("foo", FinishedStatus::Failed).contains("FAILED"));
        // The completion line for a failure no longer mentions "full
        // output below" — the captured output is dumped immediately
        // after this line by `dump_failure_block`, not deferred to a
        // post-run replay. Pin the absence so a future revert is
        // caught.
        assert!(
            !render_completion_line("foo", FinishedStatus::Failed).contains("full output below"),
            "completion line should not promise a deferred replay anymore",
        );
        // Fixup is the new "success but file rewrite" status —
        // distinct word + color from Failed so the user can tell
        // at a glance which bookmarks need a squash vs which need
        // a full re-run.
        assert!(render_completion_line("foo", FinishedStatus::Fixup).contains("AUTOFIX"));
        assert!(render_completion_line("foo", FinishedStatus::Fixup).contains("foo"));
        assert!(render_completion_line("foo", FinishedStatus::Cancelled).contains("cancelled"));
        assert!(
            render_completion_line("foo", FinishedStatus::Cancelled).contains("sibling failed")
        );
    }

    #[test]
    fn tracker_started_and_finished_complete_without_panic() {
        // Smoke test: the tracker can be driven end-to-end without
        // an actual terminal (the test harness redirects stderr, so
        // we take the non-TTY path). Mainly here to catch deadlocks
        // / panics in the lock plumbing.
        let t = Tracker::new();
        t.started("alpha");
        t.started("beta");
        t.started("gamma");
        t.finished("alpha", FinishedStatus::Passed, None);
        t.finished("beta", FinishedStatus::Failed, Some("cargo error E0308\n"));
        t.finished(
            "gamma",
            FinishedStatus::Fixup,
            Some("fmt rewrote 2 files\n"),
        );
        t.finish();
    }

    #[test]
    fn tracker_finish_is_idempotent_via_drop_if_caller_forgets() {
        // If the caller forgets to call `finish()`, dropping the
        // tracker should still let the renderer thread exit
        // (because the JoinHandle is dropped and the Drop ordering
        // ensures the Arc<AtomicBool> is the last live reference).
        // No assertion needed — if this hangs in CI we know the
        // teardown leaked.
        let t = Tracker::new();
        t.started("x");
        t.finished("x", FinishedStatus::Passed, None);
        drop(t);
    }

    #[test]
    fn dump_failure_block_renders_fenced_output() {
        // The block format is the same fence the post-run replay
        // used to print, so users who know to look for "--- <name>"
        // in their scrollback still find it.
        let mut buf: Vec<u8> = Vec::new();
        write_failure_block(
            &mut buf,
            "sea-559",
            "failure output",
            "Diff in foo.rs:\n-extra\n",
        );
        let out = String::from_utf8(buf).unwrap();
        assert!(
            out.contains("--- sea-559 (failure output) ---"),
            "missing header fence: {out:?}",
        );
        assert!(
            out.contains("Diff in foo.rs:"),
            "missing captured content: {out:?}",
        );
        assert!(
            out.contains("--- end sea-559 ---"),
            "missing footer fence: {out:?}",
        );
    }

    #[test]
    fn dump_failure_block_renders_autofix_kind_in_header() {
        // The same helper serves Failed (failure output) and Fixup
        // (autofix output). Pin the kind label so a future tweak
        // to the enum doesn't silently lose the distinction.
        let mut buf: Vec<u8> = Vec::new();
        write_failure_block(&mut buf, "sea-559", "autofix output", "fmt wrote 2 files\n");
        let out = String::from_utf8(buf).unwrap();
        assert!(
            out.contains("--- sea-559 (autofix output) ---"),
            "header should carry the autofix kind: {out:?}",
        );
    }

    #[test]
    fn dump_failure_block_appends_newline_when_capture_lacks_one() {
        // Same trailing-newline contract the old report had: even
        // if the captured buffer didn't end with '\n', the footer
        // fence must land on its own line.
        let mut buf: Vec<u8> = Vec::new();
        write_failure_block(&mut buf, "sea-559", "failure output", "no trailing newline");
        let out = String::from_utf8(buf).unwrap();
        let end_pos = out.find("--- end sea-559 ---").unwrap();
        assert!(
            out[..end_pos].ends_with('\n'),
            "fence ran on the same line as the captured content: {out:?}",
        );
    }
}
