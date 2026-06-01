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
    /// (so the next frame stops drawing its row) and prints the
    /// final status line above the spinner — the same one the
    /// existing `progress` callback was printing, just routed
    /// through here so the spinner has a chance to clear its
    /// previous frame first.
    pub fn finished(&self, bookmark: &str, status: FinishedStatus) {
        let line = render_completion_line(bookmark, status);
        // Non-TTY: completion line stands alone, no clear-and-redraw
        // dance needed.
        if self.renderer.is_none() {
            eprintln!("{line}");
            return;
        }

        let mut state = self.state.lock().unwrap();
        if let Some(pos) = state.running.iter().position(|e| e.label == bookmark) {
            state.running.remove(pos);
        }
        // Print the completion line through the same erase-redraw
        // protocol the renderer uses, so it ends up *above* the
        // current spinner block instead of mixing into it.
        clear_block(&mut std::io::stderr(), state.lines_drawn);
        let _ = writeln!(std::io::stderr(), "{line}");
        // After the wipe the next render-tick will redraw the
        // running set from scratch — set lines_drawn to 0 so it
        // doesn't try to erase the bookmarks we just cleared.
        state.lines_drawn = 0;
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
            let mut state = self.state.lock().unwrap();
            clear_block(&mut std::io::stderr(), state.lines_drawn);
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
/// three states the existing `progress` callback in
/// `jj_gt::hooks::run_pre_push_stack` already handles — kept here
/// so the tracker owns the line formatting in one place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FinishedStatus {
    Passed,
    Failed,
    Cancelled,
}

fn render_completion_line(bookmark: &str, status: FinishedStatus) -> String {
    match status {
        FinishedStatus::Passed => format!(
            "  {}     {bookmark}",
            "passed".if_supports_color(Stream::Stderr, |s| s.green())
        ),
        FinishedStatus::Failed => format!(
            "  {}     {bookmark} (full output below)",
            "FAILED".if_supports_color(Stream::Stderr, |s| s.red())
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
        // Snapshot under the lock; format + write outside the lock
        // so worker threads don't block on terminal I/O.
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
        };

        let mut err = std::io::stderr();
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

        tick = tick.wrapping_add(1);
        std::thread::sleep(FRAME_DURATION);
    }

    // Final wipe on the way out so the loop exits with no leftover
    // spinner block on screen. [`Tracker::finish`] also wipes
    // before signalling stop, but a duplicate wipe is harmless
    // (writes 0 escapes when `lines_drawn` is 0).
    let mut err = std::io::stderr();
    let last = state.lock().unwrap().lines_drawn;
    clear_block(&mut err, last);
    state.lock().unwrap().lines_drawn = 0;
}

/// Erase `n` lines above the cursor by emitting `n` repeats of
/// "cursor-up + clear-line", then leave the cursor at the start of
/// the now-empty block. Bails on the first write failure (terminal
/// closed mid-run) — there's no recovery path so silently dropping
/// matches what `eprintln!` does on a write error.
fn clear_block(err: &mut std::io::Stderr, n: usize) {
    for _ in 0..n {
        if write!(err, "\x1b[1A\r\x1b[2K").is_err() {
            return;
        }
    }
    let _ = err.flush();
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
        assert!(
            render_completion_line("foo", FinishedStatus::Failed).contains("full output below")
        );
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
        t.finished("alpha", FinishedStatus::Passed);
        t.finished("beta", FinishedStatus::Failed);
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
        t.finished("x", FinishedStatus::Passed);
        drop(t);
    }
}
