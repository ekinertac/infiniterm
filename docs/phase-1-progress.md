<!-- Phase 1 coverage ledger for the next port session. Read with HANDOVER.md and the reference source/tests. This records implementation progress, not full app parity. -->
# Phase 1 progress

Phase 1 is in progress. The workspace passes 281 tests: 273 ported reference cases and 8 additional native checks. The handover baseline is 498 reference cases, so 225 remain. Domain command registration also remains; the handover does not assign a test count to those files.

Paused at the user's request on 2026-09-15 to conserve the five-hour usage allowance. All implemented work is at a passing checkpoint. Resume at `paletteUsage.ts`; its implementation has not started.

The reference is the live `~/Code/infiniterm` worktree. Its source code wins over comments and planning documents. The port did not change that worktree or the spikes.

## Coverage

Reference test paths are relative to `~/Code/infiniterm/src/lib/`. Rust module paths are relative to `infiniterm-core/src/` unless a crate path is shown. Each reference case has a corresponding Rust test; combined source files retain their separate cases.

| Reference tests | Rust module | Reference cases | Native checks |
|---|---|---:|---:|
| `grid.test.ts` | `grid.rs` | 16 | 1 |
| `layout.test.ts` | `layout.rs` | 15 | 0 |
| `resize.test.ts + cardActions.test.ts` | `resize.rs` | 16 | 0 |
| `cardSize.test.ts` | `cards.rs` | 6 | 0 |
| `navigate.test.ts` | `navigate.rs` | 17 | 0 |
| `slots.test.ts` | `slots.rs` | 6 | 0 |
| `split.test.ts` | `split.rs` | 21 | 0 |
| `swap.test.ts` | `swap.rs` | 14 | 0 |
| `multiSelect.test.ts` | `multi_select.rs` | 4 | 0 |
| `groups.test.ts` | `groups.rs` | 20 | 0 |
| `workspaces.test.ts` | `workspaces.rs` | 11 | 0 |
| `zoomActions.test.ts + zoomAnimation.test.ts` | `viewport.rs` | 20 | 3 |
| `momentum.test.ts` | `momentum.rs` | 10 | 0 |
| `panMode.test.ts` | `pan_mode.rs` | 4 | 0 |
| `chrome.test.ts` | `chrome.rs` | 15 | 0 |
| `formatZoom.test.ts` | `format_zoom.rs` | 3 | 0 |
| `agentState.test.ts` | `agent_state.rs` | 7 | 0 |
| `fuzzy.test.ts` | `fuzzy.rs` | 12 | 1 |
| `palette.test.ts` | `palette.rs` | 22 | 1 |
| `sidebar.test.ts` | `sidebar.rs` | 6 | 0 |
| `blame.test.ts` | `blame.rs` | 4 | 0 |
| `labelColors.test.ts` | `label_colors.rs` | 11 | 0 |
| `outputScheduler.test.ts` | `infiniterm-term/src/scheduler.rs` | 9 | 2 |
| `flowControl.test.ts` | `infiniterm-term/src/credit.rs` | 4 | 0 |
| **Total** | | **273** | **8** |

Native checks cover negative half-cell rounding, fit padding, animation durations, coordinate conversion, Unicode matching/highlighting, scheduler arrival order, and empty chunks.

## Port decisions

Pure geometry uses `f64`, matching TypeScript numbers. `grid.rs` owns `Rect`, `Point`, and `Size`; `viewport.rs` re-exports them for existing callers. UI code must convert to rendering coordinates at the boundary.

`cards::PlacedCard` carries only an id, rect, and group membership. `cards::CardRect` carries a returned geometry update. Live card state and card bodies remain outside these algorithms. Callers must supply cards and occupied frames from one workspace.

Fuzzy match positions use UTF-16 offsets, as in the reference. `palette::highlight` resolves those offsets before it returns Rust strings. Usage bonuses are callback inputs; saved usage history is not yet ported.

The scheduler moves reader buffers into shared storage. Pieces refer to ranges of the same allocation, so splitting across frames does not copy the remaining bytes. Ordered queue storage preserves JavaScript Map arrival order. Parsing and engine integration remain later work.

## Next work

1. Port `paletteUsage.ts` and its tests. Preserve entry order and keep recency separate from frequency.
2. Port save/config parsers, migrations, JSONC editing, theme parsing, and their tests. Use the reference's validation rules.
3. Complete the Turkish-Q physical-key check before keymap work. Then port keymap, shortcut, command, editor-key, and browser-key modules.
4. Port card labels, path plans/links, transcript formatting, and editor theme helpers with their tests.
5. Port domain command registration and behavior. Update the case count against the 498-case baseline before marking Phase 1 complete.

## Validation

`cargo test --offline` passes all 281 tests. `cargo fmt --all --check` and `cargo clippy --workspace --all-targets --offline -- -D warnings` pass. The workspace still has no external dependencies.

No app window exists yet, so this checkpoint has no UI acceptance result. Screen checks remain required when the UI is available.
