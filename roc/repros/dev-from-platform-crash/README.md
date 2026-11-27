# Roc dev backend: rift's traditional layout crashes (no standalone repro yet)

**Superseded (2026-11-27):** the standalone repro is `../recursive-nominal-payload/`. The trigger
is a nominal record (`TraditionalAxis := {…}`) as a recursive union's variant payload; the
reductions below wrote the payload as an anonymous record, which does not crash. It crashes at
`--opt=size` too.

At nightly-2026-11-26-890d6c8, rift-core-roc linked against a `--opt=dev` build of `roc/core`
SIGSEGVs in `the_roc_core_tiles_traditional_layouts_as_the_rust_core_does` and
`the_roc_core_tiles_what_the_rust_core_tiles`: `Traditional.tree_of` → `roc__rc_incref_*` →
`roc_builtins_incref_data_ptr` on 0x4088F8…, which is the `F64` 799.0 (a window's
`locked_height`) read as a pointer. `--opt=speed` is correct. Every `__DATA,__const` in that dev
archive is at 0 mod 8 and it links with a default `cc`, so this is not roc-lang/roc#13568.

## What crashes, from rift's side

- The smallest host input: a `TraditionalShape` whose root is `Axis { horizontal, fill: Tiled,
  first: Child { size: 1, node: Window(unconstrained, Tiled) }, rest: [] }`. A bare `Window`
  root does not crash; any `Axis` does (constrained or not, nested or not).
- The smallest Roc side: `roc/core/main.roc`'s `traditional` cut down to
  ```roc
  traditional = |shape| {
  	_laid = Traditional.from_platform(shape)
  	[]
  }
  ```
  still crashes (`__DATA,__const` at 0x1fb0c8): the fault is in `from_platform` / `tree_of`, before
  any geometry.

## What did not reproduce (this directory)

`run.sh dev|speed` builds `main.roc` on `platform/`, prints each `__DATA,__const` address, links
`host.c` with a default `cc` (it refuses a misaligned build: that is #13568) and runs it. `host.c`
builds the value exactly as `roc glue` (RustGlue.roc) lays it out. Each step below prints the right
answer (7) whenever `__DATA,__const` is at 0 mod 8 (pad `leaf.window.idx.to_u64()` with `* 1`,
`+ 0` and so on to move it):
1. the recursive union with a `U64` leaf;
2. rift's leaf (`WindowId`, `Constraints` → `WindowConstraints` → `Sizing`, `Fill`) and the Axis's
   fields;
3. a host record (`Shape { screen, stack_offset, root }`) and `from_platform` building a record
   with `root: tree_of(shape.root)`;
4. calling through a required `core : { … }` record of functions;
5. a parametric `Tree(s)` / `Branch(s)` with a `Stack({ label : s, … })` variant at `s = {}`;
6. the three-variant host node (`Axis`, `Stack`, `Window`), as here.

Still different from rift: its full `Boundary` and `main.roc` (100 and more types and `core`
functions), the full `TraditionalShape` (`area`, `gaps`, `stack_line`), and its `Traditional`
module (a nominal `Traditional := [].{ Laid(s), Tree(s), … }` with `Summarized` / `Annotated`
beside them).

## Next step

Bisect from rift's side: in a copy of `roc/`, with `traditional` cut down as above, remove `core`
record fields and `Boundary` types until the crash stops; keep `__DATA,__const` at 0 mod 8 at every
step.

**Gotcha:** rift-core-roc bundles the static Roc archive into its rlib. Swapping the archive under
`target/` changes nothing until rift-core-roc's lib recompiles (touch its `src/lib.rs`); without
that, a probe runs the optimized code and looks fixed. `roc/bump.sh` runs the layout tests under
dev in a target directory of their own (`target/roc-dev`, `ROC_OPT=dev`), which avoids this.
