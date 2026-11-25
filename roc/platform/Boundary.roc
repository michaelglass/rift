## The platform's boundary types: every record and union that crosses between Rust and Roc has
## a name of its own here, so the generated glue names it (never by hash or number; the build
## fails on either). Each mirrors the Rust core's type of the same meaning.
Boundary := [].{

	# Shared geometry (`rift_core::plain`, `rift_core::layout`).

	## A window: its app's pid and its index within the app. Equal when both are.
	WindowId := { pid : I32, idx : U32 }.{
		is_eq : WindowId, WindowId -> Bool
		is_eq = |a, b| a.pid == b.pid and a.idx == b.idx

		## Hashed as its pid, then its index (a key of the core's windows).
		to_hash : WindowId, Hasher -> Hasher
		to_hash = |id, hasher| id.idx.to_hash(id.pid.to_hash(hasher))
	}

	## A frame.
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

	# A scrolling strip on screen (`rift_core::layout::scrolling::translate`).

	FullscreenMode := [Screen, WithinGaps]

	FullscreenWindow := { window : WindowId, mode : FullscreenMode }

	Parking := [Park, InPlace]

	## A placed strip's world frames shown by a camera: its screen and tiling area, its offset, and
	## whether windows wholly out of view are parked; the fullscreen windows.
	TranslateQuestion := { frames : List(Frame), screen : Rect, tiling : Rect, offset : F64, parking : Parking, fullscreen : List(FullscreenWindow) }

	# One axis (`rift_core::layout::axis`).

	## A segment's fixed length, if it has one.
	AxisFixed := [Fixed(F64), Free]

	## A segment's maximum, if it has one.
	AxisMax := [Max(F64), NoMax]

	## One segment to solve, and what it is the length of.
	AxisItem := { min : F64, fixed : AxisFixed, max : AxisMax, weight : F64, can_grow : Bool }

	## Segments along one axis and the length they share (`solve_axis_lengths`).
	AxisQuestion := { items : List(AxisItem), usable : F64 }

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

	## A window in a column: its constraints as reported, its share of the column's height.
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

	## A window placed for the first time: its constraints, and the frame it is seen at, if seen.
	Newcomer := { constraints : WindowConstraints, seen : Seen }

	## Newcomers to a floating layout on `screen`, spread or not.
	FloatingQuestion := { screen : Rect, spread : Bool, windows : List(Newcomer) }

	## Where a window of a floating layout goes: the screen, the tiling area (fullscreen within
	## the gaps), the frame the layout keeps for it, or its first frame.
	FloatingPlace := [Screen, WithinGaps, Kept(Rect), First(WindowConstraints)]

	FloatingWindow := { window : WindowId, place : FloatingPlace }

	## One of a floating layout's windows: left out (it holds its position in a cascade), or placed.
	FloatingSlot := [LeftOut, Placed(FloatingWindow)]

	## A floating layout's windows in its order, on `screen`, spreading first frames or not.
	FloatingLayout := { screen : Rect, spread : Bool, gaps : Gaps, windows : List(FloatingSlot) }

	# Model-free questions, shared shapes. A fact is what the platform's run of the decision
	# answered, or that it never asked it (`rift_core::plain::Fact`), one named type per
	# payload; a fact asked about keys is a list of key and answer (`Answers`). An answer is
	# the decision, or the first fact it read that its question does not carry.

	## A word (a Space, a workspace, a window-server id), or none.
	MaybeWord := [Some(U64), Nothing].{
		## The same word, or both none.
		is_eq : MaybeWord, MaybeWord -> Bool
		is_eq = |a, b|
			match (a, b) {
				(Some(x), Some(y)) => x == y
				(Nothing, Nothing) => True
				(Some(_), Nothing) | (Nothing, Some(_)) => False
			}
	}

	## A window, or none.
	MaybeWindow := [SomeWindow(WindowId), NoWindow]

	FactBool := [Asked(Bool), Unasked].{
		## The answer, if the platform asked.
		answer : FactBool -> Try(Bool, [Unasked])
		answer = |fact|
			match fact {
				Asked(given) => Ok(given)
				Unasked => Err(Unasked)
			}
	}

	## A fact of a word or none.
	FactWord := [Asked(MaybeWord), Unasked].{
		## The answer, if the platform asked.
		answer : FactWord -> Try(MaybeWord, [Unasked])
		answer = |fact|
			match fact {
				Asked(given) => Ok(given)
				Unasked => Err(Unasked)
			}
	}

	## A fact of a window or none.
	FactWindow := [Asked(MaybeWindow), Unasked].{
		## The answer, if the platform asked.
		answer : FactWindow -> Try(MaybeWindow, [Unasked])
		answer = |fact|
			match fact {
				Asked(given) => Ok(given)
				Unasked => Err(Unasked)
			}
	}

	## A fact of a list of words.
	FactWords := [Asked(List(U64)), Unasked].{
		## The answer, if the platform asked.
		answer : FactWords -> Try(List(U64), [Unasked])
		answer = |fact|
			match fact {
				Asked(given) => Ok(given)
				Unasked => Err(Unasked)
			}
	}

	## A yes or no asked about a word.
	WordBool := { key : U64, answer : Bool }.{
		## The key asked and its answer.
		entry : WordBool -> { key : U64, answer : Bool }
		entry = |given| { key: given.key, answer: given.answer }
	}

	## A yes or no asked about a window.
	WindowBool := { key : WindowId, answer : Bool }.{
		## The key asked and its answer.
		entry : WindowBool -> { key : WindowId, answer : Bool }
		entry = |given| { key: given.key, answer: given.answer }
	}

	## A window asked about a window.
	WindowWindow := { key : WindowId, answer : WindowId }.{
		## The key asked and its answer.
		entry : WindowWindow -> { key : WindowId, answer : WindowId }
		entry = |given| { key: given.key, answer: given.answer }
	}

	## A word or none, decided.
	WordAnswer := [Decided(MaybeWord), Unasked(U64)]

	## A yes or no, decided.
	VerdictAnswer := [Decided(Bool), Unasked(U64)]

	# Focus (`rift_core::focus`).

	Removal := [AppClosed(I32), WindowRemoved(WindowId)]

	RemovalQuestion := { removal : Removal, focused : MaybeWindow, removed_space : FactWord, native_focus : FactWindow, successors : List(WindowWindow), inactive : List(WindowBool) }

	RemovalFocus := [Unaffected, KeepNative(WindowId), Refocus(U64)]

	RemovalAnswer := [Decided(RemovalFocus), Unasked(U64)]

	CandidateQuestion := { preferred : MaybeWord, last_focused : FactWord, in_workspace : FactWords, visible : List(WordBool) }

	HidesQuestion := { window : U64, main : MaybeWord, focused : MaybeWord, in_inactive_workspace : FactBool }

	## Whether an app is blacklisted for auto-focus; no bundle id, no listing.
	Listing := [Listed(Bool), NoBundle]

	FactListing := [Asked(Listing), Unasked].{
		## The answer, if the platform asked.
		answer : FactListing -> Try(Listing, [Unasked])
		answer = |fact|
			match fact {
				Asked(given) => Ok(given)
				Unasked => Err(Unasked)
			}
	}

	ActivationQuestion := { standard : FactBool, unavailable : FactBool, space : FactWord, workspace : FactWord, active_workspace : FactWord, pending_window : FactBool, mode_refusal : FactWord, active_space_fullscreen : FactBool, unmanageable_source : FactWord, blacklisted : FactListing }

	FollowTo := { space : U64, workspace : U64 }

	## Why an activation does not follow its window.
	Stay := [NotStandard, Unavailable, NoSpace, NoWorkspace, AlreadyActive, PendingWindow, Mode(U64), FullscreenActive, Unmanageable(U64), NoBundleId, Blacklisted]

	## Whether an activation follows its window, and where, or why not.
	Follow := [To(FollowTo), Stay(Stay)]

	FollowAnswer := [Decided(Follow), Unasked(U64)]

	DisplayFollowQuestion := { focused_space : MaybeWord, previous : MaybeWord, active : List(WordBool), on_known_display : List(WordBool) }

	## What is known of a window: its layer and its app.
	KnownWindow := { layer : I32, owner : U64 }

	Known := [Known(KnownWindow), Unknown]

	FactKnown := [Asked(Known), Unasked].{
		## The answer, if the platform asked.
		answer : FactKnown -> Try(Known, [Unasked])
		answer = |fact|
			match fact {
				Asked(given) => Ok(given)
				Unasked => Err(Unasked)
			}
	}

	UntrackedQuestion := { under_cursor : MaybeWord, tracked : FactBool, known : FactKnown }

	Surface := { layer : I32, window : U64 }

	KeyWindow := { owner : U64, window : U64 }

	UntrackedFocus := [Nothing, SystemSurface(Surface), MakeKey(KeyWindow)]

	UntrackedAnswer := [Decided(UntrackedFocus), Unasked(U64)]

	DesktopQuestion := { active : Bool, workspace_empty : FactBool }

	## The window under the pointer: its frame and its window-server id, if it has one.
	HoveredWindow := { frame : Rect, server : MaybeWord }

	Pointed := [Hovered(HoveredWindow), Untracked]

	## A floating window's frame, if the window is one.
	MaybeFrame := [Frame(Rect), NoFrame]

	WordFrame := { key : U64, answer : MaybeFrame }.{
		## The key asked and its answer.
		entry : WordFrame -> { key : U64, answer : MaybeFrame }
		entry = |given| { key: given.key, answer: given.answer }
	}

	WindowLevels := { level : I64, sub_level : I32 }

	## A window's levels, if known.
	MaybeLevels := [Levels(WindowLevels), NoLevels]

	WordLevels := { key : U64, answer : MaybeLevels }.{
		## The key asked and its answer.
		entry : WordLevels -> { key : U64, answer : MaybeLevels }
		entry = |given| { key: given.key, answer: given.answer }
	}

	RaiseQuestion := { window : Pointed, space : MaybeWord, claimed : FactBool, admitted : FactBool, floating : FactBool, active : FactBool, in_active_workspace : FactBool, could_occlude : FactBool, order : FactWords, floaters : List(WordFrame), levels : List(WordLevels) }

	## Why the window under the pointer is not raised.
	Unraised := [Unknown, OutsideAdmission, NoSpace, InactiveSpace, OutsideActiveWorkspace, BuriesFloater]

	## Whether focus-follows-mouse raises the window under the pointer, or why not.
	MouseRaise := [Raise, Skip(Unraised)]

	RaiseAnswer := [Decided(MouseRaise), Unasked(U64)]

	# Topology (`rift_core::topology`).

	## A Space, or none.
	MaybeSpace := [Space(U64), NoSpace]

	## What an event is, to an unsettled topology's quarantine.
	EventClass := [WindowReport, WindowInput, Other]

	FactEventClass := [Asked(EventClass), Unasked].{
		## The answer, if the platform asked.
		answer : FactEventClass -> Try(EventClass, [Unasked])
		answer = |fact|
			match fact {
				Asked(given) => Ok(given)
				Unasked => Err(Unasked)
			}
	}

	## Whether to quarantine an event: whether the topology is settled, the event's class.
	QuarantineQuestion := { topology : TopologyNow, class : FactEventClass }

	## A point on the screens.
	Point := { x : F64, y : F64 }

	## Where a display selector is looked up from, if anywhere.
	LookupOrigin := [At(Point), Nowhere]

	FactOrigin := [Asked(LookupOrigin), Unasked].{
		## The answer, if the platform asked.
		answer : FactOrigin -> Try(LookupOrigin, [Unasked])
		answer = |fact|
			match fact {
				Asked(given) => Ok(given)
				Unasked => Err(Unasked)
			}
	}

	## A display a selector found (one word) and its Space.
	FoundScreen := { display : U64, space : MaybeWord }

	ScreenFound := [Found(FoundScreen), NotFound]

	OriginScreen := { key : LookupOrigin, answer : ScreenFound }.{
		## The key asked and its answer.
		entry : OriginScreen -> { key : LookupOrigin, answer : ScreenFound }
		entry = |given| { key: given.key, answer: given.answer }
	}

	TargetQuestion := { source_center : LookupOrigin, current_center : FactOrigin, screens : List(OriginScreen), active : List(WordBool) }

	## A display and its active Space.
	TargetSpace := { display : U64, space : U64 }

	## The display a selector names, or why there is none.
	Target := [Display(TargetSpace), DisplayNotFound, SpaceUnavailable]

	TargetAnswer := [Decided(Target), Unasked(U64)]

	FactSpace := [Asked(MaybeSpace), Unasked].{
		## The answer, if the platform asked.
		answer : FactSpace -> Try(MaybeSpace, [Unasked])
		answer = |fact|
			match fact {
				Asked(given) => Ok(given)
				Unasked => Err(Unasked)
			}
	}

	## Where an app inventory finds a window, by what the platform already knows of it.
	Standing := [RecoveringFullscreen, Untracked, Tracked]

	DiscoveredQuestion := { standing : Standing, live : FactWords, observable : List(WordBool), recorded : FactSpace, resolved : FactSpace }

	## The Space an app inventory takes a window to be on.
	DiscoveredSpace := [Observed(U64), Live(U64), Fallback(MaybeSpace)]

	DiscoveredAnswer := [Decided(DiscoveredSpace), Unasked(U64)]

	## What an app inventory may do with a window it reports, for its discovery Space.
	DiscoveryStanding := [FullscreenRecorded, Placeholder, Admissible]

	DiscoveryQuestion := { standing : DiscoveryStanding, native : MaybeSpace, assigned : MaybeSpace, by_frame : FactSpace }

	## A reported window's authoritative Space and the Space discovery may admit it on.
	DiscoverySpaces := { authoritative : MaybeSpace, discovery : MaybeSpace }

	DiscoveryAnswer := [Decided(DiscoverySpaces), Unasked(U64)]

	## A screen's display, or none for a display with no id (each display a word).
	Screen := [Display(U64), Unnamed].{
		## The same display, or both unnamed.
		is_eq : Screen, Screen -> Bool
		is_eq = |a, b|
			match (a, b) {
				(Display(x), Display(y)) => x == y
				(Unnamed, Unnamed) => True
				(Display(_), Unnamed) | (Unnamed, Display(_)) => False
			}
	}

	DisplaySpaces := { display : U64, spaces : List(U64) }

	## The displays of a snapshot: each screen's display (none without an id), each display's
	## Spaces in the platform's order.
	Displays := { screens : List(Screen), spaces : List(DisplaySpaces) }

	## A Space macOS renumbered.
	Remap := { old : U64, new : U64 }

	## A snapshot being admitted.
	Incoming := { displays : Displays, remaps : List(Remap), display_set_changed : Bool, force_refresh : Bool, membership_complete : Bool }

	MissionControl := [Open, Closed]

	AdmitQuestion := { mission_control : MissionControl, current : Displays, incoming : Incoming }

	## Why a snapshot waits.
	Hold := [MissionControl, MembershipIncomplete]

	Commit := { invalidated : List(U64), lost : List(U64) }

	Admission := [Hold(Hold), Commit(Commit)]

	ServerSpace := { server : U64, space : U64 }

	## The window a window-server id is tracked as, if any.
	TrackedWindow := [Tracked(WindowId), NotTracked]

	WordTracked := { key : U64, answer : TrackedWindow }.{
		## The key asked and its answer.
		entry : WordTracked -> { key : U64, answer : TrackedWindow }
		entry = |given| { key: given.key, answer: given.answer }
	}

	CandidatesQuestion := { window_spaces : List(ServerSpace), activated : List(U64), tracked : List(WordTracked), reconcilable : List(WindowBool) }

	## An app's windows.
	AppWindows := { pid : I32, windows : List(WindowId) }

	CandidatesAnswer := [Decided(List(AppWindows)), Unasked(U64)]

	# Lifecycle (`rift_core::lifecycle`).

	## Why app rules run over some windows.
	RuleRun := [Admission, Reapply]

	## A Space or none asked about a word.
	WordSpace := { key : U64, answer : MaybeSpace }.{
		## The key asked and its answer.
		entry : WordSpace -> { key : U64, answer : MaybeSpace }
		entry = |given| { key: given.key, answer: given.answer }
	}

	RuleTargetsQuestion := { windows : List(U64), run : RuleRun, reconcilable : List(WordBool), spaces : List(WordSpace), active : List(WordBool) }

	## A Space's windows.
	SpaceWindows := { space : U64, windows : List(U64) }

	RuleTargetsAnswer := [Decided(List(SpaceWindows)), Unasked(U64)]

	RuleOutcome := { workspace_before : MaybeSpace, workspace : U64, floating_before : Bool, floating : Bool, was_ignored : Bool, unavailable : Bool }

	RuleRefreshQuestion := { run : RuleRun, outcome : RuleOutcome }

	WorkspaceMode := [PerDisplay, Global]

	## A window that just opened, as placement sees it.
	NewWindow := [Untracked, NotStandard, Standard]

	PointerTargetQuestion := { mode : WorkspaceMode, opened_on : U64, pointer : FactSpace, active : List(WordBool), window : NewWindow, rule_names_workspace : FactBool }

	## A Space or none, decided.
	SpaceAnswer := [Decided(MaybeSpace), Unasked(U64)]

	## Where a transient entry's evidence came from.
	Origin := [Native, Heuristic]

	EntryOf := { origin : Origin, pid : U64 }

	TransientEntry := [Entry(EntryOf), NoEntry]

	NativeTransientQuestion := { entry : TransientEntry, owner : U64, exempt : FactBool }

	## A window lifecycle event, as the transient entries hear of it.
	Departure := [Destroyed(MaybeSpace), Closed(U64), AppGone(U64)]

	KeepsTransientQuestion := { departure : Departure, id : U64, origin : Origin, owner : U64 }

	MisplacedQuestion := { assigned : MaybeSpace, native : MaybeSpace }

	OutOfViewQuestion := { native : MaybeSpace, assigned : FactSpace, active : List(WordBool), parked : FactBool }

	OnTitleChange := [Reapply, Ignore]

	TitleChangeQuestion := { configured : OnTitleChange, space : FactSpace, active : List(WordBool), reconcilable : FactBool }

	Manageability := [Manageable, Unmanageable]

	## What an app rule says about managing the window.
	Rule := [NoRule, Manage, Ignore]

	AxKind := [Ordinary, DialogWindow, Transient]

	NativeKind := [Ordinary, Transient]

	Minimize := [Shown, Minimized]

	## What admission reads of a window.
	AdmissionFacts := { manageability : Manageability, rule : Rule, ax : AxKind, native : NativeKind, minimize : Minimize }

	## Why a window is not admitted.
	Veto := [NativeTransient, Minimized, AxTransient, RuleIgnores, Unmanageable]

	WindowAdmission := [Admitted, Vetoed(Veto)]

	## One admission fact, changed.
	FactChange := [Manageability(Manageability), Rule(Rule), Ax(AxKind), Native(NativeKind), Minimize(Minimize)]

	## A reading of a window: its facts afresh, one fact changed, or its AX state gone.
	Reading := [Live(AdmissionFacts), Changed(FactChange), Gone]

	## A window's facts, or none without AX state.
	KnownFacts := [Known(AdmissionFacts), NoFacts]

	ReadQuestion := { before : KnownFacts, reading : Reading }

	## A change in whether rift manages a window.
	Change := [NoChange, Admitted, Suspended(Veto)]

	ReadAnswer := { after : KnownFacts, change : Change }

	## A window's presence: none, live, or kept with its AX element invalidated.
	Presence := [NoPresence, Live(AdmissionFacts), Invalidated(AdmissionFacts)]

	PresenceEvent := [Reading(Reading), Invalidate, Revalidate]

	PresenceQuestion := { before : Presence, event : PresenceEvent }

	PresenceOutcome := [Read(Change), Invalidated(Bool), Revalidated(Bool)]

	PresenceAnswer := { after : Presence, outcome : PresenceOutcome, manages : Bool }

	## Two admission changes in a row.
	NetQuestion := { first : Change, then : Change }

	## The mode table's question: an initiative, and the modes Rift is in.
	InputClass := [Steering, Arranging, Inert, Hover]

	## Things Rift would do that a mode can refuse.
	Initiative := [UserInput(InputClass), Arrange, RetireWindows, SwitchWorkspaceFocus, WarpFocus, FollowActivation]

	## What the pointer is doing with windows.
	PointerNow := [Idle, Pressing(U64), Dragging(U64)]

	MenuNow := [Closed, Open]

	MissionControlNow := [Inactive, Active]

	ModalNow := [Clear, Frontmost]

	ViewportNow := [Free, Gesturing]

	FocusClaimNow := [Free, PausesHover]

	UserSpacesNow := [Shown, None]

	WakeNow := [UserPresent, AwaitingInput]

	WorkspaceSwitchNow := [Idle, Switching]

	TopologyNow := [Settled, Unsettled]

	ModesQuestion := { initiative : Initiative, pointer : PointerNow, menu : MenuNow, mission_control : MissionControlNow, modal : ModalNow, viewport : ViewportNow, focus_claim : FocusClaimNow, user_spaces : UserSpacesNow, wake : WakeNow, workspace_switch : WorkspaceSwitchNow, topology : TopologyNow }

	## The axis that refused.
	RefusingAxis := [Pointer(PointerNow), Menu, MissionControl, Modal, Viewport, FocusClaim, UserSpaces, Wake, WorkspaceSwitch, Topology]

	## Admitted, or the first axis that refuses.
	ModeVerdict := [Admitted, Refused(RefusingAxis)]

	## The lifecycle kept between events: a reading of one window's facts.
	LifecycleEvent := { window : U64, reading : Reading }

	## A window with AX state: valid, or invalidated.
	Present := [Live(AdmissionFacts), Invalidated(AdmissionFacts)]

	## A window with AX state and its presence.
	PresentWindow := { window : U64, presence : Present }


	## Whether an invalidation found AX state to invalidate.
	Invalidation := [Invalidated, NoAxState]

	## Whether a revalidation brought an invalidated window back (a valid window, or one
	## without AX state, was not invalidated).
	Revalidation := [Revalidated, NotInvalidated]




	Report := [Reported, Omitted]

	InventoryView := [InView, OutOfView]

	## A window's minimize state, or none without AX state.
	InventoryMinimize := [Shown, Minimized, NoAxState]

	FullscreenRecord := [Recorded, NotRecorded]

	NativeIdentity := [Known(U64), Unknown]

	AxValidity := [Valid, Invalidated]

	## What an inventory knows of a window it is reconciled against.
	InventoryFacts := { report : Report, view : InventoryView, minimize : InventoryMinimize, fullscreen : FullscreenRecord, identity : NativeIdentity, validity : AxValidity }

	## The window server's type for a window, if it says.
	Suitability := [Suitable, Unsuitable, SuitabilityUnknown]

	Ordering := [OrderedIn, OrderedOut, OrderingUnknown]

	## Whether the window server's record looks like a window, if it says.
	Look := [AWindow, NotAWindow, LookUnknown]

	## What the window server says of an omitted window.
	NativeEvidence := { suitability : Suitability, ordering : Ordering, look : Look }

	FactEvidence := [Asked(NativeEvidence), Unasked].{
		## The answer, if the platform asked.
		answer : FactEvidence -> Try(NativeEvidence, [Unasked])
		answer = |fact|
			match fact {
				Asked(given) => Ok(given)
				Unasked => Err(Unasked)
			}
	}

	InventoryQuestion := { facts : InventoryFacts, evidence : FactEvidence }

	## Why an inventory leaves a window alone.
	Keep := [Reported, OutOfView, NoAxState, Minimized, Fullscreen, NoNativeIdentity, NativeOrdering, NoNegativeEvidence]

	Retirement := [Retire, Keep(Keep)]

	RetirementAnswer := [Decided(Retirement), Unasked(U64)]

	# Workspaces (`rift_core::workspaces`).

	## Where a workspace command is given: the acting Space, and the Spaces visible now.
	Context := { space : U64, visible : List(U64) }

	CatalogEntry := { id : U64, space : U64 }

	## Which way stepping goes.
	Way := [Next, Previous]

	SkipEmpty := [Skip, Keep]

	Wrapping := [Wrap, Stop]

	## Fact: 0 whether a workspace has windows (asked only when skipping empty ones).
	StepQuestion := { mode : WorkspaceMode, catalog : List(CatalogEntry), context : Context, current : U64, way : Way, skip_empty : SkipEmpty, wrapping : Wrapping, has_windows : List(WordBool) }

	ScopeEntry := { id : U64, space : U64, name : U64 }

	## How a command names a workspace: its place in the scope, or its name.
	Selector := [Index(U64), Name(U64)]

	ResolveQuestion := { mode : WorkspaceMode, scope : List(ScopeEntry), context : Context, selector : Selector }

	## A workspace and its Space.
	WorkspaceSpace := { workspace : U64, space : U64 }

	## The workspace a command reaches and its Space, if any.
	Reached := [Reached(WorkspaceSpace), Unreached]

	## Fact: 0 the Space the history's workspace is on (asked only with a history).
	PreviousQuestion := { mode : WorkspaceMode, context : Context, last : MaybeWord, on : FactWord }

	ReachedAnswer := [Decided(Reached), Unasked(U64)]

	EligibleQuestion := { mode : WorkspaceMode, context : Context, on : U64 }

	## Whether discovery emits a window: its workspace, and the Space's active one (asked).
	EmitsQuestion := { assigned : MaybeWord, active : FactWord }

	Showing := { current : U64, previous : MaybeWord, remembered : MaybeWord }

	## What a native Space shows.
	Activation := [Showing(Showing), Vacant(MaybeWord)]

	SpaceActivation := { space : U64, activation : Activation }

	# The main window (`rift_core::main_window`). Pids and windows are one word each.

	## Whether an app is frontmost by its own AX activation events.
	AppStanding := [Frontmost, Background]

	## Who started an activation: Rift itself, or anything else.
	Initiator := [Rift, Other]

	MainLaunch := { pid : U64, standing : AppStanding, main_window : MaybeWord }

	MainActivation := { pid : U64, initiator : Initiator }

	MainWindowChange := { pid : U64, window : MaybeWord, initiator : Initiator }

	## A window and its app.
	FocusedWindow := { pid : U64, window : U64 }

	MainReplacement := { pid : U64, window : U64, replacement : MaybeWord }

	MainWindowEvent := [
		Launched(MainLaunch),
		Terminated(U64),
		Activated(MainActivation),
		Deactivated(U64),
		GloballyActivated(U64),
		GloballyDeactivated(U64),
		MainWindowChanged(MainWindowChange),
		NativeFocused(FocusedWindow),
		Replaced(MainReplacement),
	]

	## The window the window server last reported focused, while it exists.
	ReportedFocus := [NoWindow, Window(FocusedWindow)]

	## What the window server has said about focus.
	NativeFocus := [NotYetReported, Reported(ReportedFocus)]

	## One app as the machine knows it.
	MainApp := { pid : U64, standing : AppStanding, last_activation : Initiator, main_window : MaybeWord }

	# A BSP layout system's trees (`rift_core::layout::bsp_tree::Tree`), kept between operations.
	# Layouts and nodes are slotmap keys, one word each (`version << 32 | index`, slotmap's ffi
	# form); windows are one word each.

	## Which way a split divides its area.
	BspOrientation := [Horizontal, Vertical]

	BspDirection := [Left, Right, Up, Down]

	## One of a split's two children.
	BspSide := [First, Second]

	## Where the selection goes when it reaches a split: it stops there, or goes on into a side.
	BspMemory := [Stops, Toward(BspSide)]

	## A direction the next window put into a leaf splits it toward, if one was chosen.
	BspPreselect := [Unset, Toward(BspDirection)]

	## A stack's windows in order, with the one shown on top.
	BspTabs := { before : List(U64), top : U64, after : List(U64) }

	## What a leaf holding windows holds: one window, or a stack.
	BspMembers := [Window(U64), Stack(BspTabs)]

	BspHeld := { members : BspMembers, fill : Fill }

	## What a leaf holds: nothing (a placeholder), or windows filling their place one way.
	BspContent := [Empty, Held(BspHeld)]

	BspTreeLeaf := { content : BspContent, preselect : BspPreselect }

	## A split: the first child's share, its children's keys, and where the selection goes.
	BspTreeSplit := { orientation : BspOrientation, ratio : F32, first : U64, second : U64, memory : BspMemory }

	BspKind := [Split(BspTreeSplit), Leaf(BspTreeLeaf)]

	## A node's place: a layout's root (or a node not placed yet), or a split's child.
	BspParent := [Root, Split(U64)]

	BspTreeNode := { parent : BspParent, kind : BspKind }

	## Where a new window goes when no direction was preselected.
	BspInsertion := [NextToSelection, EndOfTree]

	## Whether a window takes part in layout.
	BspTakesPart := [TakesPart, LeftOut]

	BspParticipation := { window : U64, participation : BspTakesPart }

	## One layout slot: a layout's root under an odd version, or vacant under an even one.
	BspLayoutHeld := { version : U32, root : U64 }

	BspLayoutSlot := [Occupied(BspLayoutHeld), Vacant(U32)]

	## One node slot: a node under an odd version, or vacant under an even one.
	BspNodeHeld := { version : U32, node : BspTreeNode }

	BspNodeSlot := [Occupied(BspNodeHeld), Vacant(U32)]

	## A window added to a layout.
	BspWindowIn := { layout : U64, window : U64 }

	## A window that took another's place.
	BspReplacement := { from : U64, to : U64 }

	## A native resize of a layout's window, with the screen and its tiling area it happened on.
	BspNativeResize := { layout : U64, window : U64, old : Rect, new : Rect, screen : Rect, tiling : Rect }

	## A layout and a direction.
	BspLayoutDirection := { layout : U64, direction : BspDirection }

	## A layout and an app's windows.
	BspAppIn := { layout : U64, windows : List(U64) }

	## Which of a layout's windows to list (upstream's `WindowQuery`).
	BspQuery := [All, Visible, Selection, Stack(U64)]

	BspWindowsQuery := { layout : U64, query : BspQuery }

	BspMoveQuery := { layout : U64, window : U64, direction : BspDirection }

	## A window's place: the side taken at each split from the root (0 first, 1 second), then
	## its position in its stack, if stacked; none when the layout does not hold it.
	BspSlot := [Slot(List(U64)), NoSlot]

	## How the selection moves (upstream's `Focus`).
	BspFocus := [Select(U64), Ascend, Descend, Direction(BspDirection)]

	BspFocusIn := { layout : U64, focus : BspFocus }

	## A focus movement: the window it reached, if any, and those to raise.
	BspMovement := { window : MaybeWord, raise : List(U64) }

	## What a focus did (upstream's `FocusResult`): whether a selection change was handled, or a
	## movement.
	BspFocused := [Selection(Bool), Movement(BspMovement)]

	## What splitting the selection makes (upstream's `LayoutKind`): a tiled split, or a stacked
	## one (which BSP does not do).
	BspSplitKind := [Tiled(BspOrientation), Stacked]

	## Which fullscreen a toggle asks for: the whole screen, or the screen within its gaps.
	BspFullscreen := [Screen, WithinGaps]

	## Which split a resize changes (upstream's `ResizeOrientation`): the nearest along an axis,
	## or the nearest whatever its orientation.
	BspResizeAlong := [Horizontal, Vertical, Smart]

	BspResize := { amount : F64, along : BspResizeAlong }

	## Two windows.
	BspPair := { a : U64, b : U64 }

	## What a window dropped onto another does (upstream's `WindowDropAction`; its `Move` is
	## `Moved`, since the glue does not escape Rust's `move`).
	BspDropAction := [Swap, Stack, Insert(BspDirection), Moved(BspDirection), ConsumeOrExpel(BspDirection)]

	BspDrop := { source : U64, target : U64, action : BspDropAction }

	## An edit of a layout (upstream's `LayoutEdit`, the ones its BSP system reads; its `Move` is
	## `Moved`, since the glue does not escape Rust's `move`).
	BspEdit := [Moved(BspDirection), Swap(BspPair), Drop(BspDrop), Join(BspDirection), ConsumeOrExpel(BspDirection), Unjoin, Split(BspSplitKind), Fullscreen(BspFullscreen), Resize(BspResize), Rebalance, ToggleOrientation]

	BspEditIn := { layout : U64, edit : BspEdit }

	## What an edit did (upstream's `EditResult`).
	BspEdited := [Unsupported, Unchanged, Changed, Raise(List(U64))]

	## A system as the platform holds it (`Tree::from_parts`): each table's slots and free list
	## (in taking order: the last first), the windows left out, the settings.
	BspTreeSeed := { layouts : List(BspLayoutSlot), layout_free : List(U32), nodes : List(BspNodeSlot), node_free : List(U32), left_out : List(U64), insertion : BspInsertion, aspect : Aspect }

	## A window's constraints the host knows, by the window's word (the tree's windows are
	## words).
	BspKnown := { window : U64, constraints : WindowConstraints }

	## A layout's frames (`Tree::shape`, then the BSP geometry): the layout, the screen, the
	## gaps, and every window's constraints the host knows.
	BspFramesQuestion := { layout : U64, screen : Rect, gaps : Gaps, known : List(BspKnown) }

	## One operation of the system (`Tree`'s methods, one for one; an app closing lists the
	## app's windows, since Roc has no pid).
	BspTreeEvent := [
		CreateLayout,
		CloneLayout(U64),
		RemoveLayout(U64),
		Participation(BspParticipation),
		Insertion(BspInsertion),
		AspectRatio(Aspect),
		AddWindow(BspWindowIn),
		RemoveWindow(U64),
		ReplaceWindow(BspReplacement),
		CloseApp(List(U64)),
		ResizeWindow(BspNativeResize),
		ContainsLayout(U64),
		WindowParticipates(U64),
		SelectedWindow(U64),
		WindowInDirection(BspLayoutDirection),
		ContainsWindow(BspWindowIn),
		HasAnyFullscreenNode(U64),
		HasWindowsForApp(BspAppIn),
		Windows(BspWindowsQuery),
		CanMoveWindow(BspMoveQuery),
		WindowSlot(BspWindowIn),
		Focus(BspFocusIn),
		Edit(BspEditIn),
	]

	## What an operation answers: a layout it made, whether a window's participation changed, a
	## query's answer, or nothing.
	BspTreeAnswer := [Layout(U64), Participated(Bool), Done, Verdict(Bool), Window(MaybeWord), Windows(List(U64)), WindowSlot(BspSlot), Focused(BspFocused), Edited(BspEdited)]



	## Where a claimed window returns to when its claim ends.
	Destination := [Returns(U64), Nowhere].{
		## The same workspace, or both nowhere.
		is_eq : Destination, Destination -> Bool
		is_eq = |a, b|
			match (a, b) {
				(Returns(x), Returns(y)) => x == y
				(Nowhere, Nowhere) => True
				(Returns(_), Nowhere) | (Nowhere, Returns(_)) => False
			}
	}

	## Whether focus follows the mouse while the claimed window is focused.
	FocusFollowsMouse := [Followed, Suppressed]

	## What a claim asks of rift besides holding the window.
	ClaimFlags := { focus_follows_mouse : FocusFollowsMouse }

	## A claim, whole: its owner (the one part a decision reads), how it claimed, the id it
	## claimed, and where the window returns when it ends.
	Claim := { owner : U64, flags : ClaimFlags, original : U64, destination : Destination }

	## Who places a window.
	Ownership := [Assigned(U64), Claimed(Claim), Held]

	WindowOwnership := { window : U64, ownership : Ownership }

	Owned := [Unowned, Owner(Ownership)]

	OwnershipEvent := { window : U64, owned : Owned }

	## Who takes a removed workspace's windows and claims.
	Successor := [Into(U64), Unassign]

	WorkspaceRemoved := { workspace : U64, successor : Successor }

	## A change to the joined workspaces.
	JoinedEvent := [Created(WorkspaceSpace), Removed(WorkspaceRemoved), Moved(WorkspaceSpace), Activated(SpaceActivation), Cleared(U64), Owned(OwnershipEvent)]

	JoinedQuestion := { events : List(JoinedEvent) }

	## What an event met: accepted, or why it was refused.
	Outcome := [Accepted, Exists(U64), Unknown(U64), OwnSuccessor(U64), Shown(WorkspaceSpace)]

	WorkspaceWindows := { workspace : U64, windows : List(U64) }

	## A run's outcomes and the joined workspaces it leaves: catalog, activations, owners, and
	## each catalogued workspace's assigned windows, in catalog order.
	JoinedView := { outcomes : List(Outcome), catalog : List(WorkspaceSpace), activations : List(SpaceActivation), owners : List(WindowOwnership), windows : List(WorkspaceWindows) }



	# The core's events and effects (`rift_core::Event`, `rift_core::Effect`).

	Button := [Left, Right]

	## What a press lands on: a window, or none.
	PressOn := [OnWindow(WindowId), OnNothing]

	Press := { on : PressOn, button : Button }

	## A release: of a button, or one the platform missed.
	ReleaseOf := [ButtonUp(Button), Missed]

	Rekey := { from : WindowId, to : WindowId }

	## A window seen at a frame.
	Sighting := { window : WindowId, frame : Rect }

	Want := [Arrange, ArrangePositions]

	Desire := { window : WindowId, frame : Rect, request : U32 }

	ArrangeOf := { now : U64, want : Want, desires : List(Desire) }

	AnimateOf := { now : U64, desires : List(Desire) }

	## A time and a request (an animation's end, a tick).
	Stamp := { now : U64, request : U32 }

	## Where a stopped animation left its window, if it published a frame.
	StoppedAt := [StoppedAt(Rect), NotPublished]

	AnimationStop := { window : WindowId, request : U32, at : StoppedAt }

	FollowOf := { request : U32, window : WindowId, frame : Rect }

	## The request an observed frame answers, if the platform stamped it.
	AsOf := [AsOf(U32), Unstamped]

	Observation := { now : U64, request : U32, window : WindowId, frame : Rect, as_of : AsOf }

	SentOutsideOf := { window : WindowId, request : U32 }

	## The platform could not deliver the write sent under `failed`; a desire waiting behind it
	## goes out under `request`.
	WriteFailure := { now : U64, request : U32, window : WindowId, failed : U32 }

	Event := [
		Press(Press),
		Drag(WindowId),
		## A button came up.
		Release(ReleaseOf),
		LetGo,
		Gone(WindowId),
		Rekeyed(Rekey),
		Seen(Sighting),
		Ceded(WindowId),
		Unseen(WindowId),
		Distrust(List(WindowId)),
		Arrange(ArrangeOf),
		Animate(AnimateOf),
		AnimationEnded(Stamp),
		AnimationStopped(AnimationStop),
		Follow(FollowOf),
		Observe(Observation),
		Tick(Stamp),
		Forget(WindowId),
		SentOutside(SentOutsideOf),
		WriteFailed(WriteFailure),
	]

	## A frame written to a window under a request.
	WriteEffect := { window : WindowId, frame : Rect, request : U32 }

	AnimateEffect := { window : WindowId, from : Rect, to : Rect, request : U32 }

	PendingTarget := [Target(Rect), NoTarget]

	PendingEffect := { window : WindowId, target : PendingTarget }

	PlanEffect := { request : U32, positions : List(WriteEffect), frames : List(WriteEffect) }

	Reply := [NoNews, InTransit, Acked, Refused, Foreign]

	ReplyEffect := { window : WindowId, reply : Reply }

	NoteKind := [Discarded, TimedOut, GaveUp]

	NoteEffect := { window : WindowId, note : NoteKind }

	## The timer: armed for a time, or not.
	Armed := [Armed(U64), Disarmed].{
		## Armed for the same time, or both disarmed.
		is_eq : Armed, Armed -> Bool
		is_eq = |a, b|
			match (a, b) {
				(Armed(x), Armed(y)) => x == y
				(Disarmed, Disarmed) => True
				(Armed(_), Disarmed) | (Disarmed, Armed(_)) => False
			}
	}

	UngripWhy := [Released, LetGo, Gone]

	UngripEffect := { window : WindowId, why : UngripWhy, changed : List(WindowId) }

	Effect := [
		Write(WriteEffect),
		Animate(AnimateEffect),
		Pending(PendingEffect),
		Plan(PlanEffect),
		Queued(WindowId),
		Seen(Sighting),
		Replied(ReplyEffect),
		Note(NoteEffect),
		Timer(Armed),
		Grip(WindowId),
		DragBegan(WindowId),
		Ungrip(UngripEffect),
		Regrip(Rekey),
	]
}
