## The platform's boundary types: every record and union that crosses between Rust and Roc has
## a name of its own here, so the generated glue names it (and never by hash or number).
Boundary := [].{
	## One step's result: the model's box and the effects.
	Stepped(model) := { model : Box(model), effects : List(U64) }
}
