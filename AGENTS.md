# Agents

## TODO.md is the shared issue list

Several agents work on this repo at once, often from separate worktrees. `TODO.md` at the
repo root is the one list of open work they share. Read it before starting a task and keep
it current.

**Edit it only on `master` in the main checkout** (the first line of `git worktree list`),
never on a feature branch, so every agent sees the same list. Commit it on its own:
`git commit TODO.md -m "TODO"`. Never sweep it into a feature commit, and never
`git add -A` there, because the main checkout often holds someone else's uncommitted work.
Pull your view of it with `git show master:TODO.md` when you are in a worktree.

### Logging an item

When you hit a gap, bug or missing feature you are not fixing right now, add it straight
away. Put it under the matching `##` section, or start a new section for a larger piece of
work. An item says:

- what is wrong or missing, with the command, file or symbol (`edit pcb cutout`,
  `crates/agentee-layout/src/place.rs`)
- how it showed up, if that is not obvious (the board, the check, the input)
- branch, worktree or commit hashes if work on it already exists

One or two lines per item. No speculation and no "consider" items: if you cannot say what
to change, say what you saw.

### Taking an item

Before you start, claim it so nobody else does: append `(taken: <branch>)` to the item or
section heading and commit that on `master`. If an item is already taken, leave it alone
unless the user says otherwise.

If you stop without finishing, remove the claim and update the item with where you got to.

### Finishing an item

Delete the item from `TODO.md` once its fix is on `master`. Delete the whole section when
its last item goes. Clean up any branch or worktree the item names.

### Rules

- Only edit or delete items you logged or took. Correcting a stale fact in someone else's
  item is fine; rewriting it is not.
- On a merge conflict in `TODO.md`, keep both sides.
- Do not move finished work to a "done" section. Git history is the record.
