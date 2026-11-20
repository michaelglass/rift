## The platform's boundary types: every record and union that crosses between Rust and Roc has
## a name of its own here, so the generated glue names it (never by hash or number; the build
## fails on either). Each mirrors the Rust core's type of the same meaning.
Boundary := [].{
	## One step's result: the model's box and the effects. Generic in the model, so the
	## platform's model variable stays out of a nominal type.
	Stepped(model) := { model : Box(model), effects : List(U64) }

	# Shared geometry (`rift_core::plain`, `rift_core::layout`).

	## A window: its app's pid and its index within the app.
	WindowId := { pid : I32, idx : U32 }

	Rect := { x : F64, y : F64, w : F64, h : F64 }

	## A size an app reported, as rift parsed it: each axis finite and not negative.
	Size := { width : F64, height : F64 }

	## How tiling may size a window, with the minimum it reports.
	Sizing := [Fixed(Size), Resizable(Size), ResizableAtLeastMin(Size)]

	WindowConstraints := { sizing : Sizing, locked_width : F64, locked_height : F64, max_width : F64, max_height : F64 }

	## A window's constraints, if known.
	Constraints := [Constrained(WindowConstraints), Unconstrained]

	Gaps := { top : F64, left : F64, bottom : F64, right : F64, horizontal : F64, vertical : F64 }

	## What a node's frame fills: the tiling area, the whole screen, or the screen within the
	## outer gaps.
	Fill := [Tiled, Screen, WithinGaps]

	## A window and the frame a layout gives it.
	Frame := { window : WindowId, frame : Rect }

	# BSP (`rift_core::layout::bsp`).

	BspWindow := { window : WindowId, constraints : Constraints }

	Slot := [Empty, Window(BspWindow)]

	BspLeaf := { slot : Slot, fill : Fill, stacked : Bool }

	## A split: `ratio` is the first child's share.
	BspSplit := { horizontal : Bool, ratio : F32, children : BspChildren }

	BspChildren := [One(BspNode), Two(BspNode, BspNode)]

	BspNode := [Split(BspSplit), Leaf(BspLeaf)]

	## The width/height a layout's only window is fitted to, if set.
	Aspect := [Aspect(F64), Free]

	BspShape := { screen : Rect, gaps : Gaps, aspect : Aspect, root : BspNode }

	# Traditional (`rift_core::layout::traditional`).

	TraditionalWindow := { window : WindowId, constraints : Constraints, fill : Fill }

	## A container's child and its weight (the share of an axis it asks for).
	Child := { size : F32, node : TraditionalNode }

	## Along an axis by weight; there is always a first.
	TraditionalAxis := { horizontal : Bool, fill : Fill, first : Child, rest : List(Child) }

	## A stack, `focused` shown on top.
	TraditionalStack := { horizontal : Bool, fill : Fill, before : List(Child), focused : Child, after : List(Child) }

	TraditionalNode := [Window(TraditionalWindow), Axis(TraditionalAxis), Stack(TraditionalStack)]

	## The bar a stack reserves room for.
	StackLine := { thickness : F64, spacing : F64, top : Bool, left : Bool }

	TraditionalShape := { screen : Rect, area : Rect, gaps : Gaps, stack_offset : F64, stack_line : StackLine, root : TraditionalNode }

	# Scrolling (`rift_core::layout::scrolling`).

	Widths := { ratio : F64, min_ratio : F64, max_ratio : F64, preserve_window_sizes : Bool, expand_single_column : Bool }

	Alignment := [Left, Center, Right]

	Navigation := [Anchored, Free]

	## A column's width: the default, a share of the view, or exactly so many points.
	Width := [Default, Proportion(F64), Exactly(F64)]

	Row := { window : WindowId, constraints : WindowConstraints, weight : F64 }

	## A column with at least one window.
	Column := { id : U64, width : Width, first : Row, rest : List(Row) }

	## A strip with at least one column.
	Strip := { tiling : Rect, column_gap : F64, row_gap : F64, widths : Widths, alignment : Alignment, navigation : Navigation, first : Column, rest : List(Column) }

	## A column in world space, and where the camera stops to show it.
	Spot := { id : U64, world_x : F64, width : F64, anchor : F64 }

	## A camera offset the camera may rest at, and the column it shows.
	Snap := { offset : F64, column : U64 }

	## Where a strip placed everything: its spots (at least one), its windows' frames, its
	## snaps (at least one), and the camera's bounds.
	Placed := { first : Spot, rest : List(Spot), frames : List(Frame), first_snap : Snap, rest_snaps : List(Snap), low : F64, high : F64 }

	## How ties go when the camera settles.
	Settle := [Niri, Anchored, AnchoredTowards(U64)]

	## Where the camera settles on a placement, seen through a view of `view` points, projected
	## to `projected`, coming from `from`.
	SettleQuestion := { placed : Placed, view : F64, projected : F64, from : F64, how : Settle }

	Settled := { column : U64, offset : F64 }

	# Floating (`rift_core::layout::floating`).

	## Where a window is seen, if it is.
	Seen := [Live(Rect), Unseen]

	Newcomer := { constraints : WindowConstraints, seen : Seen }

	## Newcomers to a floating layout on `screen`, spread or not.
	FloatingQuestion := { screen : Rect, spread : Bool, windows : List(Newcomer) }
}
