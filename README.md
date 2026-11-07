# Vegit

A fast, friendly desktop Git client written in Rust with [egui](https://github.com/emilk/egui) (eframe).
No AI features, no account, no subscription: it drives your own `git` install, so credentials,
hooks, config and SSH keys behave exactly as they do in a terminal.

[![Vegit – A Fast, Friendly Desktop Git Client](https://img.youtube.com/vi/QB5GjLwnyeY/maxresdefault.jpg)](https://youtu.be/QB5GjLwnyeY)

## Features

- **Commit graph**: branches, tags, remotes, merges and stashes laid out in stable lanes, with commit search.
  Columns can be resized, shown or hidden from the gear at the end of the header (with a compact layout and
  a reset), the current branch is pinned to the left, and clicking "Author" filters commits by author.
  Dates read `dd-MM-yyyy HH:mm`. A branch label too long for its column is cut with "…", and labels that
  do not fit fold into a "+N" badge whose list opens on hover. Hold the middle mouse button and drag to pan
  the history. Loads the last 3,000 commits so refreshes stay quick.
- **Right-click menu** on any commit: push, pull, bring a branch into the current one or replay the current
  one on top of it, start a pull request, rename or delete a branch, and copy the SHA, commit link, branch
  link or branch name. Double-click a branch label to check it out.
- **Merge options**, in the branch menus: « Bring feature into main » (merge), « … as one commit » (squash:
  the changes are staged and the commit message is prefilled with the list of combined commits),
  « Replay main on top of feature » (rebase), and under « More options » « Always create a merge commit »
  (`--no-ff`) and « Only if no merge commit is needed » (`--ff-only`). The git command is in each tooltip.
- **Staging**: stage, unstage or discard files one by one, in bulk or by folder, and stage or unstage
  single hunks. File lists switch between full paths and a folder tree ("Path" / "Tree"), sorted by name
  or by kind of change.
- **Diffs**: hunk, inline or split view, virtualized so huge diffs (Unity scenes, big asset imports) stay smooth.
  Image changes (PNG, JPEG, GIF, WebP, BMP, ICO, TGA, SVG) can be rendered side by side.
- **Commit**: summary and description, amend the previous commit, undo / redo the last commit.
- **Branches and remotes**: create, check out (local or remote), fetch, pull, push (with force), stash and pop.
- **Remotes**: the "+" of the remotes header opens « Add remote »: paste an address, Vegit checks it answers
  (`ls-remote`, errors explained in plain words) and proposes a name (`origin`, or `upstream` for the original
  of a fork). Right-click a remote's name to fetch it alone, rename it, change or copy its URL, open it on
  GitHub/GitLab/…, or remove it (nothing is deleted on the server).
- **Push…** (right-click Push): choose the remote and the branch name on the server, and also push the new
  tags on the pushed commits (one by one, lightweight tags included). The toolbar Push stays one click and
  only asks where to push for a first push when there are several remotes.
- **Tags on the server**: after each fetch or push Vegit asks the server which tags it has; the others get a
  « local » badge. Right-click a tag to push it; creating a tag offers « Push it to origin » in the toast;
  deleting a tag can also delete it on the server.
- **Upstream**: right-click a local branch to « Track a remote branch… » or « Stop tracking ». A branch whose
  remote branch was deleted shows « gone » and offers « Delete local branch too ».
- **Deleting a branch** happens at once with an Undo in the toast. If the branch has commits that no other
  branch has, they are listed first and « Delete anyway » is needed.
- **Undo for every local action**: commits, merges, rebases, resets (hard ones too), branch and tag
  changes, stashes and discards are recorded; the toolbar says what it takes back (« Undo rebase »,
  « Undo discard (3 files) »), each toast has its Undo, and Ctrl+Z / Ctrl+Y work. Files a discard or a hard
  reset would lose are saved first in `refs/vegit/backups/` (kept 30 days). Once a branch is pushed, its
  Undo is greyed: undoing would need a force push. Discards, hard resets and stash deletes no longer ask
  for confirmation.
- **Activity** (sidebar): the recent history of the repository (reflog) in plain sentences; click a line
  to show it, right-click to restore it as a new branch.
- **Drag and drop**: drag a branch (graph label or sidebar) onto a local branch for « Bring … into … »,
  « … as one commit », « Replay … on top of … » or the « More options ».
- **Picking several commits**: Ctrl+click and Shift+click in the graph; the detail panel lists them.
- **Editing history** (right-click a commit of the current branch): « Edit message… », « Combine with the
  commit below » (keep both messages, keep the one below, or write a new one), « Remove this commit »,
  « Move up » / « Move down », each in one click with Undo. « Edit history from here… » turns the detail
  panel into the list of commits: drag to reorder, pick what happens to each, then « Apply N changes ». It
  warns when commits are already on the server and refuses ranges with merge commits. No editor ever opens:
  git receives the prepared list from Vegit itself (`vegit --sequence-editor`).
- **Add to an earlier commit…** (commit panel): the staged changes go into a commit not pushed yet
  (`commit --fixup` + `rebase --autosquash`).
- **Conflicts block by block**: a conflicted file opens as cards, both versions side by side and titled with
  the branch names (even during a rebase, where git swaps "ours" and "theirs"), the original underneath
  (« Show the original », zdiff3 style), and « Keep main », « Keep feature », « Keep both », « Edit… ».
  « 2 of 5 conflicts left »; the last choice marks the file resolved. « Open in my merge tool » too.
- **Worktrees**: « Open in a separate folder… » on a branch (or « Work on it in a separate folder » when
  creating one) makes a folder and a tab; a « Worktrees » section lists them with a dot when they have
  changes; branch labels open elsewhere carry a folder badge and double-click goes to that tab; switching to
  such a branch offers « Go there »; « Remove folder… » offers to stash the changes first; vanished folders
  are pruned on refresh.
- **Git LFS** (for Unity projects): a banner when the project uses LFS but LFS is missing; « LFS 12.4 MB »
  badges in the file lists; previews show the real picture behind a pointer; staging a file over 50 MB
  suggests storing its kind with LFS; « Set up LFS for Unity… » (editable patterns, scenes and prefabs
  lockable); « Lock so others don't edit it » / « Unlock » on files; a « Big files (LFS) » section lists
  my locks and others' (refreshed after each fetch), with « Force unlock » under More options; files locked
  by someone else show « Locked by Alice since Tuesday »; after a push, a file I still lock offers « Unlock it ».
- **Identity**: when git doesn't know your name or e-mail, the commit panel asks « Who are you? » (for all
  projects or only this one). Settings → Identity shows both levels and which one applies.
- **File history and blame** for any file.
- **Merge conflicts**: while a merge, rebase, cherry-pick or revert is in progress, a banner counts the
  conflicted files and offers Continue (once none is left) and Abort, plus « Skip this commit » during a
  rebase, cherry-pick or revert. Right-click a conflicted file to keep
  your version ("ours"), keep theirs, or mark it as resolved.
- **Pull requests** on GitHub, GitLab (including self-hosted), Bitbucket Cloud and Azure DevOps:
  list, check out and create them (drafts too).
- **Private repositories**: sign in with a token saved through git's credential helper
  (Git Credential Manager on Windows), or generate an SSH key. The app never stores secrets itself.
- **Tabs** for several repositories, clone / init / open, submodules, "open in terminal / file explorer".
  The "+" tab opens the welcome page: open, clone or create a repository, set up access to private
  repositories, try the sample, or reopen a recent one.
- **Sidebar**: branches grouped into folders (`feature/…`), with merge and rebase actions.
- **Appearance settings**: theme presets (Midnight, Dracula, Monokai, Nord, Dusk, Paper and Solarized light),
  interface color, UI and code fonts (including a custom font file), text size, and branch colors
  (theme, gradient, rainbow, random, a single color or the interface color).
  A halftone background sits behind the panels: dots, circles, squares, diamonds, lines, crosses, plus signs
  or triangles, shaded by a horizontal, vertical, diagonal or radial gradient or by noise, with adjustable
  strength and cell size. It can be turned off, or picked at random at each launch (the default).
- **Author pictures** from GitHub (for `users.noreply.github.com` addresses) or Gravatar, with initials as
  a fallback. This sends a hash of each author e-mail to those services, so it can be turned off in the settings.
- **Non-blocking UI**: git runs on a worker thread and network operations on their own threads,
  so the window never freezes.

A read-only sample project is built in, so the first screen shows a realistic graph before you open anything.

## Download

A prebuilt Windows executable is attached to each [release](https://github.com/Vegaxys/Vegit/releases).
It only needs [Git](https://git-scm.com/) on your `PATH`.

## Requirements (building from source)

- [Git](https://git-scm.com/) on your `PATH`.
- [Rust](https://rustup.rs/) 1.85 or newer (the crate uses the 2024 edition).

Developed and tested on Windows; eframe also targets macOS and Linux.

## Build and run

```bash
cargo run --release
```

Open a repository directly:

```bash
cargo run --release -- path/to/repository
```

Save a screenshot of the sample project and exit (handy for checking the UI in CI):

```bash
cargo run --release -- --screenshot out.png
```

In screenshot mode, environment variables open other screens of the sample project:

| Variable | Shows |
| --- | --- |
| `VEGIT_THEME=Dracula` | A theme preset (`Midnight`, `Dracula`, `Monokai`, `Nord`, `Dusk`, `Paper`, `SolarizedLight`) |
| `VEGIT_PATTERN=Ring`, `VEGIT_GRADIENT=Radial` | A background pattern and its gradient (names as in `src/backdrop.rs`) |
| `VEGIT_PREVIEW_HOME=1` | The welcome page |
| `VEGIT_TREE=1` | File lists as a folder tree |
| `VEGIT_PREVIEW_DIFF=panel.rs`, `VEGIT_DIFF_MODE=split` | The diff of an unstaged file, in `hunk`, `inline` or `split` mode |
| `VEGIT_PREVIEW_COMMIT=1`, `VEGIT_PREVIEW_HISTORY=1` | A commit's details, a file's history |
| `VEGIT_PREVIEW_PR=1` | The Create Pull Request dialog |
| `VEGIT_PREVIEW_CONFLICT=1` | A merge stopped on a conflicted file |
| `VEGIT_PREVIEW_SETTINGS=1`, `VEGIT_PREVIEW_ACCESS=1` | The settings or the access dialog |
| `VEGIT_PREVIEW_QUICK=remote` | `remote`, `push`, `unmerged`: Add remote, Push…, delete an unmerged branch; `identity`: « Who are you? »; `rebase`: the banner with « Skip this commit »; `toast`: a toast with Undo; `undo`: named Undo / Redo; `picked`: three picked commits; `drop`: the menu after a drag and drop; `history`: Edit history; `conflict`: conflict cards; `worktree`: Open in a separate folder; `lfs`: Set up LFS for Unity |

For example (in a POSIX shell):

```bash
VEGIT_PREVIEW_HOME=1 VEGIT_THEME=Nord VEGIT_PATTERN=Line VEGIT_GRADIENT=Vertical cargo run --release -- --screenshot home.png
```

Keyboard shortcuts: `F5` refreshes, `Ctrl+Enter` commits, `Esc` closes the open diff or dialog.

## Tests

```bash
cargo test
```

The tests run the git layer against throwaway repositories in the temp folder (log, refs and status
parsing, hunk staging, graph layout) and against a local bare repository standing in for the server
(remotes, pushing and deleting tags, upstream and "gone" branches, forced delete and its restore, skipping
during a rebase, identity at the project level, merge options), and the safety net (undo of commits,
branch changes, discards with their backups, hard resets, merges into another branch and stashes; backup
purge; the reflog in words), and phase 3 (history edits without an editor, refusals, adding to an earlier
commit, the sequence-editor mode, conflict blocks and real zdiff3 conflicts with branch names, worktrees
added, listed, removed and pruned, LFS tracking, locks and sizes). One extra test lists the open pull requests of a public GitHub
repository; it needs network access, so it is ignored by default. Run it with:

```bash
cargo test -- --ignored
```

## Project layout

| File | Role |
| --- | --- |
| `src/main.rs` | Entry point and command-line arguments |
| `src/app.rs` | Application state, action queue and the git operations behind every button |
| `src/git.rs` | Thin layer over the `git` command line |
| `src/graph.rs` | Lane layout for the commit graph |
| `src/diff.rs` | Unified diff parsing into hunks |
| `src/preview.rs` | Before / after image rendering |
| `src/forge.rs` | Pull requests on GitHub, GitLab, Bitbucket and Azure DevOps |
| `src/access.rs` | Credentials and SSH keys for private repositories |
| `src/theme.rs` | Color palettes and fonts |
| `src/backdrop.rs` | Halftone background patterns and gradients |
| `src/ui*.rs` | Window, sidebar, graph and detail panels |
| `src/demo.rs` | Built-in sample repository |
| `src/icons.rs` | Draws the SVG icons, tinted with the theme colour |
| `assets/icons/`, `assets/brand/` | Icons, logo and app icon, exported as SVG from the Figma icon sheet |
| `build.rs` | Turns the app icon into the Windows `.exe` icon |
| `docs/` | The presentation video and its poster |
