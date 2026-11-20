## The platform's boundary types: every record and union that crosses between Rust and Roc has
## a name of its own here, so the generated glue names it (never by hash or number; the build
## fails on either). Each mirrors the Rust core's type of the same meaning.
Boundary := [].{
	## One step's result: the model's box and the effects. Generic in the model, so the
	## platform's model variable stays out of a nominal type.
	Stepped(model) := { model : Box(model), effects : List(Effect) }

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

	# Model-free questions, shared shapes. A fact is what the platform's run of the decision
	# answered, or that it never asked it (`rift_core::plain::Fact`), one named type per
	# payload; a fact asked about keys is a list of key and answer (`Answers`). An answer is
	# the decision, or the first fact it read that its question does not carry.

	## A word (a Space, a workspace, a window-server id), or none.
	MaybeWord := [Some(U64), Nothing]

	## A window, or none.
	MaybeWindow := [SomeWindow(WindowId), NoWindow]

	FactBool := [Asked(Bool), Unasked]

	## A fact of a word or none.
	FactWord := [Asked(MaybeWord), Unasked]

	## A fact of a window or none.
	FactWindow := [Asked(MaybeWindow), Unasked]

	## A fact of a list of words.
	FactWords := [Asked(List(U64)), Unasked]

	## A yes or no asked about a word.
	WordBool := { key : U64, answer : Bool }

	## A yes or no asked about a window.
	WindowBool := { key : WindowId, answer : Bool }

	## A window asked about a window.
	WindowWindow := { key : WindowId, answer : WindowId }

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

	FactListing := [Asked(Listing), Unasked]

	ActivationQuestion := { standard : FactBool, unavailable : FactBool, space : FactWord, workspace : FactWord, active_workspace : FactWord, pending_window : FactBool, mode_refusal : FactWord, active_space_fullscreen : FactBool, unmanageable_source : FactWord, blacklisted : FactListing }

	FollowTo := { space : U64, workspace : U64 }

	## Why an activation does not follow its window.
	Stay := [NotStandard, Unavailable, NoSpace, NoWorkspace, AlreadyActive, PendingWindow, Mode(U64), FullscreenActive, Unmanageable(U64), NoBundleId, Blacklisted]

	Follow := [To(FollowTo), Stay(Stay)]

	FollowAnswer := [Decided(Follow), Unasked(U64)]

	DisplayFollowQuestion := { focused_space : MaybeWord, previous : MaybeWord, active : List(WordBool), on_known_display : List(WordBool) }

	## What is known of a window: its layer and its app.
	KnownWindow := { layer : I32, owner : U64 }

	Known := [Known(KnownWindow), Unknown]

	FactKnown := [Asked(Known), Unasked]

	UntrackedQuestion := { under_cursor : MaybeWord, tracked : FactBool, known : FactKnown }

	Surface := { layer : I32, window : U64 }

	KeyWindow := { owner : U64, window : U64 }

	UntrackedFocus := [Nothing, SystemSurface(Surface), MakeKey(KeyWindow)]

	UntrackedAnswer := [Decided(UntrackedFocus), Unasked(U64)]

	DesktopQuestion := { active : Bool, workspace_empty : FactBool }

	# Topology (`rift_core::topology`).

	## A Space, or none.
	MaybeSpace := [Space(U64), NoSpace]

	FactSpace := [Asked(MaybeSpace), Unasked]

	## Where an app inventory finds a window, by what the platform already knows of it.
	Standing := [RecoveringFullscreen, Untracked, Tracked]

	DiscoveredQuestion := { standing : Standing, live : FactWords, observable : List(WordBool), recorded : FactSpace, resolved : FactSpace }

	## The Space an app inventory takes a window to be on.
	DiscoveredSpace := [Observed(U64), Live(U64), Fallback(MaybeSpace)]

	DiscoveredAnswer := [Decided(DiscoveredSpace), Unasked(U64)]

	DiscoveryStanding := [FullscreenRecorded, Placeholder, Admissible]

	DiscoveryQuestion := { standing : DiscoveryStanding, native : MaybeSpace, assigned : MaybeSpace, by_frame : FactSpace }

	## A reported window's authoritative Space and the Space discovery may admit it on.
	DiscoverySpaces := { authoritative : MaybeSpace, discovery : MaybeSpace }

	DiscoveryAnswer := [Decided(DiscoverySpaces), Unasked(U64)]

	## A screen's display, or none for a display with no id (each display a word).
	Screen := [Display(U64), Unnamed]

	DisplaySpaces := { display : U64, spaces : List(U64) }

	Displays := { screens : List(Screen), spaces : List(DisplaySpaces) }

	## A Space macOS renumbered.
	Remap := { old : U64, new : U64 }

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

	WordTracked := { key : U64, answer : TrackedWindow }

	CandidatesQuestion := { window_spaces : List(ServerSpace), activated : List(U64), tracked : List(WordTracked), reconcilable : List(WindowBool) }

	## An app's windows.
	AppWindows := { pid : I32, windows : List(WindowId) }

	CandidatesAnswer := [Decided(List(AppWindows)), Unasked(U64)]

	# Lifecycle (`rift_core::lifecycle`).

	## Why app rules run over some windows.
	RuleRun := [Admission, Reapply]

	## A Space or none asked about a word.
	WordSpace := { key : U64, answer : MaybeSpace }

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

	AdmissionFacts := { manageability : Manageability, rule : Rule, ax : AxKind, native : NativeKind, minimize : Minimize }

	## Why a window is not admitted.
	Veto := [NativeTransient, Minimized, AxTransient, RuleIgnores, Unmanageable]

	WindowAdmission := [Admitted, Vetoed(Veto)]

	## One admission fact, changed.
	FactChange := [Manageability(Manageability), Rule(Rule), Ax(AxKind), Native(NativeKind), Minimize(Minimize)]

	Reading := [Live(AdmissionFacts), Changed(FactChange), Gone]

	## A window's facts, or none without AX state.
	KnownFacts := [Known(AdmissionFacts), NoFacts]

	ReadQuestion := { before : KnownFacts, reading : Reading }

	## A change in whether rift manages a window.
	Change := [NoChange, Admitted, Suspended(Veto)]

	ReadAnswer := { after : KnownFacts, change : Change }

	Presence := [NoPresence, Live(AdmissionFacts), Invalidated(AdmissionFacts)]

	PresenceEvent := [Reading(Reading), Invalidate, Revalidate]

	PresenceQuestion := { before : Presence, event : PresenceEvent }

	PresenceOutcome := [Read(Change), Invalidated(Bool), Revalidated(Bool)]

	PresenceAnswer := { after : Presence, outcome : PresenceOutcome, manages : Bool }

	## Two admission changes in a row.
	NetQuestion := { first : Change, then : Change }

	## The mode table's question: an initiative, and the modes Rift is in.
	InputClass := [Steering, Arranging, Inert, Hover]

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

	## The lifecycle after a reading, and the change in the window's admission.
	LifecycleStepped(model) := { model : Box(model), change : Change }

	## Whether an invalidation found AX state to invalidate.
	Invalidation := [Invalidated, NoAxState]

	## Whether a revalidation brought an invalidated window back (a valid window, or one
	## without AX state, was not invalidated).
	Revalidation := [Revalidated, NotInvalidated]

	## The lifecycle after an invalidation, and what it found.
	LifecycleInvalidated(model) := { model : Box(model), invalidation : Invalidation }

	## The lifecycle after a revalidation, and what it found.
	LifecycleRevalidated(model) := { model : Box(model), revalidation : Revalidation }

	## The lifecycle, unchanged, and every window with AX state.
	LifecycleViewed(model) := { model : Box(model), windows : List(PresentWindow) }

	Report := [Reported, Omitted]

	InventoryView := [InView, OutOfView]

	## A window's minimize state, or none without AX state.
	InventoryMinimize := [Shown, Minimized, NoAxState]

	FullscreenRecord := [Recorded, NotRecorded]

	NativeIdentity := [Known(U64), Unknown]

	AxValidity := [Valid, Invalidated]

	InventoryFacts := { report : Report, view : InventoryView, minimize : InventoryMinimize, fullscreen : FullscreenRecord, identity : NativeIdentity, validity : AxValidity }

	## The window server's type for a window, if it says.
	Suitability := [Suitable, Unsuitable, SuitabilityUnknown]

	Ordering := [OrderedIn, OrderedOut, OrderingUnknown]

	## Whether the window server's record looks like a window, if it says.
	Look := [AWindow, NotAWindow, LookUnknown]

	NativeEvidence := { suitability : Suitability, ordering : Ordering, look : Look }

	FactEvidence := [Asked(NativeEvidence), Unasked]

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

	Showing := { current : U64, previous : MaybeWord, remembered : MaybeWord }

	## What a native Space shows.
	Activation := [Showing(Showing), Vacant(MaybeWord)]

	SpaceActivation := { space : U64, activation : Activation }

	TopologyEvent := [Created(WorkspaceSpace), Removed(U64), Moved(WorkspaceSpace), Activated(SpaceActivation), Cleared(U64)]

	TopologyQuestion := { events : List(TopologyEvent) }

	## A topology: its catalog, and what each Space shows.
	TopologyView := { catalog : List(WorkspaceSpace), activations : List(SpaceActivation) }

	## The topology, unchanged, and what it holds: the catalog and every Space's activation.
	TopologyViewed(model) := { model : Box(model), catalog : List(WorkspaceSpace), activations : List(SpaceActivation) }

	## Where a claimed window returns to when its claim ends.
	Destination := [Returns(U64), Nowhere]

	Claim := { owner : U64, destination : Destination }

	## Who places a window.
	Ownership := [Assigned(U64), Claimed(Claim), Held]

	WindowOwnership := { window : U64, ownership : Ownership }

	Owned := [Unowned, Owner(Ownership)]

	OwnershipEvent := { window : U64, owned : Owned }

	OwnershipQuestion := { events : List(OwnershipEvent), catalog : List(U64) }

	## A window owned by a workspace the catalog lacks.
	Orphan := { window : U64, workspace : U64 }

	OwnershipView := { owners : List(WindowOwnership), orphans : List(Orphan) }

	## The ownerships after a change, and who places the changed window now.
	OwnershipsStepped(model) := { model : Box(model), owned : Owned }

	## The ownerships, unchanged, and every owned window.
	OwnershipsViewed(model) := { model : Box(model), owners : List(WindowOwnership) }

	## Who takes a removed workspace's windows and claims.
	Successor := [Into(U64), Unassign]

	WorkspaceRemoved := { workspace : U64, successor : Successor }

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
		## A button came up (`Release`, renamed: a tag named `Release` collides with the glue's
		## own `<Type>Release` helpers).
		Released(ReleaseOf),
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
	Armed := [Armed(U64), Disarmed]

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
