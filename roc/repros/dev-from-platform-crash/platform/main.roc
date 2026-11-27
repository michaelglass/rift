platform ""
	requires {} { core : { count : Shape -> U64, other : U64 -> U64 } }
	exposes [Types]
	packages {}
	provides { "roc_count": count_for_host }
	targets: {
		arm64mac: { inputs: [app], output: Archive },
	}

import Types exposing [Shape]

count_for_host : Shape -> U64
count_for_host = |shape| (core.count)(shape)
