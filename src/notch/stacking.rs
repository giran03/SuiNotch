//! Recover the notch's place above application windows without taking keyboard focus.
use super::*;
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
use windows::Win32::System::Threading::GetCurrentProcessId;

pub(super) fn ensure_topmost(app: &App) {
    maintain(app.notch.hwnd, app.notch.hidden, unsafe { GetCurrentProcessId() }, false);
}

pub(super) fn raise_for_interaction(app: &App) {
    maintain(app.notch.hwnd, app.notch.hidden, unsafe { GetCurrentProcessId() }, true);
}

fn process(hwnd: HWND) -> u32 {
    let mut pid = 0;
    unsafe {
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
    }
    pid
}

fn maintain(hwnd: HWND, hidden: bool, own_process: u32, force: bool) -> bool {
    unsafe {
        if hidden || !IsWindowVisible(hwnd).as_bool() {
            return false;
        }
        // Our settings, file pickers and menus keep their normal dialog priority.
        let foreground = GetForegroundWindow();
        if foreground != hwnd && process(foreground) == own_process {
            return false;
        }
        let topmost = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST.0 != 0;
        if !force && topmost && !covered(hwnd, own_process) {
            return false;
        }
        // No move, resize, activation or show: full-screen hiding and text focus are unaffected.
        SetWindowPos(hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER).is_ok()
    }
}

fn covered(hwnd: HWND, own_process: u32) -> bool {
    unsafe {
        let mut notch = RECT::default();
        if GetWindowRect(hwnd, &mut notch).is_err() {
            return false;
        }
        let mut current = hwnd;
        // Bounded because the desktop's window list can change while walking it.
        for _ in 0..256 {
            let Ok(above) = GetWindow(current, GW_HWNDPREV) else { break };
            if above.0.is_null() || above == hwnd {
                break;
            }
            current = above;
            if !IsWindowVisible(above).as_bool() || IsIconic(above).as_bool() || process(above) == own_process {
                continue;
            }
            let mut rect = RECT::default();
            if GetWindowRect(above, &mut rect).is_err() || !overlaps(&notch, &rect) {
                continue;
            }
            let ex = GetWindowLongW(above, GWL_EXSTYLE) as u32;
            if ex & WS_EX_TRANSPARENT.0 != 0 {
                continue;
            }
            let mut cloaked = 0u32;
            let _ = DwmGetWindowAttribute(above, DWMWA_CLOAKED, &mut cloaked as *mut _ as *mut _, 4);
            if cloaked != 0 {
                continue;
            }
            let mut name = [0u16; 80];
            let n = GetClassNameW(above, &mut name);
            let class = String::from_utf16_lossy(&name[..n.max(0) as usize]);
            // Do not compete with the Start menu, task switcher, taskbar or popup menus.
            if [
                "#32768",
                "Shell_TrayWnd",
                "Shell_SecondaryTrayWnd",
                "MultitaskingViewFrame",
                "TaskSwitcherWnd",
                "XamlExplorerHostIslandWindow",
                "ForegroundStaging",
            ]
            .contains(&class.as_str())
            {
                continue;
            }
            return true;
        }
    }
    false
}

fn overlaps(a: &RECT, b: &RECT) -> bool {
    a.left < b.right && b.left < a.right && a.top < b.bottom && b.top < a.bottom
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "native window stacking; creates only off-screen, non-activating test windows"]
    fn recovers_topmost_and_occlusion_without_moving_showing_or_taking_focus() {
        unsafe {
            let create = || {
                CreateWindowExW(
                    WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                    w!("STATIC"),
                    w!("SuiNotch stacking test"),
                    WS_POPUP,
                    -32000,
                    -32000,
                    160,
                    40,
                    None,
                    None,
                    None,
                    None,
                )
                .unwrap()
            };
            let notch = create();
            let cover = create();
            let foreground = GetForegroundWindow();
            // No real app or desktop input is involved. Treat the second test window as a foreign app.
            let own_process = u32::MAX;
            assert!(!maintain(notch, false, own_process, false)); // invisible stays invisible
            SetWindowPos(notch, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW).unwrap();
            SetWindowPos(notch, Some(HWND_NOTOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE).unwrap();
            let mut before = RECT::default();
            GetWindowRect(notch, &mut before).unwrap();
            assert!(maintain(notch, false, own_process, false)); // recover a lost WS_EX_TOPMOST
            assert_ne!(GetWindowLongW(notch, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST.0, 0);
            SetWindowPos(cover, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW).unwrap();
            assert!(covered(notch, own_process));
            assert!(maintain(notch, false, own_process, false)); // another topmost window was raised later
            assert!(!covered(notch, own_process));
            assert!(!maintain(notch, false, own_process, false)); // no redundant restacking once clear
            assert!(maintain(notch, false, own_process, true)); // expansion proactively raises before growing
            let mut after = RECT::default();
            GetWindowRect(notch, &mut after).unwrap();
            assert_eq!((before.left, before.top, before.right, before.bottom), (after.left, after.top, after.right, after.bottom));
            assert_eq!(GetForegroundWindow(), foreground);
            SetWindowPos(cover, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE).unwrap();
            assert!(!covered(notch, GetCurrentProcessId())); // our own dialogs are exempt
            assert!(!maintain(notch, true, own_process, false)); // fullscreen policy prevents recovery
            assert!(!maintain(notch, true, own_process, true));
            let _ = ShowWindow(notch, SW_HIDE);
            assert!(!maintain(notch, false, own_process, false));
            assert!(!IsWindowVisible(notch).as_bool());
            DestroyWindow(cover).unwrap();
            DestroyWindow(notch).unwrap();
        }
    }
}
