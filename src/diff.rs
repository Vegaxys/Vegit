//! Parsing of unified diffs into hunks, so single hunks can be staged or unstaged.
//!
//! Parsing runs on the worker thread. It also builds a flat row index so the view
//! can draw only the visible lines: a Unity scene diff can have 100 000+ lines.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineKind {
    Context,
    Added,
    Removed,
    Note,
}

#[derive(Clone, Debug)]
pub struct DiffLine {
    pub kind: LineKind,
    pub old_no: Option<u32>,
    pub new_no: Option<u32>,
    pub text: String,
}

#[derive(Clone, Debug)]
pub struct Hunk {
    pub header: String,
    pub lines: Vec<DiffLine>,
    /// The raw hunk text (header included), replayed with `git apply`.
    raw: String,
}

/// One drawable row of the diff view.
#[derive(Clone, Copy, Debug)]
pub enum DiffRow {
    Header(usize),
    Line(usize, usize),
    Gap,
}

/// One row of the split view: the old line on the left, the new one on the right.
#[derive(Clone, Copy, Debug)]
pub enum SplitRow {
    Header(usize),
    Pair(Option<(usize, usize)>, Option<(usize, usize)>),
    Gap,
}

/// How a change is shown, like the three view buttons of a classic git client.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum DiffMode {
    /// Only the changed parts with a few lines around them, each with stage/discard buttons.
    #[default]
    Hunk,
    /// The whole file, with removed and added lines in place.
    Inline,
    /// The whole file, old version on the left and new version on the right.
    Split,
}

#[derive(Clone, Debug, Default)]
pub struct ParsedDiff {
    /// `diff --git` / `---` / `+++` lines, needed to build a patch for one hunk.
    header: String,
    pub hunks: Vec<Hunk>,
    /// Hunk view rows (with a header per hunk).
    pub rows: Vec<DiffRow>,
    /// Inline view rows (lines only).
    pub inline_rows: Vec<DiffRow>,
    pub split_rows: Vec<SplitRow>,
    /// Row index where each change starts, per view, for the previous/next buttons.
    pub starts_hunk: Vec<usize>,
    pub starts_inline: Vec<usize>,
    pub starts_split: Vec<usize>,
    pub binary: bool,
    pub truncated: bool,
    pub added: usize,
    pub removed: usize,
    /// Length of the longest line, to size the horizontal scroll.
    pub max_chars: usize,
}

impl ParsedDiff {
    pub fn parse(text: &str) -> Self {
        let mut d = ParsedDiff::default();
        let mut current: Option<Hunk> = None;
        let (mut old_no, mut new_no) = (0u32, 0u32);
        for line in text.split_inclusive('\n') {
            let bare = line.trim_end_matches(['\n', '\r']);
            if bare.starts_with("@@") {
                if let Some(h) = current.take() {
                    d.hunks.push(h);
                }
                let (o, n) = parse_hunk_start(bare);
                old_no = o;
                new_no = n;
                current = Some(Hunk { header: bare.to_string(), lines: Vec::new(), raw: line.to_string() });
                continue;
            }
            match current.as_mut() {
                None => {
                    if bare.starts_with("Binary files") {
                        d.binary = true;
                    }
                    d.header.push_str(line);
                }
                Some(h) => {
                    h.raw.push_str(line);
                    let (kind, text) = match bare.chars().next() {
                        Some('+') => (LineKind::Added, &bare[1..]),
                        Some('-') => (LineKind::Removed, &bare[1..]),
                        Some('\\') => (LineKind::Note, bare),
                        Some(_) => (LineKind::Context, &bare[1..]),
                        None => (LineKind::Context, ""),
                    };
                    let (o, n) = match kind {
                        LineKind::Added => {
                            d.added += 1;
                            new_no += 1;
                            (None, Some(new_no - 1))
                        }
                        LineKind::Removed => {
                            d.removed += 1;
                            old_no += 1;
                            (Some(old_no - 1), None)
                        }
                        LineKind::Context => {
                            old_no += 1;
                            new_no += 1;
                            (Some(old_no - 1), Some(new_no - 1))
                        }
                        LineKind::Note => (None, None),
                    };
                    let text = if text.contains('\t') { text.replace('\t', "    ") } else { text.to_string() };
                    d.max_chars = d.max_chars.max(text.len());
                    h.lines.push(DiffLine { kind, old_no: o, new_no: n, text });
                }
            }
        }
        if let Some(h) = current.take() {
            d.hunks.push(h);
        }
        for (hi, h) in d.hunks.iter().enumerate() {
            if hi > 0 {
                d.rows.push(DiffRow::Gap);
                d.inline_rows.push(DiffRow::Gap);
                d.split_rows.push(SplitRow::Gap);
            }
            d.starts_hunk.push(d.rows.len());
            d.rows.push(DiffRow::Header(hi));
            d.rows.extend((0..h.lines.len()).map(|li| DiffRow::Line(hi, li)));

            let changed = |li: usize| matches!(h.lines[li].kind, LineKind::Added | LineKind::Removed);
            for li in 0..h.lines.len() {
                if changed(li) && (li == 0 || !changed(li - 1)) {
                    d.starts_inline.push(d.inline_rows.len());
                }
                d.inline_rows.push(DiffRow::Line(hi, li));
            }

            // Split: pair each block of removed lines with the block of added lines after it.
            d.split_rows.push(SplitRow::Header(hi));
            let mut li = 0;
            while li < h.lines.len() {
                match h.lines[li].kind {
                    LineKind::Context | LineKind::Note => {
                        d.split_rows.push(SplitRow::Pair(Some((hi, li)), Some((hi, li))));
                        li += 1;
                    }
                    _ => {
                        let start = li;
                        while li < h.lines.len() && h.lines[li].kind == LineKind::Removed {
                            li += 1;
                        }
                        let removed: Vec<usize> = (start..li).collect();
                        let mid = li;
                        while li < h.lines.len() && h.lines[li].kind == LineKind::Added {
                            li += 1;
                        }
                        let added: Vec<usize> = (mid..li).collect();
                        d.starts_split.push(d.split_rows.len());
                        for k in 0..removed.len().max(added.len()) {
                            d.split_rows.push(SplitRow::Pair(removed.get(k).map(|&l| (hi, l)), added.get(k).map(|&l| (hi, l))));
                        }
                    }
                }
            }
        }
        d
    }

    pub fn starts(&self, mode: DiffMode) -> &[usize] {
        match mode {
            DiffMode::Hunk => &self.starts_hunk,
            DiffMode::Inline => &self.starts_inline,
            DiffMode::Split => &self.starts_split,
        }
    }

    /// A complete patch containing only hunk `index`, for `git apply`.
    pub fn hunk_patch(&self, index: usize) -> Option<String> {
        let h = self.hunks.get(index)?;
        let mut p = self.header.clone();
        p.push_str(&h.raw);
        if !p.ends_with('\n') {
            p.push('\n');
        }
        Some(p)
    }
}

fn parse_hunk_start(header: &str) -> (u32, u32) {
    // "@@ -12,7 +12,9 @@ fn name"
    let mut old = 1;
    let mut new = 1;
    for part in header.split_whitespace().skip(1).take(2) {
        let num = part[1..].split(',').next().unwrap_or("1").parse().unwrap_or(1);
        if part.starts_with('-') {
            old = num;
        } else if part.starts_with('+') {
            new = num;
        }
    }
    (old, new)
}
