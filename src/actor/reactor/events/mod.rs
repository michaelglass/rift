pub mod app;
pub mod command;
pub mod drag;
pub mod focus;
pub mod space;
pub mod system;
pub mod window;
pub mod window_discovery;

mod outcome;

pub(crate) use outcome::{
    CameraMotion, CloseWindowRequest, EventOutcome, HeldCameras, TopologyReassignment,
    WindowDiscoveryRequest,
};
