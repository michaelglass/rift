use std::path::PathBuf;

use serde::de::Error as DeError;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

use crate::{
    Direction, DisplaySelector, LayoutMode, ResizeOrientation, RestoreScope, RestoreSource,
    WindowId, WorkspaceSelector,
};

/// A number a command carries (an amount, a delta, a ratio, a duration, a gap), parsed once
/// where the command enters rift (IPC, the CLI, a config file, the settings window): always
/// finite, since no layout or setting means a NaN or an infinity.
#[derive(Clone, Copy, Debug, Default, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(try_from = "f64", into = "f64")]
pub struct Amount(f64);

/// Why a number is not an [`Amount`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AmountError {
    NotANumber,
    NotFinite,
}

impl std::fmt::Display for AmountError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            AmountError::NotANumber => "expected a number",
            AmountError::NotFinite => "a command's number must be finite (not NaN or infinite)",
        })
    }
}

impl std::error::Error for AmountError {}

impl Amount {
    /// A literal the code writes; a non-finite one is a bug in that code.
    pub const fn literal(value: f64) -> Self {
        assert!(value.is_finite(), "a literal amount is finite");
        Amount(value)
    }

    pub fn get(self) -> f64 { self.0 }
}

impl TryFrom<f64> for Amount {
    type Error = AmountError;

    fn try_from(value: f64) -> Result<Self, AmountError> {
        if value.is_finite() {
            Ok(Amount(value))
        } else {
            Err(AmountError::NotFinite)
        }
    }
}

impl From<Amount> for f64 {
    fn from(amount: Amount) -> f64 { amount.0 }
}

impl std::str::FromStr for Amount {
    type Err = AmountError;

    fn from_str(text: &str) -> Result<Self, AmountError> {
        text.trim().parse::<f64>().map_err(|_| AmountError::NotANumber)?.try_into()
    }
}

impl std::fmt::Display for Amount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { self.0.fmt(f) }
}

/// Editable argument types shared by command discovery and native settings.
#[derive(Clone, Debug, PartialEq)]
pub enum CommandArgument {
    Direction(Direction),
    Orientation(ResizeOrientation),
    Workspace(WorkspaceSelector),
    WorkspaceIndex(usize),
    Display(DisplaySelector),
    Layout(LayoutMode),
    Keymap(String),
    Number(Amount),
    Integer(i32),
    Backwards(bool),
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FloatingWindowSizePreset {
    Smart,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum FloatingWindowSize {
    Dimensions { w: Amount, h: Amount },
    Preset(FloatingWindowSizePreset),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToggleWindowFloatingOptions {
    #[serde(default)]
    pub center: bool,
    #[serde(default)]
    pub size: Option<FloatingWindowSize>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, rift_config_derive::CommandCatalog)]
#[serde(rename_all = "snake_case")]
#[command(
    group = 0,
    defaults(
        Direction = "Direction::Left",
        ResizeOrientation = "ResizeOrientation::Smart",
        LayoutMode = "LayoutMode::Traditional"
    )
)]
pub enum LayoutCommand {
    #[command(group = 1, label = "Focus next window", order = 13)]
    NextWindow,
    #[command(group = 1, label = "Focus previous window", order = 14)]
    PrevWindow,
    #[command(group = 1, label = "Focus window", order = 12)]
    MoveFocus(#[serde(rename = "direction")] Direction),
    #[command(group = 1, label = "Focus parent", order = 15)]
    Ascend,
    #[command(group = 1, label = "Focus child", order = 16)]
    Descend,
    #[command(label = "Move window", order = 0)]
    MoveNode(Direction),
    #[command(order = 1)]
    JoinWindow(Direction),
    #[command(order = 2)]
    ConsumeOrExpelWindow(Direction),
    #[command(group = 4, order = 17)]
    ToggleStack,
    #[command(group = 4, order = 18)]
    ToggleOrientation,
    #[command(group = 4, order = 19)]
    Balance,
    #[command(group = 4, order = 20)]
    UnjoinWindows,
    #[command(order = 4)]
    ToggleFocusFloating,
    #[command(label = "Toggle floating", order = 3)]
    ToggleWindowFloating,
    #[command(skip, alias = "ToggleWindowFloating")]
    ToggleWindowFloatingWithOptions(ToggleWindowFloatingOptions),
    #[command(order = 5)]
    ToggleFullscreen,
    #[command(label = "Fullscreen within gaps", order = 6)]
    ToggleFullscreenWithinGaps,
    #[command(label = "Grow window", order = 7)]
    ResizeWindowGrow(ResizeOrientation),
    #[command(label = "Shrink window", order = 8)]
    ResizeWindowShrink(ResizeOrientation),
    #[command(
        order = 9,
        sample = "Self::ResizeWindowBy { amount: Amount::literal(10.0) }"
    )]
    ResizeWindowBy { amount: Amount },
    #[command(
        group = 4,
        label = "Scroll strip by",
        order = 22,
        sample = "Self::ScrollStrip { delta: Amount::literal(100.0) }"
    )]
    ScrollStrip { delta: Amount },
    #[command(group = 4, order = 23)]
    SnapStrip,
    #[command(group = 4, label = "Cycle preset width", order = 24)]
    SwitchPresetColumnWidth {
        #[serde(default)]
        backwards: bool,
    },
    #[command(order = 10)]
    CenterSelection,
    #[command(skip)]
    NextWorkspace(Option<bool>),
    #[command(skip)]
    PrevWorkspace(Option<bool>),
    #[command(skip)]
    SwitchToWorkspace(usize),
    #[command(skip)]
    MoveWindowToWorkspace {
        workspace: WorkspaceSelector,
        follow: bool,
        window_id: Option<u32>,
    },
    #[command(group = 4, label = "Set layout to", order = 21)]
    SetWorkspaceLayout {
        workspace: Option<usize>,
        mode: LayoutMode,
    },
    #[command(skip)]
    CreateWorkspace,
    #[command(skip)]
    SwitchToLastWorkspace,
    #[command(
        label = "Swap specific windows",
        order = 51,
        sample = "Self::SwapWindows(WindowId { pid: 0, idx: 1 }, WindowId { pid: 0, idx: 2 })"
    )]
    SwapWindows(WindowId, WindowId),
    /// Move `window`'s tab to where `target`'s tab sits in their shared stack,
    /// shifting the tabs between them toward the vacated slot.
    #[command(
        label = "Move a stack tab",
        order = 52,
        sample = "Self::MoveStackTab { window: WindowId { pid: 0, idx: 1 }, target: WindowId { pid: 0, idx: 2 } }"
    )]
    MoveStackTab { window: WindowId, target: WindowId },
    #[command(
        group = 4,
        label = "Adjust master ratio by",
        order = 25,
        sample = "Self::AdjustMasterRatio(Amount::literal(0.05))"
    )]
    AdjustMasterRatio(Amount),
    #[command(
        group = 4,
        label = "Adjust master count by",
        order = 26,
        sample = "Self::AdjustMasterCount { delta: 1 }"
    )]
    AdjustMasterCount { delta: i32 },
    #[command(group = 4, order = 27)]
    PromoteToMaster,
    #[command(group = 4, label = "Swap master and stack", order = 28)]
    SwapMasterStack,
}

#[derive(
    Clone, Debug, Eq, PartialEq, Serialize, Deserialize, rift_config_derive::CommandCatalog,
)]
#[serde(rename_all = "snake_case")]
#[command(defaults(
    Direction = "Direction::Left",
    DisplaySelector = "DisplaySelector::Direction(Direction::Left)"
))]
pub enum ReactorCommand {
    #[command(label = "Open Settings", order = 47)]
    OpenSettings,
    #[command(skip)]
    BindingMode(String),
    #[command(label = "Print layout tree", order = 48)]
    Debug,
    #[command(label = "Serialize state", order = 49)]
    Serialize,
    #[command(
        group = 4,
        label = "Save layout to file",
        order = 54,
        sample = "Self::SaveLayout { path: std::path::PathBuf::new() }"
    )]
    SaveLayout { path: PathBuf },
    #[command(order = 50)]
    SaveAndExit,
    #[command(
        group = 4,
        label = "Restore layout from file",
        order = 55,
        sample = "Self::RestoreLayout { path: std::path::PathBuf::new(), scope: RestoreScope::Workspace, source: RestoreSource::SavedActiveSpace }"
    )]
    RestoreLayout {
        path: PathBuf,
        scope: RestoreScope,
        #[serde(default)]
        source: RestoreSource,
    },
    #[command(group = 2, order = 39)]
    SwitchSpace(Direction),
    #[command(skip)]
    ToggleSpaceActivated,
    #[command(
        group = 1,
        label = "Focus specific window",
        order = 53,
        sample = "Self::FocusWindow { window_id: WindowId { pid: 0, idx: 1 }, window_server_id: None }"
    )]
    FocusWindow {
        window_id: WindowId,
        window_server_id: Option<u32>,
    },
    #[command(skip)]
    ShowMissionControlAll,
    #[command(skip)]
    ShowMissionControlCurrent,
    #[command(skip)]
    DismissMissionControl,
    #[command(group = 3, label = "Move pointer to display", order = 38)]
    MoveMouseToDisplay(DisplaySelector),
    #[command(group = 3, order = 35)]
    FocusDisplay(DisplaySelector),
    #[command(skip)]
    CloseWindow { window_server_id: Option<u32> },
    #[command(group = 3, order = 36)]
    MoveWindowToDisplay {
        selector: DisplaySelector,
        window_id: Option<u32>,
    },
    /// Move the active workspace to another display.
    ///
    /// Rift workspaces are display-local, so windows move into the destination
    /// display's workspace at the same ordinal and that workspace becomes active.
    #[command(group = 3, order = 37)]
    MoveWorkspaceToDisplay {
        selector: DisplaySelector,
        /// Continue from the opposite edge when a directional selector has no
        /// display further in that direction.
        #[serde(default)]
        wrap_around: bool,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetricsCommand {
    ShowTiming,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigCommand {
    SetAnimate(bool),
    SetAnimationDuration(Amount),
    SetAnimationFps(Amount),
    SetAnimationEasing(AnimationEasing),
    SetMouseFollowsFocus(bool),
    SetMouseHidesOnFocus(bool),
    SetFocusFollowsMouse(bool),
    SetStackOffset(Amount),
    SetOuterGaps {
        top: Amount,
        left: Amount,
        bottom: Amount,
        right: Amount,
    },
    SetInnerGaps {
        horizontal: Amount,
        vertical: Amount,
    },
    SetWorkspaceNames(Vec<String>),
    Set {
        key: String,
        value: Value,
    },
    GetConfig,
    SaveConfig,
    ReloadConfig,
}

#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    Eq,
    PartialEq,
    Serialize,
    Deserialize,
    rift_config_derive::ConfigEnum,
)]
#[serde(rename_all = "snake_case")]
pub enum AnimationEasing {
    #[default]
    EaseInOut,
    Linear,
    EaseInSine,
    EaseOutSine,
    EaseInOutSine,
    EaseInQuad,
    EaseOutQuad,
    EaseInOutQuad,
    EaseInCubic,
    EaseOutCubic,
    EaseInOutCubic,
    EaseInQuart,
    EaseOutQuart,
    EaseInOutQuart,
    EaseInQuint,
    EaseOutQuint,
    EaseInOutQuint,
    EaseInExpo,
    EaseOutExpo,
    EaseInOutExpo,
    EaseInCirc,
    EaseOutCirc,
    EaseInOutCirc,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RiftCommand {
    Layout(LayoutCommand),
    Metrics(MetricsCommand),
    Reactor(ReactorCommand),
    Config(ConfigCommand),
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum TypedRiftCommand {
    Layout(LayoutCommand),
    Metrics(MetricsCommand),
    Reactor(ReactorCommand),
    Config(ConfigCommand),
}

#[derive(Deserialize)]
enum LegacyCommand {
    #[serde(alias = "reactor")]
    Reactor(LegacyReactorCommand),
    #[serde(alias = "config")]
    Config(ConfigCommand),
}

#[derive(Deserialize)]
#[serde(untagged)]
enum LegacyReactorCommand {
    Layout(LayoutCommand),
    Metrics(MetricsCommand),
    Reactor(ReactorCommand),
}

impl From<TypedRiftCommand> for RiftCommand {
    fn from(command: TypedRiftCommand) -> Self {
        match command {
            TypedRiftCommand::Layout(command) => Self::Layout(command),
            TypedRiftCommand::Metrics(command) => Self::Metrics(command),
            TypedRiftCommand::Reactor(command) => Self::Reactor(command),
            TypedRiftCommand::Config(command) => Self::Config(command),
        }
    }
}

impl<'de> Deserialize<'de> for RiftCommand {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where D: Deserializer<'de> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum CommandInput {
            Typed(TypedRiftCommand),
            LegacyJson(String),
        }

        match CommandInput::deserialize(deserializer)? {
            CommandInput::Typed(command) => Ok(command.into()),
            CommandInput::LegacyJson(command) => decode_legacy_command(&command),
        }
    }
}

fn decode_legacy_command<E>(command: &str) -> Result<RiftCommand, E>
where E: DeError {
    match serde_json::from_str::<LegacyCommand>(command)
        .map_err(|error| E::custom(format!("invalid legacy command JSON: {error}")))?
    {
        LegacyCommand::Config(command) => Ok(RiftCommand::Config(command)),
        LegacyCommand::Reactor(LegacyReactorCommand::Layout(command)) => {
            Ok(RiftCommand::Layout(command))
        }
        LegacyCommand::Reactor(LegacyReactorCommand::Metrics(command)) => {
            Ok(RiftCommand::Metrics(command))
        }
        LegacyCommand::Reactor(LegacyReactorCommand::Reactor(command)) => {
            Ok(RiftCommand::Reactor(command))
        }
    }
}
