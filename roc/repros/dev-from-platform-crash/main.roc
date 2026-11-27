app [core] { pf: platform "./platform/main.roc" }

import pf.Types exposing [Node, Leaf, Fill, Shape, Rect]

Tree(s) := [Window(Leaf), Axis({ horizontal : Bool, fill : Fill, first : Branch(s), rest : List(Branch(s)) }), Stack({ label : s, focused : Branch(s) })]

Branch(s) := { size : F32, node : Tree(s) }

tree_of : Node -> Tree({})
tree_of = |node| {
	branch = |c| Branch.{ size: c.size, node: tree_of(c.node) }
	match node {
		Node.Window(leaf) => Tree.Window(leaf)
		Node.Axis({ horizontal, fill, first, rest }) => Tree.Axis({ horizontal, fill, first: branch(first), rest: rest.map(branch) })
		Node.Stack({ horizontal: _, fill: _, before: _, focused, after: _ }) => Tree.Stack({ label: {}, focused: branch(focused) })
	}
}

idx : Tree(s) -> U64
idx = |tree|
	match tree {
		Tree.Window(leaf) => leaf.window.idx.to_u64()
		Tree.Axis({ horizontal: _, fill: _, first, rest }) => rest.fold(idx(first.node), |acc, b| acc + idx(b.node))
		Tree.Stack({ label: _, focused }) => idx(focused.node)
	}

Laid(s) := { screen : Rect, stack_offset : F64, root : Tree(s) }

from_platform : Shape -> Laid({})
from_platform = |shape| Laid.{ screen: shape.screen, stack_offset: shape.stack_offset, root: tree_of(shape.root) }

count : Shape -> U64
count = |shape| idx(from_platform(shape).root)

core = { count, other: |n| n + 1 }
