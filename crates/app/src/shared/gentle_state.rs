//! The default permission mode file (`<config>/permission-mode.json`): the
//! fallback for a Shell without Desktop RPC's permission slice, which
//! otherwise owns the mode and its default. The mode itself, jobs and the ODD
//! phase are pure: `gentle_core::gentle_state`.

use std::path::Path;

use gentle_core::gentle_state::PermissionMode;
use serde_json::Value;

/// Save the default every fresh session starts in:
/// `<config>/permission-mode.json`, exactly two keys, mode 0600
/// (lib/permission-mode-policy.ts). Only for a Shell that cannot switch the
/// live session over Desktop RPC (`set_permission_mode`), so callers restart
/// the backend after.
pub fn save_default_mode(config: &Path, mode: PermissionMode) -> std::io::Result<()> {
    std::fs::create_dir_all(config)?;
    let body = format!(
        "{{\n  \"schema\": \"gentle-pi.permission-mode/v1\",\n  \"mode\": \"{}\"\n}}\n",
        mode.id()
    );
    let target = config.join("permission-mode.json");
    let temp = config.join(format!(".permission-mode.json.{}.tmp", std::process::id()));
    std::fs::write(&temp, body)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&temp, std::fs::Permissions::from_mode(0o600))?;
    }
    std::fs::rename(&temp, &target)
}

/// The default fresh sessions start in, from `<config>/permission-mode.json`
/// (`None` when there's no valid file: Gentle Shell then starts in Normal).
/// The picker's placeholder until Desktop RPC's `get_permission_mode`
/// answers, and the only source on a Shell without it.
pub fn read_default_mode(config: &Path) -> Option<PermissionMode> {
    let text = std::fs::read_to_string(config.join("permission-mode.json")).ok()?;
    let value: Value = serde_json::from_str(&text).ok()?;
    let id = value["mode"].as_str()?;
    PermissionMode::ALL.into_iter().find(|mode| mode.id() == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_mode_file_roundtrips() {
        let dir = std::env::temp_dir().join(format!("gs-mode-test-{}", std::process::id()));
        save_default_mode(&dir, PermissionMode::Yolo).unwrap();
        let raw: Value = serde_json::from_str(
            &std::fs::read_to_string(dir.join("permission-mode.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            raw.as_object().unwrap().len(),
            2,
            "the policy rejects extra keys"
        );
        assert_eq!(raw["mode"], "yolo");
        // What the picker's check mark reads back.
        assert_eq!(read_default_mode(&dir), Some(PermissionMode::Yolo));
        save_default_mode(&dir, PermissionMode::SuperYolo).unwrap();
        assert_eq!(read_default_mode(&dir), Some(PermissionMode::SuperYolo));
        std::fs::write(dir.join("permission-mode.json"), "{\"mode\":\"bogus\"}").unwrap();
        assert_eq!(read_default_mode(&dir), None);
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(read_default_mode(&dir), None);
    }
}
