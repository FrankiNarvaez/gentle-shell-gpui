//! OS integration: the single-instance guard, the agents' command shims,
//! the user's `gsd` command, the Dock/window icons, the installed code
//! editors and web browsers, and the power state.

pub mod browsers;
pub mod font_metadata;
pub mod gsd;
pub mod icons;
pub mod ides;
pub mod instance;
pub mod power;
pub mod shims;
