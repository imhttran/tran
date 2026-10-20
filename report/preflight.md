# Phase 0 Preflight — Branch and Working-Tree Preservation

## Current branch

The plan branch `sop/plan-tran-rbac` is checked out in the worktree
`.worktrees/plan-tran-rbac`. The worktree was created from `origin/main`, so it
is isolated from the primary working tree.

## Feature branch

The feature branch name for the tran-rbac work is `feat/tran-rbac`. The branch is
created from `origin/main` in the isolated worktree, so existing working-tree
changes in the primary checkout are not touched.

## Preservation evidence

`git status --short --branch` at the start of this stage showed:

```
## sop/plan-tran-rbac
 M docs/reports/rust-migration/web-e2e/markdown-answer.png
 M docs/reports/rust-migration/web-e2e/pdf-page-answer.png
?? report/
```

These modified files are user-owned screenshot updates. They stay unchanged by
this stage. The stage adds only new files under `report/`.

## Preservation rule

The stage does not run `reset`, `clean`, `stash`, or `checkout --force`. Existing
working-tree changes in the primary checkout stay as they are.

## Related Phase 0 reports

- `report/endpoint-map.md`
- `report/genealogy-catalog-presence.md`
- `report/identity-approach.md`
- `report/baseline-tests.md`
