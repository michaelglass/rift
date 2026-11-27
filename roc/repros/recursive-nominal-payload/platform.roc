platform ""
	requires {} { zero : U64 }
	exposes []
	packages {}
	provides { "roc_sum": sum_for_host }
	targets: {
		arm64mac: { inputs: [app], output: Archive },
	}

Leaf := { id : U64 }
Child := { size : F32, node : Node }
Axis := { horizontal : Bool, first : Child, rest : List(Child) }
Node := [Win(Leaf), Ax(Axis)]

sum : Node -> U64
sum = |node|
	match node {
		Win({ id }) => id
		Ax({ horizontal: _, first, rest }) => rest.fold(sum(first.node), |total, child| total + sum(child.node))
	}

sum_for_host : Node -> U64
sum_for_host = |node| sum(node) + zero
