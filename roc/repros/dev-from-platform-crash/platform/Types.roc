Types := [].{
	WindowId := { pid : I32, idx : U32 }
	Size := { width : F64, height : F64 }
	Sizing := [Fixed(Size), Resizable(Size), ResizableAtLeastMin(Size)]
	WindowConstraints := { sizing : Sizing, locked_width : F64, locked_height : F64, max_width : F64, max_height : F64 }
	Constraints := [Constrained(WindowConstraints), Unconstrained]
	Fill := [Tiled, Screen, WithinGaps]
	Leaf := { window : WindowId, constraints : Constraints, fill : Fill }
	Node := [Window(Leaf), Axis({ horizontal : Bool, fill : Fill, first : Child, rest : List(Child) }), Stack({ horizontal : Bool, fill : Fill, before : List(Child), focused : Child, after : List(Child) })]
	Child := { size : F32, node : Node }
	Rect := { x : F64, y : F64, w : F64, h : F64 }
	Shape := { screen : Rect, stack_offset : F64, root : Node }
}
