// REQ-0124: shared source-tree walker that honours .gitignore.
//
// Wraps `ignore::WalkBuilder` (ripgrep's walker), so the same
// directories git considers untracked are skipped by req coverage,
// req lint, and req review's marker scans. Previously each walker
// hand-rolled `fs::read_dir` + a hard-coded SKIP_DIRS list, which
// missed project-specific ignore patterns (tmp/, build outputs)
// and produced ghost references in coverage reports.
//
// The visitor receives (path, extension-without-dot) pairs filtered
// by the caller's `exts` list. Files outside the ext list, hidden
// files, and gitignored paths are skipped.
use std::path::Path;

/// Walk `root` honouring .gitignore, .ignore, and global git excludes.
/// Calls `visit` once per regular file whose extension matches `exts`
/// (case-sensitive; pass lowercase). When .gitignore parsing fails for
/// any reason, falls back to walking everything (better to over-report
/// than to silently skip).
pub fn walk_source_tree<F>(root: &Path, exts: &[String], mut visit: F)
where
    F: FnMut(&Path),
{
    let mut builder = ignore::WalkBuilder::new(root);
    builder
        .hidden(true) // skip dotfiles by default
        .git_ignore(true) // honour .gitignore
        .git_exclude(true) // honour .git/info/exclude
        .git_global(true) // honour ~/.gitignore_global
        .require_git(false) // honour .ignore even outside a git tree
        .parents(true); // walk parent .gitignore files too
    for entry in builder.build().flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let ext = match path.extension().and_then(|e| e.to_str()) {
            Some(e) => e,
            None => continue,
        };
        if !exts.iter().any(|x| x == ext) {
            continue;
        }
        visit(path);
    }
}
