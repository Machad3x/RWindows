//! Session power actions for the Start menu, all from documented Win32.
//! Restart and shutdown need `SeShutdownPrivilege`, enabled on our own token.

use windows::Win32::Foundation::{CloseHandle, ERROR_NOT_ALL_ASSIGNED, GetLastError, HANDLE, LUID};
use windows::Win32::Security::{
    AdjustTokenPrivileges, LUID_AND_ATTRIBUTES, LookupPrivilegeValueW, SE_PRIVILEGE_ENABLED,
    SE_SHUTDOWN_NAME, TOKEN_ADJUST_PRIVILEGES, TOKEN_PRIVILEGES, TOKEN_QUERY,
};
use windows::Win32::System::Power::SetSystemPowerState;
use windows::Win32::System::Shutdown::{
    EWX_FORCEIFHUNG, EWX_LOGOFF, EWX_REBOOT, EWX_SHUTDOWN, ExitWindowsEx, LockWorkStation,
};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows::core::Result;

/// An action from the Start power button.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PowerAction {
    /// Lock the workstation; needs no privilege.
    Lock,
    /// Sign out the current user.
    Logoff,
    /// Sleep (suspend to RAM).
    Sleep,
    /// Restart the machine.
    Restart,
    /// Shut the machine down.
    Shutdown,
}

/// Enables `SeShutdownPrivilege` on the current process token.
fn enable_shutdown_privilege() -> Result<()> {
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY, &mut token)?;
        let mut luid = LUID::default();
        LookupPrivilegeValueW(None, SE_SHUTDOWN_NAME, &mut luid)?;
        let privileges = TOKEN_PRIVILEGES {
            PrivilegeCount: 1,
            Privileges: [LUID_AND_ATTRIBUTES { Luid: luid, Attributes: SE_PRIVILEGE_ENABLED }],
        };
        let result = AdjustTokenPrivileges(token, false, Some(&privileges), 0, None, None);
        // AdjustTokenPrivileges succeeds even when the privilege was not
        // assigned; the failure only shows up as this last error.
        let assigned = GetLastError() != ERROR_NOT_ALL_ASSIGNED;
        _ = CloseHandle(token);
        result?;
        if !assigned {
            return Err(ERROR_NOT_ALL_ASSIGNED.into());
        }
        Ok(())
    }
}

/// Performs `action`. The caller must confirm destructive actions first.
pub fn perform(action: PowerAction) -> Result<()> {
    unsafe {
        match action {
            PowerAction::Lock => LockWorkStation(),
            PowerAction::Logoff => ExitWindowsEx(EWX_LOGOFF | EWX_FORCEIFHUNG, Default::default()),
            PowerAction::Sleep => SetSystemPowerState(true, false),
            PowerAction::Restart => {
                enable_shutdown_privilege()?;
                ExitWindowsEx(EWX_REBOOT | EWX_FORCEIFHUNG, Default::default())
            }
            PowerAction::Shutdown => {
                enable_shutdown_privilege()?;
                ExitWindowsEx(EWX_SHUTDOWN | EWX_FORCEIFHUNG, Default::default())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn power_actions_are_distinct() {
        let actions = [PowerAction::Lock, PowerAction::Logoff, PowerAction::Sleep,
            PowerAction::Restart, PowerAction::Shutdown];
        for (index, action) in actions.iter().enumerate() {
            assert_ne!(*action, actions[(index + 1) % actions.len()]);
        }
    }
}
