//! Whether Windows asks apps to keep quiet: Do Not Disturb (Focus assist), a presentation,
//! or a full-screen game or video.

use std::ffi::c_void;
use std::sync::OnceLock;

use windows::core::{s, w};
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows::Win32::UI::Shell::{
    SHQueryUserNotificationState, QUNS_BUSY, QUNS_PRESENTATION_MODE, QUNS_QUIET_TIME, QUNS_RUNNING_D3D_FULL_SCREEN,
};

/// WNF_SHEL_QUIETHOURS_ACTIVE_PROFILE_CHANGED: the active Do Not Disturb profile, 0 when off.
const WNF_QUIET_HOURS: u64 = 0x0D83_063E_A3BF_1C75;

type QueryWnf = unsafe extern "system" fn(*const u64, *const c_void, *const c_void, *mut u32, *mut c_void, *mut u32) -> i32;

pub fn active() -> bool {
    do_not_disturb() || busy()
}

fn busy() -> bool {
    unsafe {
        SHQueryUserNotificationState().is_ok_and(|s| {
            s == QUNS_BUSY || s == QUNS_RUNNING_D3D_FULL_SCREEN || s == QUNS_PRESENTATION_MODE || s == QUNS_QUIET_TIME
        })
    }
}

/// Reads the Do Not Disturb state Windows publishes for the shell; false if it can't be read.
fn do_not_disturb() -> bool {
    static QUERY: OnceLock<Option<QueryWnf>> = OnceLock::new();
    let query = QUERY.get_or_init(|| unsafe {
        let ntdll = GetModuleHandleW(w!("ntdll.dll")).ok()?;
        let f = GetProcAddress(ntdll, s!("NtQueryWnfStateData"))?;
        Some(std::mem::transmute::<unsafe extern "system" fn() -> isize, QueryWnf>(f))
    });
    let Some(query) = query else { return false };
    let (mut stamp, mut value, mut size) = (0u32, 0u32, 4u32);
    let status = unsafe {
        query(&WNF_QUIET_HOURS, std::ptr::null(), std::ptr::null(), &mut stamp, &mut value as *mut u32 as *mut c_void, &mut size)
    };
    status >= 0 && size >= 4 && value != 0
}
