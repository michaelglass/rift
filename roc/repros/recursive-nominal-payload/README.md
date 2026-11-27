# A nominal record as a recursive union's variant payload: `--opt=size` and `--opt=dev` misread a host-built value

At nightly-2026-11-27-f79ae91 (and nightly-2026-11-26-890d6c8), macOS arm64.

`platform.roc` declares a recursive nominal union whose `Ax` variant carries a nominal record:

```roc
Leaf := { id : U64 }
Child := { size : F32, node : Node }
Axis := { horizontal : Bool, first : Child, rest : List(Child) }
Node := [Win(Leaf), Ax(Axis)]
```

`host.c` builds `Ax({ first: Child { size: 1, node: Win({ id: 7 }) }, rest: [] })` exactly as
`roc glue` (RustGlue.roc) lays it out (`Node` 48 bytes, the `Axis` inline in its payload with
`first: *mut Child` boxed, `Child.node: *mut Node` boxed, two-word box headers, tag at 40,
`Ax = 0`, `Win = 1`) and passes it to `roc_sum`, which should return 7.

```sh
./run.sh speed   # 7
./run.sh size    # Segmentation fault
./run.sh dev     # Segmentation fault
```

(`run.sh` links with `-Wl,-no_fixup_chains` so the dev archive links despite roc-lang/roc#13568;
the crash is the same without the misaligned constant.)

With `Ax({ horizontal : Bool, first : Child, rest : List(Child) })`, the same record written
inline instead of as the nominal `Axis`, all three print 7.

Under dev, the generated code reads the `Ax` payload's first word as a pointer to a 40-byte
`Axis` (it copies 5 words from it), where the glue and `--opt=speed` keep the `Axis` inline; a
host that boxes the `Axis` instead crashes at every level too, so the layout dev and size expect
is neither.

## In rift

`roc/platform/Boundary.roc` has `TraditionalAxis := {…}` and `TraditionalStack := {…}` as
`TraditionalNode`'s variant payloads. Under `--opt=size` and `--opt=dev`,
rift-core-roc's `the_roc_core_tiles_traditional_layouts_as_the_rust_core_does` and
`the_roc_core_tiles_what_the_rust_core_tiles` SIGSEGV on the first host-built `Axis` (in case 0,
the `F64` 799.0, a window's `locked_height`, increfed as a pointer; in the smallest case, the
`F32` 1.0 of a `Child.size`, increfed as a list). Every other differential passes under size,
including the traditional systems, whose trees Roc builds itself.
