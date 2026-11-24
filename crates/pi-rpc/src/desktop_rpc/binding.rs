//! `get_profile_binding`, `clear_profile_binding` and
//! `profile_binding_changed`: which profile this session routes its
//! subagents with, read through the rule launches use
//! (`resolveSessionProfile`), and `/gentle:profiles clear` for this session.
//! Publishing a profile to the repository is not here: it is the `repo` pin
//! layer, written with `set_config` on `pins`.

use super::{ChangeSource, DesktopRpcError, closed_field, decode_data};
use serde::Deserialize;
use serde_json::Value;

wire_enum! {
    /// How the session holds its profile: chosen for it (`explicit`), the
    /// one it started with (`inherited`), following the shared layers live
    /// (`follow`), or a child session reading the layers (`live`).
    ProfileBindingKind, "kind" {
        Explicit => "explicit",
        Inherited => "inherited",
        Follow => "follow",
        Live => "live",
    }
}

wire_enum! {
    /// What supplied the profile: an explicit choice for this session, or
    /// the layer it came from (the clone pin, the repository declaration, or
    /// the global active profile).
    ProfileBindingOrigin, "origin" {
        Session => "session",
        Local => "local",
        Repo => "repo",
        Global => "global",
    }
}

/// The decoded `get_profile_binding` data. `profile` is `None` for the
/// routing without a profile, and then `origin` is `None` too.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ProfileBinding {
    pub kind: ProfileBindingKind,
    pub profile: Option<String>,
    pub origin: Option<ProfileBindingOrigin>,
}

impl ProfileBinding {
    pub(super) fn from_data(data: &Value) -> Result<Self, DesktopRpcError> {
        decode_data(data)
    }

    /// The profile chosen for this session, the one a clear would undo.
    pub fn explicit_profile(&self) -> Option<&str> {
        (self.kind == ProfileBindingKind::Explicit)
            .then_some(self.profile.as_deref())
            .flatten()
    }
}

wire_enum! {
    /// How a `clear_profile_binding` ended: the binding store changed, there
    /// was nothing to clear, or the owner warned (even if the binding moved).
    ClearBindingStatus, "status" {
        Cleared => "cleared",
        Unchanged => "unchanged",
        NotCleared => "not_cleared",
    }
}

/// The decoded `clear_profile_binding` data: `binding` is the state after
/// the call and `changed` whether it differs from before.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ClearProfileBindingResult {
    pub status: ClearBindingStatus,
    pub changed: bool,
    pub binding: ProfileBinding,
}

impl ClearProfileBindingResult {
    pub(super) fn from_data(data: &Value) -> Result<Self, DesktopRpcError> {
        decode_data(data)
    }
}

/// The data of a `profile_binding_changed` event: the new
/// `get_profile_binding` answer, and whether a `clear_profile_binding` made
/// it (`desktop`) or anything else did (`external`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileBindingChanged {
    pub binding: ProfileBinding,
    pub source: ChangeSource,
}

impl ProfileBindingChanged {
    pub(super) fn from_data(data: &Value) -> Result<Self, DesktopRpcError> {
        Ok(Self {
            binding: ProfileBinding::from_data(data)?,
            source: closed_field(data, "source")?,
        })
    }
}
