//! Hoist issue-linking references from a bookmark's commit messages
//! into its PR description.
//!
//! Why this exists: a Graphite-queue PR is squash-merged with the
//! merge commit body set to the PR title + description. GitHub and
//! Linear scan that text for `<magic-word> <issue>` references to
//! close / link issues on merge. `gt submit --ai` regenerates the
//! description from the diff and is non-deterministic about preserving
//! those references, so they get dropped and issues strand in their
//! pre-merge state.
//!
//! The reliable source of truth is the commit messages — the author
//! already wrote `Closes SEA-1` / `Refs #42` there. This module
//! extracts every reference across a bookmark's commit range and
//! reconciles them into a machine-managed, HTML-comment-fenced block
//! at the end of the PR description. The block is regenerated from the
//! current commit set on every submit, so it's idempotent and a
//! review-cycle commit that adds a new reference gets picked up
//! automatically.
//!
//! Normalization: rather than hoist phrases verbatim and trust the
//! tracker to fan out a `Fixes A, B and C` list (GitHub in particular
//! does not reliably close every issue in a comma list under one
//! keyword), we parse each `(magic-word, issue)` pair, group by issue,
//! and emit one line per issue with a single canonical keyword —
//! `Closes <id>` if ANY commit closed it, else `Refs <id>`. That's the
//! form both trackers recognize unambiguously. A reference we can't
//! parse into an ID (a bare tracker URL) is hoisted verbatim on its
//! own line rather than dropped.

use std::sync::LazyLock;

use regex::Regex;

/// Open + close markers for the managed block. Everything between them
/// is owned by jj-gt and regenerated each submit; the AI prose above
/// is never touched.
const BLOCK_OPEN: &str = "<!-- jj-gt:links -->";
const BLOCK_CLOSE: &str = "<!-- /jj-gt:links -->";

/// Closing magic words (Linear's set, a superset of GitHub's). A
/// reference under one of these means "close this issue on merge".
const CLOSING_WORDS: &[&str] = &[
    "close",
    "closes",
    "closed",
    "closing",
    "fix",
    "fixes",
    "fixed",
    "fixing",
    "resolve",
    "resolves",
    "resolved",
    "resolving",
    "complete",
    "completes",
    "completed",
    "completing",
    "implement",
    "implements",
    "implemented",
    "implementing",
];

/// Non-closing magic words. A reference under one of these links the
/// issue without closing it. Multi-word phrases are matched as written
/// (spaces collapse to `\s+` in the pattern).
const NONCLOSING_WORDS: &[&str] = &[
    "ref",
    "refs",
    "references",
    "part of",
    "related to",
    "contributes to",
    "toward",
    "towards",
];

/// A single hoisted reference, after grouping + normalization.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HoistedRef {
    /// A reference parsed into a canonical issue ID, with its
    /// resolved closing/non-closing intent.
    Issue { id: String, closing: bool },
    /// A reference we recognized as magic-word-introduced but could
    /// not parse into an ID (a bare tracker URL). Hoisted verbatim on
    /// its own line, keyed by the whole captured span so dedupe still
    /// works.
    Verbatim(String),
}

impl HoistedRef {
    /// The line emitted into the managed block.
    fn render(&self) -> String {
        match self {
            HoistedRef::Issue { id, closing } => {
                if *closing {
                    format!("Closes {id}")
                } else {
                    format!("Refs {id}")
                }
            }
            HoistedRef::Verbatim(text) => text.clone(),
        }
    }
}

/// One `(magic-word, reference)` pair as captured from a message,
/// before grouping. Internal to extraction.
#[derive(Debug, Clone, PartialEq, Eq)]
struct RawRef {
    /// The reference token as captured (`SEA-1`, `#42`,
    /// `org/repo#7`, or a URL).
    reference: String,
    closing: bool,
}

/// Matches a magic word (closing or non-closing) followed by a
/// reference list. Capture group `list` holds the whole reference
/// list (`SEA-1, SEA-2 and SEA-3`); we re-scan it for individual
/// references with [`REF_TOKEN`].
///
/// The word alternation is sorted longest-first so multi-word phrases
/// (`part of`) win over any prefix, and `\b` anchors avoid matching
/// inside larger words (`prefix`, `refactor`).
static MAGIC_PHRASE: LazyLock<Regex> = LazyLock::new(|| {
    let mut words: Vec<String> = CLOSING_WORDS
        .iter()
        .chain(NONCLOSING_WORDS.iter())
        .map(|w| w.replace(' ', r"\s+"))
        .collect();
    // Longest-first so `references` is tried before `ref`, `part of`
    // before any single-word prefix, etc.
    words.sort_by_key(|w| std::cmp::Reverse(w.len()));
    let alt = words.join("|");
    // After the word, allow an optional colon/space, then capture the
    // reference list lazily up to end-of-line. The list itself is
    // validated token-by-token afterward, so over-capturing prose is
    // bounded by the line.
    let pat = format!(r"(?im)\b(?P<word>{alt})\b[:\s]+(?P<list>.+)$");
    Regex::new(&pat).expect("magic-phrase regex is valid")
});

/// A single reference token inside a list: tracker ID (`SEA-1`),
/// GitHub `#42` or `org/repo#42`, or a tracker URL. The list joiner
/// (`,` / `and`) is consumed by the caller's scan loop, not here.
static REF_TOKEN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?x)
        ^(?P<ref>
            https?://[^\s,]+            # tracker URL
          | [A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+\#\d+  # org/repo#42
          | \#\d+                       # #42
          | [A-Za-z]+-\d+               # SEA-123
        )",
    )
    .expect("ref-token regex is valid")
});

/// Matches a `Co-authored-by:` trailer line (GitHub is
/// case-insensitive on the key). Captures the `Name <email>` value
/// verbatim — we hoist it as-is rather than reconstructing it, so the
/// avatar/attribution GitHub derives from the email is preserved.
static COAUTHOR_TRAILER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?im)^\s*co-authored-by:\s*(?P<value>.+?)\s*$")
        .expect("co-author trailer regex is valid")
});

/// True if `word` (lowercased, internal whitespace collapsed to a
/// single space) is a closing magic word.
fn is_closing_word(word: &str) -> bool {
    let norm = collapse_ws(word).to_ascii_lowercase();
    CLOSING_WORDS.contains(&norm.as_str())
}

fn collapse_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Canonicalize a reference token for grouping. Tracker IDs uppercase
/// their project key (`sea-1` -> `SEA-1`) so two commits that wrote
/// the ID in different cases group together. URLs and `#42` forms are
/// left as-is (case already canonical / not applicable).
fn canonical_id(reference: &str) -> Option<String> {
    // URL or repo-qualified / bare GitHub number: keep verbatim, but
    // these still count as "parseable IDs" for grouping purposes only
    // when they're the `#42` / `org/repo#42` shapes. A URL stays
    // Verbatim.
    if reference.starts_with("http://") || reference.starts_with("https://") {
        return None;
    }
    if reference.starts_with('#') || reference.contains('#') {
        // `#42` or `org/repo#42` — canonical as written.
        return Some(reference.to_owned());
    }
    // Tracker ID `SEA-123`: uppercase the alpha key, keep the number.
    if let Some((key, num)) = reference.split_once('-')
        && !key.is_empty()
        && num.chars().all(|c| c.is_ascii_digit())
    {
        return Some(format!("{}-{}", key.to_ascii_uppercase(), num));
    }
    None
}

/// Scan one reference list (the text after a magic word) and pull out
/// the individual reference tokens, stopping at the first token that
/// is neither a reference nor a joiner (`,` / `and` / `&`). This is
/// the conservative boundary that keeps us from swallowing prose:
/// `Fixes SEA-1 because the old code was wrong` captures only `SEA-1`.
fn references_in_list(list: &str, closing: bool, out: &mut Vec<RawRef>) {
    let mut rest = list.trim_start();
    while let Some(m) = REF_TOKEN.find(rest) {
        let reference = m.as_str().trim_end_matches([',', '.', ';', ')']).to_owned();
        out.push(RawRef { reference, closing });
        rest = &rest[m.end()..];
        // Consume a joiner before looking for the next reference.
        // Anything else terminates the list. The `and` joiner needs a
        // word boundary after it, else a list like `SEA-1 android-5`
        // would consume the `and` prefix of `android` and then match
        // the `roid-5` tail as a spurious reference.
        let trimmed = rest.trim_start();
        let after_joiner = trimmed
            .strip_prefix(',')
            .or_else(|| {
                trimmed.strip_prefix("and").filter(|after| {
                    after.is_empty() || after.starts_with(|c: char| !c.is_ascii_alphanumeric())
                })
            })
            .or_else(|| trimmed.strip_prefix('&'));
        match after_joiner {
            Some(next) => rest = next.trim_start(),
            None => break,
        }
    }
}

/// Extract every magic-word reference across the given commit
/// messages, group by canonical issue ID, and normalize to one
/// [`HoistedRef`] per issue (closing wins). Verbatim (unparseable)
/// references are deduped by their captured text. First-seen order is
/// preserved across the whole message set.
pub fn extract_references(messages: &[String]) -> Vec<HoistedRef> {
    let mut raws: Vec<RawRef> = Vec::new();
    for msg in messages {
        for line in msg.lines() {
            for caps in MAGIC_PHRASE.captures_iter(line) {
                let closing = is_closing_word(&caps["word"]);
                references_in_list(&caps["list"], closing, &mut raws);
            }
        }
    }
    group_and_normalize(&raws)
}

/// Extract every distinct `Co-authored-by:` trailer across the commit
/// messages, deduped by identity (the `Name <email>` value), in
/// first-seen order. Returns the verbatim trailer values (without the
/// `Co-authored-by:` key); the caller renders them as trailer lines.
///
/// Why this matters: the Graphite squash-merge body = PR title +
/// description, so a `Co-Authored-By: seal <…>` trailer that lives
/// only in the source commits is dropped on squash — and GitHub
/// derives co-authorship ONLY from a trailer in the merged commit.
/// Hoisting the trailer into the description's trailing block puts it
/// in the squash body where GitHub will parse it.
pub fn extract_coauthors(messages: &[String]) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for msg in messages {
        for caps in COAUTHOR_TRAILER.captures_iter(msg) {
            let value = caps["value"].trim().to_owned();
            if value.is_empty() {
                continue;
            }
            // Dedupe by a case-insensitive identity key so the same
            // co-author written with different casing collapses; emit
            // the first-seen verbatim form.
            let key = value.to_ascii_lowercase();
            if seen.insert(key) {
                out.push(value);
            }
        }
    }
    out
}

/// Group raw `(reference, closing)` pairs into normalized hoisted
/// refs. Closing intent wins per issue; first-seen order preserved.
fn group_and_normalize(raws: &[RawRef]) -> Vec<HoistedRef> {
    // Order of first appearance, keyed by the grouping key (canonical
    // ID for parseable refs, the verbatim text otherwise).
    let mut order: Vec<String> = Vec::new();
    let mut closing_by_key: std::collections::HashMap<String, bool> =
        std::collections::HashMap::new();
    let mut is_verbatim: std::collections::HashMap<String, bool> = std::collections::HashMap::new();
    let mut id_by_key: std::collections::HashMap<String, String> = std::collections::HashMap::new();

    for raw in raws {
        let (key, verbatim) = match canonical_id(&raw.reference) {
            Some(id) => (id.clone(), false),
            None => (raw.reference.clone(), true),
        };
        if !closing_by_key.contains_key(&key) {
            order.push(key.clone());
            is_verbatim.insert(key.clone(), verbatim);
            if !verbatim {
                id_by_key.insert(key.clone(), key.clone());
            }
        }
        // Closing wins: once any commit closed this issue, it stays
        // closing.
        let entry = closing_by_key.entry(key.clone()).or_insert(false);
        *entry = *entry || raw.closing;
    }

    order
        .into_iter()
        .map(|key| {
            let closing = closing_by_key[&key];
            if is_verbatim[&key] {
                HoistedRef::Verbatim(key)
            } else {
                HoistedRef::Issue {
                    id: id_by_key[&key].clone(),
                    closing,
                }
            }
        })
        .collect()
}

/// Render the fenced reference lines (the content between the fence
/// markers), or `None` when there are no references to hoist.
///
/// References only — co-authors are rendered separately, *after* the
/// close fence, so they remain the contiguous trailing block GitHub
/// needs to parse `Co-Authored-By:` trailers on squash (a fence line
/// after them would break trailer detection). See [`reconcile_body`].
///
/// `ai_body` is the current PR description; a reference already
/// present verbatim in it (the AI echoed `Closes SEA-1` into its
/// prose) is skipped so the block doesn't duplicate it.
fn render_ref_lines(refs: &[HoistedRef], ai_body: &str) -> Option<String> {
    let lines: Vec<String> = refs
        .iter()
        .map(HoistedRef::render)
        .filter(|line| !body_already_has(ai_body, line))
        .collect();
    if lines.is_empty() {
        None
    } else {
        Some(lines.join("\n"))
    }
}

/// Render the trailing `Co-Authored-By:` trailer lines, or `None` when
/// there are none to hoist. These go last in the body (after the
/// fence) so GitHub parses them as commit-message trailers on squash.
///
/// A trailer already present verbatim in `ai_body` is skipped.
fn render_coauthor_lines(coauthors: &[String], ai_body: &str) -> Option<String> {
    let lines: Vec<String> = coauthors
        .iter()
        .map(|value| format!("Co-Authored-By: {value}"))
        .filter(|line| !body_already_has(ai_body, line))
        .collect();
    if lines.is_empty() {
        None
    } else {
        Some(lines.join("\n"))
    }
}

/// True if the AI body already contains `line` as an exact standalone
/// line (after trimming). Only a line whose trimmed content equals
/// `line` exactly suppresses the corresponding block entry: `Closes
/// SEA-12` does not satisfy `Closes SEA-1`, and a prose line like
/// `See Closes SEA-1 above` does not satisfy `Closes SEA-1` either.
fn body_already_has(body: &str, line: &str) -> bool {
    body.lines().any(|l| l.trim() == line)
}

/// True if `line` (trimmed) is a `Co-Authored-By:` trailer.
fn is_coauthor_line(line: &str) -> bool {
    let t = line.trim();
    let prefix = "co-authored-by:";
    t.len() >= prefix.len() && t[..prefix.len()].eq_ignore_ascii_case(prefix)
}

/// Strip everything jj-gt manages — the fenced reference block AND the
/// trailing `Co-Authored-By:` trailer run that follows it — from
/// `body`, returning the AI-owned prose alone. Idempotent: feeding a
/// previously-reconciled body back in yields the original prose.
///
/// The trailing co-author run is peeled first (it's the final
/// paragraph), then the fence. Tolerant of a missing close marker
/// (truncated block) by cutting to end-of-string from the open marker.
fn strip_managed(body: &str) -> String {
    let without_trailers = strip_trailing_coauthors(body);
    strip_fence(&without_trailers)
}

/// Remove a trailing paragraph composed solely of `Co-Authored-By:`
/// lines (and the blank line separating it from what precedes). A
/// paragraph with any non-trailer line is left untouched — we only own
/// a pure trailer block.
fn strip_trailing_coauthors(body: &str) -> String {
    let trimmed = body.trim_end_matches(['\n', '\r', ' ', '\t']);
    // Walk lines from the end, collecting a contiguous run of
    // co-author lines (blank lines within/around the run are skipped).
    // Stop at the first non-coauthor, non-blank line.
    let lines: Vec<&str> = trimmed.lines().collect();
    let mut coauthor_start: Option<usize> = None;
    let mut i = lines.len();
    while i > 0 {
        let line = lines[i - 1];
        if line.trim().is_empty() {
            i -= 1;
            continue;
        }
        if is_coauthor_line(line) {
            coauthor_start = Some(i - 1);
            i -= 1;
            continue;
        }
        break;
    }
    match coauthor_start {
        None => body.to_owned(),
        Some(start) => lines[..start]
            .join("\n")
            .trim_end_matches(['\n', '\r', ' ', '\t'])
            .to_owned(),
    }
}

/// Remove the fenced managed block (and the blank line preceding it)
/// from `body`, returning the prose alone.
fn strip_fence(body: &str) -> String {
    let Some(open) = body.find(BLOCK_OPEN) else {
        return body.to_owned();
    };
    let before = body[..open].trim_end_matches(['\n', '\r', ' ', '\t']);
    let after = match body[open..].find(BLOCK_CLOSE) {
        Some(rel) => &body[open + rel + BLOCK_CLOSE.len()..],
        None => "",
    };
    let after = after.trim_start_matches(['\n', '\r']);
    if after.is_empty() {
        before.to_owned()
    } else {
        format!("{before}\n{after}")
    }
}

/// Reconcile the managed content into `body`. Strips any existing
/// managed content first (idempotent), then appends a freshly-rendered
/// fenced reference block followed by a trailing co-author paragraph,
/// either of which is omitted when empty.
///
/// Layout, when both are present:
///
/// ```text
/// <prose>
///
/// <!-- jj-gt:links -->
/// Closes SEA-1
/// <!-- /jj-gt:links -->
///
/// Co-Authored-By: seal <…>
/// ```
///
/// The co-author trailers are the final paragraph so GitHub parses
/// them as commit-message trailers when the PR is squash-merged.
pub fn reconcile_body(body: &str, refs: &[HoistedRef], coauthors: &[String]) -> String {
    let prose = strip_managed(body);
    let fence =
        render_ref_lines(refs, &prose).map(|lines| format!("{BLOCK_OPEN}\n{lines}\n{BLOCK_CLOSE}"));
    let trailers = render_coauthor_lines(coauthors, &prose);

    let mut sections: Vec<String> = Vec::new();
    let prose_trimmed = prose.trim_end_matches(['\n', '\r', ' ', '\t']);
    if !prose_trimmed.is_empty() {
        sections.push(prose_trimmed.to_owned());
    }
    if let Some(fence) = fence {
        sections.push(fence);
    }
    if let Some(trailers) = trailers {
        sections.push(trailers);
    }
    sections.join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msgs(s: &[&str]) -> Vec<String> {
        s.iter().map(|x| (*x).to_owned()).collect()
    }

    // ---- extraction: single + multi reference --------------------

    #[test]
    fn single_closing_reference() {
        let out = extract_references(&msgs(&["feat: thing\n\nCloses SEA-1"]));
        assert_eq!(
            out,
            vec![HoistedRef::Issue {
                id: "SEA-1".into(),
                closing: true
            }]
        );
    }

    #[test]
    fn single_nonclosing_reference() {
        let out = extract_references(&msgs(&["feat: thing\n\nRefs SEA-9"]));
        assert_eq!(
            out,
            vec![HoistedRef::Issue {
                id: "SEA-9".into(),
                closing: false
            }]
        );
    }

    #[test]
    fn multi_reference_list_with_and() {
        // `Fixes A, B and C` expands to three issues, all closing.
        let out = extract_references(&msgs(&["Fixes ENG-123, DES-5 and ENG-256"]));
        assert_eq!(
            out,
            vec![
                HoistedRef::Issue {
                    id: "ENG-123".into(),
                    closing: true
                },
                HoistedRef::Issue {
                    id: "DES-5".into(),
                    closing: true
                },
                HoistedRef::Issue {
                    id: "ENG-256".into(),
                    closing: true
                },
            ]
        );
    }

    #[test]
    fn closing_wins_across_commits() {
        // One commit refs the issue, another closes it. The closing
        // intent must win.
        let out = extract_references(&msgs(&["Refs SEA-1", "Closes SEA-1"]));
        assert_eq!(
            out,
            vec![HoistedRef::Issue {
                id: "SEA-1".into(),
                closing: true
            }]
        );
    }

    #[test]
    fn refs_only_stays_refs() {
        let out = extract_references(&msgs(&["Refs SEA-1", "Related to SEA-1"]));
        assert_eq!(
            out,
            vec![HoistedRef::Issue {
                id: "SEA-1".into(),
                closing: false
            }]
        );
    }

    #[test]
    fn multi_word_nonclosing_phrase() {
        let out = extract_references(&msgs(&["Part of DES-9"]));
        assert_eq!(
            out,
            vec![HoistedRef::Issue {
                id: "DES-9".into(),
                closing: false
            }]
        );
    }

    #[test]
    fn case_insensitive_word_and_id() {
        // Mixed-case keyword + lowercased project key must still
        // group and normalize.
        let out = extract_references(&msgs(&["fIxEs sea-7"]));
        assert_eq!(
            out,
            vec![HoistedRef::Issue {
                id: "SEA-7".into(),
                closing: true
            }]
        );
    }

    #[test]
    fn dedupe_same_issue_multiple_commits() {
        let out = extract_references(&msgs(&["Closes SEA-1", "Closes SEA-1", "fixes SEA-1"]));
        assert_eq!(
            out,
            vec![HoistedRef::Issue {
                id: "SEA-1".into(),
                closing: true
            }]
        );
    }

    #[test]
    fn no_magic_word_yields_empty() {
        let out = extract_references(&msgs(&["chore: bump deps\n\nNo references here."]));
        assert!(out.is_empty());
    }

    #[test]
    fn first_seen_order_preserved() {
        let out = extract_references(&msgs(&["Refs DES-5", "Closes SEA-1"]));
        assert_eq!(
            out,
            vec![
                HoistedRef::Issue {
                    id: "DES-5".into(),
                    closing: false
                },
                HoistedRef::Issue {
                    id: "SEA-1".into(),
                    closing: true
                },
            ]
        );
    }

    // ---- reference-token boundary --------------------------------

    #[test]
    fn stops_at_prose_after_reference() {
        // Must not swallow the trailing prose.
        let out = extract_references(&msgs(&["Fixes SEA-1 because the old code was wrong"]));
        assert_eq!(
            out,
            vec![HoistedRef::Issue {
                id: "SEA-1".into(),
                closing: true
            }]
        );
    }

    #[test]
    fn github_hash_reference() {
        let out = extract_references(&msgs(&["Closes #42"]));
        assert_eq!(
            out,
            vec![HoistedRef::Issue {
                id: "#42".into(),
                closing: true
            }]
        );
    }

    #[test]
    fn github_repo_qualified_reference() {
        let out = extract_references(&msgs(&["Fixes org/repo#7"]));
        assert_eq!(
            out,
            vec![HoistedRef::Issue {
                id: "org/repo#7".into(),
                closing: true
            }]
        );
    }

    #[test]
    fn tracker_url_hoisted_verbatim() {
        let out = extract_references(&msgs(&["Refs https://linear.app/ws/issue/ABC-1/x"]));
        assert_eq!(
            out,
            vec![HoistedRef::Verbatim(
                "https://linear.app/ws/issue/ABC-1/x".into()
            )]
        );
    }

    #[test]
    fn trailing_punctuation_stripped_from_id() {
        let out = extract_references(&msgs(&["Closes SEA-1."]));
        assert_eq!(
            out,
            vec![HoistedRef::Issue {
                id: "SEA-1".into(),
                closing: true
            }]
        );
    }

    #[test]
    fn word_boundary_avoids_false_match() {
        // `refactor` must not match `ref`; `prefix` must not match.
        let out = extract_references(&msgs(&["refactor: tidy prefix handling SEA-9"]));
        assert!(out.is_empty(), "got {out:?}");
    }

    #[test]
    fn and_joiner_requires_word_boundary() {
        // `SEA-1 android-5` must NOT consume the `and` prefix of
        // `android` and spuriously hoist `roid-5`. Only SEA-1 is a
        // valid reference; `android-5` is not preceded by a real
        // joiner, so the list ends after SEA-1.
        let out = extract_references(&msgs(&["Closes SEA-1 android-5"]));
        assert_eq!(
            out,
            vec![HoistedRef::Issue {
                id: "SEA-1".into(),
                closing: true
            }]
        );
    }

    #[test]
    fn and_joiner_still_works_as_real_joiner() {
        // The boundary guard must not break the legitimate `and`
        // joiner with following whitespace.
        let out = extract_references(&msgs(&["Closes SEA-1 and SEA-2"]));
        assert_eq!(
            out,
            vec![
                HoistedRef::Issue {
                    id: "SEA-1".into(),
                    closing: true
                },
                HoistedRef::Issue {
                    id: "SEA-2".into(),
                    closing: true
                },
            ]
        );
    }

    // ---- block reconcile -----------------------------------------

    fn issue(id: &str, closing: bool) -> HoistedRef {
        HoistedRef::Issue {
            id: id.into(),
            closing,
        }
    }

    #[test]
    fn reconcile_into_empty_body() {
        let out = reconcile_body("", &[issue("SEA-1", true)], &[]);
        assert_eq!(out, format!("{BLOCK_OPEN}\nCloses SEA-1\n{BLOCK_CLOSE}"));
    }

    #[test]
    fn reconcile_appends_after_prose() {
        let out = reconcile_body("AI summary of the change.", &[issue("SEA-1", true)], &[]);
        assert_eq!(
            out,
            format!("AI summary of the change.\n\n{BLOCK_OPEN}\nCloses SEA-1\n{BLOCK_CLOSE}")
        );
    }

    #[test]
    fn reconcile_is_idempotent() {
        let once = reconcile_body("Prose.", &[issue("SEA-1", true)], &[]);
        let twice = reconcile_body(&once, &[issue("SEA-1", true)], &[]);
        assert_eq!(once, twice);
    }

    #[test]
    fn reconcile_updates_existing_block() {
        let first = reconcile_body("Prose.", &[issue("SEA-1", true)], &[]);
        // A review commit added SEA-2; re-running must reflect both
        // and not duplicate the block.
        let second = reconcile_body(&first, &[issue("SEA-1", true), issue("SEA-2", false)], &[]);
        assert_eq!(
            second,
            format!("Prose.\n\n{BLOCK_OPEN}\nCloses SEA-1\nRefs SEA-2\n{BLOCK_CLOSE}")
        );
        assert_eq!(second.matches(BLOCK_OPEN).count(), 1);
    }

    #[test]
    fn reconcile_skips_phrase_already_in_ai_prose() {
        // The AI echoed `Closes SEA-1` into its prose; the block must
        // not duplicate it. With only that one ref, no block at all.
        let body = "This change closes the issue.\n\nCloses SEA-1";
        let out = reconcile_body(body, &[issue("SEA-1", true)], &[]);
        assert_eq!(out, body, "no block should be added");
    }

    #[test]
    fn reconcile_no_refs_strips_stale_block() {
        let with_block = format!("Prose.\n\n{BLOCK_OPEN}\nCloses SEA-1\n{BLOCK_CLOSE}");
        let out = reconcile_body(&with_block, &[], &[]);
        assert_eq!(out, "Prose.");
    }

    #[test]
    fn reconcile_no_refs_empty_body_stays_empty() {
        assert_eq!(reconcile_body("", &[], &[]), "");
    }

    #[test]
    fn reconcile_verbatim_url() {
        let out = reconcile_body(
            "Prose.",
            &[HoistedRef::Verbatim(
                "https://linear.app/ws/issue/ABC-1/x".into(),
            )],
            &[],
        );
        assert_eq!(
            out,
            format!("Prose.\n\n{BLOCK_OPEN}\nhttps://linear.app/ws/issue/ABC-1/x\n{BLOCK_CLOSE}")
        );
    }

    // ---- co-author extraction + hoisting -------------------------

    #[test]
    fn single_coauthor_extracted() {
        let out = extract_coauthors(&msgs(&[
            "feat: thing\n\nCo-Authored-By: seal <noreply@sealedsecurity.com>",
        ]));
        assert_eq!(out, vec!["seal <noreply@sealedsecurity.com>".to_owned()]);
    }

    #[test]
    fn coauthors_union_deduped_across_commits() {
        // Same coauthor on N commits collapses to one; a distinct
        // second coauthor is kept.
        let out = extract_coauthors(&msgs(&[
            "a\n\nCo-authored-by: seal <noreply@sealedsecurity.com>",
            "b\n\nCo-authored-by: seal <noreply@sealedsecurity.com>",
            "c\n\nCo-authored-by: Jane Dev <jane@example.com>",
        ]));
        assert_eq!(
            out,
            vec![
                "seal <noreply@sealedsecurity.com>".to_owned(),
                "Jane Dev <jane@example.com>".to_owned(),
            ]
        );
    }

    #[test]
    fn coauthor_key_is_case_insensitive_on_dedup() {
        // GitHub is case-insensitive on the key; dedupe on identity.
        let out = extract_coauthors(&msgs(&[
            "a\n\nco-authored-by: Seal <noreply@sealedsecurity.com>",
            "b\n\nCo-Authored-By: seal <noreply@sealedsecurity.com>",
        ]));
        assert_eq!(out.len(), 1, "got {out:?}");
    }

    #[test]
    fn no_coauthor_yields_empty() {
        let out = extract_coauthors(&msgs(&["chore: bump\n\nNo trailer here."]));
        assert!(out.is_empty());
    }

    #[test]
    fn reconcile_coauthor_after_references() {
        // Refs stay fenced; co-author trailers render as the final
        // paragraph AFTER the close fence so they're the trailing
        // trailer block GitHub parses on squash.
        let out = reconcile_body(
            "Prose.",
            &[issue("SEA-1", true)],
            &["seal <noreply@sealedsecurity.com>".to_owned()],
        );
        assert_eq!(
            out,
            format!(
                "Prose.\n\n{BLOCK_OPEN}\nCloses SEA-1\n{BLOCK_CLOSE}\n\n\
                 Co-Authored-By: seal <noreply@sealedsecurity.com>"
            )
        );
    }

    #[test]
    fn reconcile_coauthor_is_last_line() {
        // The structural guarantee the feature depends on: the very
        // last line of the body is a Co-Authored-By trailer, NOT the
        // close fence.
        let out = reconcile_body(
            "Prose.",
            &[issue("SEA-1", true)],
            &["seal <noreply@sealedsecurity.com>".to_owned()],
        );
        let last = out.lines().last().unwrap();
        assert!(
            is_coauthor_line(last),
            "last line must be a co-author trailer, got: {last:?}"
        );
    }

    #[test]
    fn reconcile_coauthor_only_no_references() {
        // No refs → no fence; co-author trailer is the trailing
        // paragraph directly after the prose.
        let out = reconcile_body(
            "Prose.",
            &[],
            &["seal <noreply@sealedsecurity.com>".to_owned()],
        );
        assert_eq!(
            out,
            "Prose.\n\nCo-Authored-By: seal <noreply@sealedsecurity.com>"
        );
    }

    #[test]
    fn reconcile_coauthor_idempotent() {
        let coauthors = vec!["seal <noreply@sealedsecurity.com>".to_owned()];
        let once = reconcile_body("Prose.", &[issue("SEA-1", true)], &coauthors);
        let twice = reconcile_body(&once, &[issue("SEA-1", true)], &coauthors);
        assert_eq!(once, twice);
        assert_eq!(twice.matches(BLOCK_OPEN).count(), 1);
    }

    #[test]
    fn reconcile_coauthor_already_present_is_stable() {
        // A body that already ends in the canonical co-author trailer
        // round-trips unchanged: strip_managed peels the trailing
        // trailer run, then it's re-rendered identically. (Idempotency
        // for the "trailer already there" case.)
        let body = "Summary.\n\nCo-Authored-By: seal <noreply@sealedsecurity.com>";
        let out = reconcile_body(body, &[], &["seal <noreply@sealedsecurity.com>".to_owned()]);
        assert_eq!(out, body);
    }

    #[test]
    fn strip_managed_leaves_prose_with_human_trailer_paragraph_alone() {
        // A trailing paragraph that ISN'T purely co-author lines is
        // human prose, not managed — strip_managed must not eat it.
        let body = "Summary.\n\nSee the design doc for details.";
        let out = reconcile_body(body, &[], &[]);
        assert_eq!(out, body);
    }
}
