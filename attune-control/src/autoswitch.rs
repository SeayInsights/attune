//! Switching GoXLR profiles to follow whatever you are using.
//!
//! # The problem
//!
//! A competitive voicing is right for a shooter and wrong for music. Attune's
//! settings already follow the GoXLR profile, so switching profile switches
//! everything -- correction, voicing, crossfeed, the lot. What was missing was
//! anybody to press the button.
//!
//! # How the foreground application is identified
//!
//! By the executable's file name, not its window title. Titles change with
//! what is open in them ("Notepad" versus "notes.txt - Notepad") and are
//! localised; the executable name does not move. It is the same thing
//! SteelSeries, DTS and Razer key their per-game profiles on.
//!
//! # Why polling
//!
//! `SetWinEventHook` would deliver foreground changes without polling, but it
//! needs a message loop on a thread that owns it, and the daemon has no window
//! of its own. A two-second poll costs one API call and cannot leak a hook if
//! the daemon exits badly. Nothing here is time-critical -- a profile arriving
//! two seconds after a game launches is indistinguishable from instant.
//!
//! # What it will not do
//!
//! Switch back to something you did not pick. When a rule stops matching, the
//! profile stays where it is rather than reverting to a remembered default: an
//! automation that quietly undoes deliberate changes is worse than one that
//! does too little, and alt-tabbing out of a game should not change how your
//! music sounds.

use std::time::Duration;

use serde::{Deserialize, Serialize};

/// How often the foreground application is checked.
pub const POLL_INTERVAL: Duration = Duration::from_secs(2);

/// One "when this is in front, use that profile" rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rule {
    /// Executable file name, e.g. `cs2.exe`. Matched case-insensitively.
    pub executable: String,
    /// The GoXLR profile to load.
    pub profile: String,
    /// Off without being deleted, so a rule can be parked rather than retyped.
    #[serde(default = "enabled_by_default")]
    pub enabled: bool,
}

fn enabled_by_default() -> bool {
    true
}

impl Rule {
    fn matches(&self, executable: &str) -> bool {
        self.enabled && self.executable.eq_ignore_ascii_case(executable)
    }
}

/// The rule set, in priority order: the first match wins.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Rules {
    #[serde(default)]
    pub rules: Vec<Rule>,
    /// Whether the watcher acts at all. Off by default -- software that
    /// changes your audio without being asked is software you turn off.
    #[serde(default)]
    pub enabled: bool,
}

impl Rules {
    /// The profile that should be active for this executable, if any.
    pub fn profile_for(&self, executable: &str) -> Option<&str> {
        if !self.enabled {
            return None;
        }
        self.rules
            .iter()
            .find(|rule| rule.matches(executable))
            .map(|rule| rule.profile.as_str())
    }

    /// Add or replace the rule for an executable.
    pub fn set(&mut self, executable: &str, profile: &str) {
        match self
            .rules
            .iter_mut()
            .find(|r| r.executable.eq_ignore_ascii_case(executable))
        {
            Some(existing) => {
                existing.profile = profile.to_string();
                existing.enabled = true;
            }
            None => self.rules.push(Rule {
                executable: executable.to_string(),
                profile: profile.to_string(),
                enabled: true,
            }),
        }
    }

    /// Remove the rule for an executable. Returns whether one went.
    pub fn remove(&mut self, executable: &str) -> bool {
        let before = self.rules.len();
        self.rules
            .retain(|r| !r.executable.eq_ignore_ascii_case(executable));
        self.rules.len() != before
    }
}

/// The executable name of whatever window has focus, e.g. `cs2.exe`.
///
/// `None` when there is no foreground window, when it belongs to a process
/// this one may not open -- anything running elevated, which includes some
/// anti-cheat protected games -- or on a platform without the concept.
#[cfg(windows)]
pub fn foreground_executable() -> Option<String> {
    use windows::Win32::Foundation::{CloseHandle, HANDLE, MAX_PATH};
    use windows::Win32::System::Threading::{
        OpenProcess, PROCESS_NAME_FORMAT, PROCESS_QUERY_LIMITED_INFORMATION,
        QueryFullProcessImageNameW,
    };
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};

    unsafe {
        let window = GetForegroundWindow();
        if window.is_invalid() {
            return None;
        }

        let mut pid = 0u32;
        GetWindowThreadProcessId(window, Some(&mut pid));
        if pid == 0 {
            return None;
        }

        // QUERY_LIMITED_INFORMATION rather than QUERY_INFORMATION: it is the
        // least this needs, and it is the one that works across an integrity
        // boundary without elevation.
        let process: HANDLE = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;

        let mut buffer = [0u16; MAX_PATH as usize];
        let mut length = buffer.len() as u32;
        let query = QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_FORMAT(0),
            windows::core::PWSTR(buffer.as_mut_ptr()),
            &mut length,
        );
        let _ = CloseHandle(process);
        query.ok()?;

        let path = String::from_utf16_lossy(&buffer[..length as usize]);
        // The file name alone, so a game moving between drives or library
        // folders does not break its own rule.
        path.rsplit(['\\', '/']).next().map(|s| s.to_string())
    }
}

#[cfg(not(windows))]
pub fn foreground_executable() -> Option<String> {
    None
}

/// Decide what to do about the current foreground application.
///
/// Pure, so the deciding is testable without a desktop: the caller supplies
/// what is in front and what is loaded, and gets back the profile to switch to,
/// if any.
pub fn decide(rules: &Rules, foreground: Option<&str>, current_profile: &str) -> Option<String> {
    let wanted = rules.profile_for(foreground?)?;

    // Already there. Re-sending would reload the profile on the device every
    // two seconds for as long as the game is open, which is audible.
    if wanted.eq_ignore_ascii_case(current_profile) {
        return None;
    }
    Some(wanted.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules() -> Rules {
        Rules {
            enabled: true,
            rules: vec![
                Rule {
                    executable: "cs2.exe".into(),
                    profile: "Competitive".into(),
                    enabled: true,
                },
                Rule {
                    executable: "Spotify.exe".into(),
                    profile: "Music".into(),
                    enabled: true,
                },
            ],
        }
    }

    #[test]
    fn a_matching_application_selects_its_profile() {
        assert_eq!(
            decide(&rules(), Some("cs2.exe"), "PC"),
            Some("Competitive".into())
        );
    }

    /// Windows is case-insensitive about file names and so is everyone typing
    /// one into a text box.
    #[test]
    fn matching_ignores_case() {
        assert_eq!(
            decide(&rules(), Some("CS2.EXE"), "PC"),
            Some("Competitive".into())
        );
    }

    /// Re-sending a profile that is already loaded would reload it on the
    /// device every two seconds for as long as the game is open, which is
    /// audible.
    #[test]
    fn the_profile_is_not_reloaded_when_it_is_already_active() {
        assert_eq!(decide(&rules(), Some("cs2.exe"), "Competitive"), None);
        assert_eq!(decide(&rules(), Some("cs2.exe"), "competitive"), None);
    }

    /// Alt-tabbing out of a game must not change how anything sounds. Nothing
    /// reverts; the profile stays where the last rule left it.
    #[test]
    fn leaving_a_matched_application_changes_nothing() {
        assert_eq!(decide(&rules(), Some("explorer.exe"), "Competitive"), None);
        assert_eq!(decide(&rules(), None, "Competitive"), None);
    }

    /// The master switch has to actually stop it. Software that changes your
    /// audio when you told it not to is software you uninstall.
    #[test]
    fn nothing_happens_while_the_watcher_is_off() {
        let mut rules = rules();
        rules.enabled = false;
        assert_eq!(decide(&rules, Some("cs2.exe"), "PC"), None);
    }

    #[test]
    fn a_disabled_rule_is_skipped_but_kept() {
        let mut rules = rules();
        rules.rules[0].enabled = false;
        assert_eq!(decide(&rules, Some("cs2.exe"), "PC"), None);
        assert_eq!(rules.rules.len(), 2, "the rule was deleted, not parked");
    }

    /// First match wins, so the order in the list is the priority.
    #[test]
    fn the_first_matching_rule_wins() {
        let mut rules = rules();
        rules.rules.insert(
            0,
            Rule {
                executable: "cs2.exe".into(),
                profile: "Earlier".into(),
                enabled: true,
            },
        );
        assert_eq!(
            decide(&rules, Some("cs2.exe"), "PC"),
            Some("Earlier".into())
        );
    }

    #[test]
    fn setting_an_existing_rule_replaces_it_rather_than_duplicating() {
        let mut rules = rules();
        rules.set("CS2.exe", "Something Else");
        assert_eq!(rules.rules.len(), 2);
        assert_eq!(
            decide(&rules, Some("cs2.exe"), "PC"),
            Some("Something Else".into())
        );
    }

    #[test]
    fn removing_a_rule_reports_whether_there_was_one() {
        let mut rules = rules();
        assert!(rules.remove("cs2.exe"));
        assert!(!rules.remove("cs2.exe"));
        assert_eq!(rules.rules.len(), 1);
    }

    /// Against the real desktop. Something is always in front of something.
    #[test]
    #[cfg(windows)]
    fn the_foreground_executable_is_a_file_name_not_a_path() {
        let Some(executable) = foreground_executable() else {
            // Legitimate on a headless session; nothing to assert.
            return;
        };
        assert!(
            !executable.contains('\\') && !executable.contains('/'),
            "expected a bare file name, got {executable}"
        );
        assert!(
            executable.to_lowercase().ends_with(".exe"),
            "expected an executable, got {executable}"
        );
    }
}
