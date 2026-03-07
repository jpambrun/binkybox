use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::sync::Mutex;
use std::thread;
use std::time::Duration;
use std::time::Instant;

use windows_sys::Win32::{
	Foundation::{BOOL, HWND, LPARAM, POINT, RECT},
	UI::{
		Controls::STATE_SYSTEM_INVISIBLE,
		WindowsAndMessaging::{
			EnumWindows, GetAncestor, GetClassNameW, GetForegroundWindow,
			GetTitleBarInfo, GetWindowLongW, GetWindowTextW, IsWindowVisible,
			SetForegroundWindow, WindowFromPoint, GA_ROOT, GWL_EXSTYLE, TITLEBARINFO,
			WS_EX_TOOLWINDOW,
		},
	},
};

static PRUNE_GRACE_DESKTOP: Mutex<Option<(u32, Instant)>> = Mutex::new(None);
static PREVIOUS_DESKTOP: Mutex<Option<u32>> = Mutex::new(None);

pub(crate) fn adjacent_desktop_target(delta: i32) -> Option<u32> {
	let current_index =
		match winvd::get_current_desktop().and_then(|desktop| desktop.get_index()) {
			Ok(index) => index,
			Err(_) => return None,
		};
	resolve_adjacent_desktop_target(current_index, delta)
}

pub(crate) fn previous_desktop_target() -> Option<u32> {
	let current_index =
		match winvd::get_current_desktop().and_then(|desktop| desktop.get_index()) {
			Ok(index) => index,
			Err(_) => return None,
		};
	match PREVIOUS_DESKTOP.lock() {
		Ok(guard) => resolve_previous_desktop_target(current_index, *guard),
		Err(_) => None,
	}
}

fn resolve_previous_desktop_target(
	current_desktop: u32,
	previous_desktop: Option<u32>,
) -> Option<u32> {
	match previous_desktop {
		Some(previous) if previous != current_desktop => Some(previous),
		_ => None,
	}
}

fn resolve_adjacent_desktop_target(current_desktop: u32, delta: i32) -> Option<u32> {
	let target = current_desktop as i64 + delta as i64;
	if target < 0 {
		return None;
	}
	u32::try_from(target).ok()
}

pub(crate) fn active_window_for_move() -> Option<HWND> {
	unsafe {
		let hwnd = GetForegroundWindow();
		if hwnd.is_null() || !is_normal_window(hwnd) {
			return None;
		}
		return Some(hwnd);
	}
}

pub(crate) fn draggable_window_from_point(x: i32, y: i32) -> Option<HWND> {
	unsafe {
		let hwnd = WindowFromPoint(POINT { x, y });
		if hwnd.is_null() {
			return None;
		}
		let root = GetAncestor(hwnd, GA_ROOT);
		let target = if root.is_null() { hwnd } else { root };
		if target.is_null() || !is_draggable_window(target) {
			return None;
		}
		Some(target)
	}
}

fn focus_moved_window(window: HWND) -> bool {
	for _ in 0..10 {
		if matches!(
			winvd::is_window_on_current_desktop(unsafe { std::mem::transmute(window) }),
			Ok(true)
		) {
			unsafe {
				let _ = SetForegroundWindow(window);
			}
			return true;
		}
		thread::sleep(Duration::from_millis(40));
	}
	false
}

unsafe extern "system" fn enum_windows_and_switch_app_focus(
	hwnd: HWND,
	_lparam: LPARAM,
) -> BOOL {
	if !is_normal_window(hwnd) {
		return 1;
	}

	let is_on_current_desktop =
		winvd::is_window_on_current_desktop(unsafe { std::mem::transmute(hwnd) })
			.unwrap_or(false);
	if !is_on_current_desktop {
		return 1;
	}

	let _ = SetForegroundWindow(hwnd);
	return 0;
}

#[derive(Default)]
struct OccupiedDesktopState {
	highest_index: Option<u32>,
}

unsafe extern "system" fn enum_windows_and_find_highest_occupied_desktop(
	hwnd: HWND,
	lparam: LPARAM,
) -> BOOL {
	if !is_normal_window(hwnd) {
		return 1;
	}
	let state = unsafe { &mut *(lparam as *mut OccupiedDesktopState) };
	if let Ok(index) = winvd::get_desktop_by_window(unsafe { std::mem::transmute(hwnd) })
		.and_then(|desktop| desktop.get_index())
	{
		state.highest_index = Some(match state.highest_index {
			Some(previous) => previous.max(index),
			None => index,
		});
	}
	return 1;
}

fn highest_occupied_desktop_index() -> Option<u32> {
	let mut state = OccupiedDesktopState::default();
	unsafe {
		let _ = EnumWindows(
			Some(enum_windows_and_find_highest_occupied_desktop),
			(&mut state as *mut OccupiedDesktopState) as isize,
		);
	}
	state.highest_index
}

fn protected_desktop_index() -> Option<u32> {
	const PRUNE_GRACE: Duration = Duration::from_secs(5);
	let lock = PRUNE_GRACE_DESKTOP.lock();
	if lock.is_err() {
		return None;
	}
	let mut guard = lock.unwrap();
	match *guard {
		Some((desktop, at)) if at.elapsed() < PRUNE_GRACE => Some(desktop),
		Some(_) => {
			*guard = None;
			None
		}
		None => None,
	}
}

fn is_normal_window(hwnd: HWND) -> bool {
	unsafe {
		if IsWindowVisible(hwnd) == 0 {
			return false;
		}
		let mut class_name_buffer: [u16; 256] = [0; 256];
		let class_len = GetClassNameW(
			hwnd,
			class_name_buffer.as_mut_ptr(),
			class_name_buffer.len() as i32,
		);
		if class_len > 0 {
			let class_name =
				OsString::from_wide(&class_name_buffer[..class_len as usize])
					.to_string_lossy()
					.into_owned();
			if class_name == "Progman"
				|| class_name == "WorkerW"
				|| class_name == "Shell_TrayWnd"
				|| class_name == "Windows.UI.Core.CoreWindow"
			{
				return false;
			}
		}
		let mut ti = TITLEBARINFO {
			cbSize: std::mem::size_of::<TITLEBARINFO>() as u32,
			rcTitleBar: RECT {
				left: 0,
				top: 0,
				right: 0,
				bottom: 0,
			},
			rgstate: [0; 6],
		};
		let _ = GetTitleBarInfo(hwnd, &mut ti);
		if ti.rgstate[0] & STATE_SYSTEM_INVISIBLE as u32 > 0 {
			return false;
		}
		if ((GetWindowLongW(hwnd, GWL_EXSTYLE) as u32) & WS_EX_TOOLWINDOW as u32) != 0 {
			return false;
		}
		let mut buffer: [u16; 256] = [0; 256];
		let title_len = GetWindowTextW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32);
		let window_title = OsString::from_wide(&buffer[..title_len as usize])
			.to_string_lossy()
			.into_owned();
		if window_title.is_empty()
			|| window_title.contains("Settings")
			|| window_title == "Program Manager"
		{
			return false;
		}
	}
	return true;
}

fn is_draggable_window(hwnd: HWND) -> bool {
	unsafe {
		if IsWindowVisible(hwnd) == 0 {
			return false;
		}
		let mut class_name_buffer: [u16; 256] = [0; 256];
		let class_len = GetClassNameW(
			hwnd,
			class_name_buffer.as_mut_ptr(),
			class_name_buffer.len() as i32,
		);
		if class_len > 0 {
			let class_name =
				OsString::from_wide(&class_name_buffer[..class_len as usize])
					.to_string_lossy()
					.into_owned();
			if class_name == "Progman"
				|| class_name == "WorkerW"
				|| class_name == "Shell_TrayWnd"
				|| class_name == "Windows.UI.Core.CoreWindow"
			{
				return false;
			}
		}
		if ((GetWindowLongW(hwnd, GWL_EXSTYLE) as u32) & WS_EX_TOOLWINDOW as u32) != 0 {
			return false;
		}
	}
	true
}

fn remove_tail_desktops_if_possible() {
	const REMOVE_RETRIES: u8 = 6;
	let highest_occupied = highest_occupied_desktop_index().unwrap_or(0);
	let protected_desktop = protected_desktop_index().unwrap_or(0);
	let retain_until = highest_occupied.max(protected_desktop);
	loop {
		let desktop_count = match winvd::get_desktop_count() {
			Ok(count) => count,
			Err(_) => return,
		};
		if desktop_count <= 1 || desktop_count - 1 <= retain_until {
			return;
		}
		let current_index = match winvd::get_current_desktop().and_then(|d| d.get_index())
		{
			Ok(index) => index,
			Err(_) => return,
		};
		let tail_index = desktop_count - 1;
		let fallback_index = tail_index - 1;
		if current_index > retain_until {
			if winvd::switch_desktop(retain_until).is_err() {
				return;
			}
			thread::sleep(Duration::from_millis(80));
			continue;
		}
		let mut removed = false;
		for _ in 0..REMOVE_RETRIES {
			if winvd::remove_desktop(tail_index, fallback_index).is_ok() {
				removed = true;
				break;
			}
			thread::sleep(Duration::from_millis(60));
		}
		if !removed {
			return;
		}
		thread::sleep(Duration::from_millis(40));
	}
}

pub(crate) fn switch_to_desktop(desktop: u32, tries: u8, moved_window: Option<HWND>) {
	if tries <= 10 {
		let source_desktop = winvd::get_current_desktop()
			.and_then(|d| d.get_index())
			.ok();
		match winvd::switch_desktop(desktop) {
			Ok(_) => {
				if let Some(source) = source_desktop {
					if source != desktop {
						if let Ok(mut guard) = PREVIOUS_DESKTOP.lock() {
							*guard = Some(source);
						}
					}
				}
				if let Ok(mut guard) = PRUNE_GRACE_DESKTOP.lock() {
					*guard = Some((desktop, Instant::now()));
				}
				let moved_and_focused = moved_window.is_some_and(|window| {
					let winvd_window = unsafe { std::mem::transmute(window) };
					if winvd::move_window_to_desktop(desktop, &winvd_window).is_ok() {
						return focus_moved_window(window);
					}
					false
				});
				if !moved_and_focused {
					unsafe {
						let _ = EnumWindows(Some(enum_windows_and_switch_app_focus), 0);
					}
				}
				remove_tail_desktops_if_possible();
			}
			Err(_) => match winvd::create_desktop() {
				Ok(_) => {
					switch_to_desktop(desktop, tries + 1, moved_window);
				}
				Err(e) => {
					println!("Error: {:?}", e);
				}
			},
		}
	}
}

#[cfg(test)]
mod tests {
	use super::{resolve_adjacent_desktop_target, resolve_previous_desktop_target};

	#[test]
	fn previous_desktop_target_is_none_without_previous() {
		assert_eq!(resolve_previous_desktop_target(2, None), None);
	}

	#[test]
	fn previous_desktop_target_is_none_when_previous_matches_current() {
		assert_eq!(resolve_previous_desktop_target(2, Some(2)), None);
	}

	#[test]
	fn previous_desktop_target_returns_previous_when_distinct() {
		assert_eq!(resolve_previous_desktop_target(2, Some(1)), Some(1));
	}

	#[test]
	fn adjacent_desktop_target_has_no_previous_before_zero() {
		assert_eq!(resolve_adjacent_desktop_target(0, -1), None);
	}

	#[test]
	fn adjacent_desktop_target_can_move_forward_from_zero() {
		assert_eq!(resolve_adjacent_desktop_target(0, 1), Some(1));
	}

	#[test]
	fn adjacent_desktop_target_can_move_back_from_nonzero() {
		assert_eq!(resolve_adjacent_desktop_target(3, -1), Some(2));
	}
}
