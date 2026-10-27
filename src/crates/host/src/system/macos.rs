//! SMAppService uses only public frameworks. The Mac never posts system
//! notifications: the HUD island is its alert surface.
pub(super) use super::unix::{client_command_line, configure_client_command, stop_client};
use objc2::{
    msg_send,
    rc::Retained,
    runtime::{AnyClass, AnyObject, Bool},
};
use objc2_foundation::{NSBundle, NSError, NSString};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
#[link(name = "ServiceManagement", kind = "framework")]
unsafe extern "C" {}
fn bundled() -> bool {
    let bundle = NSBundle::mainBundle();
    bundle.bundlePath().to_string().ends_with(".app") && bundle.bundleIdentifier().is_some()
}
/// macOS naps accessory apps with no visible window: their timers and file
/// watchers run seconds to minutes late. Live session tracking dies of that
/// — a remote turn's heartbeat lapses before the delayed watch fires — so a
/// background activity claim is taken once for the process. It exempts the
/// app from napping while holding no sleep: neither the display's nor the
/// machine's. The claim lives as long as the returned token, which is never
/// released.
pub(super) fn begin_background_activity() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| unsafe {
        let Some(class) = AnyClass::get(c"NSProcessInfo") else {
            return;
        };
        let info: Retained<AnyObject> = msg_send![class, processInfo];
        let reason = NSString::from_str("agent HUD tracks live sessions");
        // NSActivityBackground.
        let activity: Retained<AnyObject> =
            msg_send![&*info, beginActivityWithOptions: 0xffu64, reason: &*reason];
        std::mem::forget(activity);
    });
}
fn service() -> Option<Retained<AnyObject>> {
    if !bundled() {
        return None;
    }
    let class = AnyClass::get(c"SMAppService")?;
    Some(unsafe { msg_send![class, mainAppService] })
}
fn login() -> Value {
    let Some(service) = service() else {
        return json!({"supported":false,"enabled":false,"status":"unavailable","notice":"Launch at login requires a real app bundle on macOS 13 or later."});
    };
    let status: isize = unsafe { msg_send![&*service, status] };
    let notice = match status {
        2 => Some(format!(
            "Approve {} in System Settings → General → Login Items.",
            app_name()
        )),
        3 => Some("The login service was not found in this app bundle.".to_owned()),
        _ => None,
    };
    json!({"supported":true,"enabled":status==1,"status":match status {1=>"enabled",2=>"requires-approval",0=>"disabled",_=>"unavailable"},"notice":notice})
}
/// The name Login Items lists: this app bundle's own, so each build says its own.
fn app_name() -> String {
    let path = NSBundle::mainBundle().bundlePath().to_string();
    Path::new(&path)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("Agent HUD")
        .to_owned()
}
/// Returns whether the login item status may have changed.
pub(super) fn register_if_never_registered() -> bool {
    let Some(service) = service() else {
        return true;
    };
    let current: isize = unsafe { msg_send![&*service, status] };
    if current == 0 {
        let mut error: *mut NSError = std::ptr::null_mut();
        let _: Bool = unsafe { msg_send![&*service, registerAndReturnError: &mut error] };
    }
    true
}
pub(super) fn set_login(_identifier: &str, enabled: bool) -> Result<(), String> {
    let service = service().ok_or("Launch at login requires a real macOS app bundle")?;
    let current: isize = unsafe { msg_send![&*service, status] };
    if enabled && current == 1 || !enabled && current == 0 {
        return Ok(());
    }
    let mut error: *mut NSError = std::ptr::null_mut();
    let success: Bool = unsafe {
        if enabled {
            msg_send![&*service,registerAndReturnError:&mut error]
        } else {
            msg_send![&*service,unregisterAndReturnError:&mut error]
        }
    };
    if success.as_bool() {
        Ok(())
    } else {
        Err("macOS could not update the login item; check Login Items in System Settings.".into())
    }
}
pub(super) fn status(_identifier: &str) -> Result<Value, String> {
    Ok(json!({"platform":"macos","launchAtLogin":login()}))
}

pub(super) fn cli_candidates(home: &Path, command: &str, dirs: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut candidates = vec![];
    if command == "codex" {
        // The app's own command line, where the engine looks for it too
        // (platform `engine_candidates`): Codex.app, or the ChatGPT app that
        // carries Codex inside it.
        for root in [PathBuf::from("/Applications"), home.join("Applications")] {
            for app in ["Codex.app", "ChatGPT.app"] {
                for tail in [
                    "Contents/Resources/codex-cli/bin/codex",
                    "Contents/Resources/codex",
                ] {
                    candidates.push(root.join(app).join(tail));
                }
            }
        }
    }
    if matches!(command, "agy" | "antigravity") {
        candidates.push(home.join(".antigravity/antigravity/bin").join(command));
        for root in [PathBuf::from("/Applications"), home.join("Applications")] {
            for app in ["Antigravity.app", "Antigravity IDE.app"] {
                for tail in [
                    "Contents/Resources/app/bin/agy",
                    "Contents/Resources/app/bin/antigravity",
                    "Contents/Resources/bin/agy",
                ] {
                    candidates.push(root.join(app).join(tail));
                }
            }
        }
    }
    candidates.extend(dirs.into_iter().map(|dir| dir.join(command)));
    candidates
}

pub(super) fn private_output(options: &mut std::fs::OpenOptions) {
    use std::os::unix::fs::OpenOptionsExt;
    options.mode(0o600);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_is_found_in_the_chatgpt_app_too() {
        let home = Path::new("/Users/alice");
        let dir = PathBuf::from("/opt/homebrew/bin");
        let candidates = cli_candidates(home, "codex", vec![dir.clone()]);
        for path in [
            PathBuf::from("/Applications/Codex.app/Contents/Resources/codex"),
            PathBuf::from("/Applications/ChatGPT.app/Contents/Resources/codex-cli/bin/codex"),
            home.join("Applications/ChatGPT.app/Contents/Resources/codex-cli/bin/codex"),
        ] {
            assert!(candidates.contains(&path), "{path:?} is not a candidate");
        }
        assert_eq!(candidates.last(), Some(&dir.join("codex")));
        let agy_candidates = cli_candidates(home, "agy", vec![dir.clone()]);
        for path in [
            home.join(".antigravity/antigravity/bin/agy"),
            PathBuf::from("/Applications/Antigravity.app/Contents/Resources/app/bin/agy"),
            home.join("Applications/Antigravity.app/Contents/Resources/app/bin/agy"),
        ] {
            assert!(
                agy_candidates.contains(&path),
                "{path:?} is not an agy candidate"
            );
        }
        assert_eq!(
            cli_candidates(home, "claude", vec![dir.clone()]),
            [dir.join("claude")]
        );
    }
}
