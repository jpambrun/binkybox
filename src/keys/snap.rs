use windows_sys::Win32::{
	Foundation::{HWND, RECT},
	Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS},
	Graphics::Gdi::{
		GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
	},
	UI::WindowsAndMessaging::{
		GetWindowRect, IsIconic, IsZoomed, MoveWindow, ShowWindow, SW_MAXIMIZE,
		SW_RESTORE,
	},
};

use super::desktop::active_window_for_move;

const SNAP_TOLERANCE_PX: i32 = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SnapDirection {
	Left,
	Right,
	Up,
	Down,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SnapState {
	Other,
	Maximized,
	LeftHalf,
	RightHalf,
	TopHalf,
	BottomHalf,
	TopLeft,
	TopRight,
	BottomLeft,
	BottomRight,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SnapRect {
	left: i32,
	top: i32,
	right: i32,
	bottom: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TargetWindowState {
	Rect(SnapRect),
	Maximized,
}

impl SnapRect {
	fn width(self) -> i32 {
		self.right - self.left
	}

	fn height(self) -> i32 {
		self.bottom - self.top
	}
}

pub(crate) fn snap_active_window(direction: SnapDirection) -> bool {
	let hwnd = match active_window_for_move() {
		Some(hwnd) => hwnd,
		None => return false,
	};

	let was_maximized = is_maximized(hwnd);
	if was_maximized && matches!(direction, SnapDirection::Up) {
		return true;
	}
	restore_window_if_needed(hwnd);

	let work_area = match monitor_work_area(hwnd) {
		Some(rect) => rect,
		None => return false,
	};
	let outer_rect = match window_rect(hwnd) {
		Some(rect) => rect,
		None => return false,
	};
	let visible_rect = visible_window_rect(hwnd).unwrap_or(outer_rect);
	let state = if was_maximized {
		SnapState::Maximized
	} else {
		classify_rect(visible_rect, work_area)
	};

	match target_state_for_direction(state, direction, work_area) {
		TargetWindowState::Maximized => unsafe { ShowWindow(hwnd, SW_MAXIMIZE) != 0 },
		TargetWindowState::Rect(target_visible_rect) => {
			if rect_matches(visible_rect, target_visible_rect, SNAP_TOLERANCE_PX) {
				return true;
			}

			let frame_insets = frame_insets(outer_rect, visible_rect);
			let target_outer_rect =
				outer_rect_for_visible_target(target_visible_rect, frame_insets);

			unsafe {
				MoveWindow(
					hwnd,
					target_outer_rect.left,
					target_outer_rect.top,
					target_outer_rect.width(),
					target_outer_rect.height(),
					1,
				) != 0
			}
		}
	}
}

fn restore_window_if_needed(hwnd: HWND) {
	unsafe {
		if IsIconic(hwnd) != 0 || IsZoomed(hwnd) != 0 {
			let _ = ShowWindow(hwnd, SW_RESTORE);
		}
	}
}

fn is_maximized(hwnd: HWND) -> bool {
	unsafe { IsZoomed(hwnd) != 0 }
}

fn monitor_work_area(hwnd: HWND) -> Option<SnapRect> {
	unsafe {
		let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
		if monitor.is_null() {
			return None;
		}
		let mut info = MONITORINFO {
			cbSize: std::mem::size_of::<MONITORINFO>() as u32,
			rcMonitor: RECT {
				left: 0,
				top: 0,
				right: 0,
				bottom: 0,
			},
			rcWork: RECT {
				left: 0,
				top: 0,
				right: 0,
				bottom: 0,
			},
			dwFlags: 0,
		};
		if GetMonitorInfoW(monitor, &mut info) == 0 {
			return None;
		}
		Some(SnapRect::from(info.rcWork))
	}
}

fn window_rect(hwnd: HWND) -> Option<SnapRect> {
	let mut rect = RECT {
		left: 0,
		top: 0,
		right: 0,
		bottom: 0,
	};
	unsafe {
		if GetWindowRect(hwnd, &mut rect) == 0 {
			return None;
		}
	}
	Some(SnapRect::from(rect))
}

fn visible_window_rect(hwnd: HWND) -> Option<SnapRect> {
	let mut rect = RECT {
		left: 0,
		top: 0,
		right: 0,
		bottom: 0,
	};
	unsafe {
		let hr = DwmGetWindowAttribute(
			hwnd,
			DWMWA_EXTENDED_FRAME_BOUNDS as u32,
			(&mut rect as *mut RECT).cast(),
			std::mem::size_of::<RECT>() as u32,
		);
		if hr < 0 {
			return None;
		}
	}
	Some(SnapRect::from(rect))
}

fn classify_rect(rect: SnapRect, work_area: SnapRect) -> SnapState {
	let layout = SnapLayout::new(work_area);
	for (candidate, state) in [
		(layout.top_left, SnapState::TopLeft),
		(layout.top_right, SnapState::TopRight),
		(layout.bottom_left, SnapState::BottomLeft),
		(layout.bottom_right, SnapState::BottomRight),
		(layout.left_half, SnapState::LeftHalf),
		(layout.right_half, SnapState::RightHalf),
		(layout.top_half, SnapState::TopHalf),
		(layout.bottom_half, SnapState::BottomHalf),
	] {
		if rect_matches(rect, candidate, SNAP_TOLERANCE_PX) {
			return state;
		}
	}
	SnapState::Other
}

fn target_state_for_direction(
	state: SnapState,
	direction: SnapDirection,
	work_area: SnapRect,
) -> TargetWindowState {
	let layout = SnapLayout::new(work_area);
	match direction {
		SnapDirection::Left => TargetWindowState::Rect(match state {
			SnapState::Maximized => layout.left_half,
			SnapState::TopRight => layout.top_left,
			SnapState::BottomRight => layout.bottom_left,
			SnapState::TopHalf => layout.top_left,
			SnapState::BottomHalf => layout.bottom_left,
			SnapState::TopLeft | SnapState::BottomLeft => layout.left_half,
			_ => layout.left_half,
		}),
		SnapDirection::Right => TargetWindowState::Rect(match state {
			SnapState::Maximized => layout.right_half,
			SnapState::TopLeft => layout.top_right,
			SnapState::BottomLeft => layout.bottom_right,
			SnapState::TopHalf => layout.top_right,
			SnapState::BottomHalf => layout.bottom_right,
			SnapState::TopRight | SnapState::BottomRight => layout.right_half,
			_ => layout.right_half,
		}),
		SnapDirection::Up => match state {
			SnapState::TopHalf => TargetWindowState::Maximized,
			SnapState::Maximized => TargetWindowState::Maximized,
			_ => TargetWindowState::Rect(match state {
				SnapState::BottomLeft => layout.top_left,
				SnapState::BottomRight => layout.top_right,
				SnapState::LeftHalf => layout.top_left,
				SnapState::RightHalf => layout.top_right,
				SnapState::TopLeft | SnapState::TopRight => layout.top_half,
				_ => layout.top_half,
			}),
		},
		SnapDirection::Down => TargetWindowState::Rect(match state {
			SnapState::Maximized => layout.top_half,
			SnapState::TopLeft => layout.bottom_left,
			SnapState::TopRight => layout.bottom_right,
			SnapState::LeftHalf => layout.bottom_left,
			SnapState::RightHalf => layout.bottom_right,
			SnapState::BottomLeft | SnapState::BottomRight => layout.bottom_half,
			_ => layout.bottom_half,
		}),
	}
}

fn rect_matches(actual: SnapRect, expected: SnapRect, tolerance: i32) -> bool {
	(actual.left - expected.left).abs() <= tolerance
		&& (actual.top - expected.top).abs() <= tolerance
		&& (actual.right - expected.right).abs() <= tolerance
		&& (actual.bottom - expected.bottom).abs() <= tolerance
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FrameInsets {
	left: i32,
	top: i32,
	right: i32,
	bottom: i32,
}

fn frame_insets(outer_rect: SnapRect, visible_rect: SnapRect) -> FrameInsets {
	FrameInsets {
		left: visible_rect.left - outer_rect.left,
		top: visible_rect.top - outer_rect.top,
		right: outer_rect.right - visible_rect.right,
		bottom: outer_rect.bottom - visible_rect.bottom,
	}
}

fn outer_rect_for_visible_target(
	target_visible_rect: SnapRect,
	frame_insets: FrameInsets,
) -> SnapRect {
	SnapRect {
		left: target_visible_rect.left - frame_insets.left,
		top: target_visible_rect.top - frame_insets.top,
		right: target_visible_rect.right + frame_insets.right,
		bottom: target_visible_rect.bottom + frame_insets.bottom,
	}
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SnapLayout {
	left_half: SnapRect,
	right_half: SnapRect,
	top_half: SnapRect,
	bottom_half: SnapRect,
	top_left: SnapRect,
	top_right: SnapRect,
	bottom_left: SnapRect,
	bottom_right: SnapRect,
}

impl SnapLayout {
	fn new(work_area: SnapRect) -> Self {
		let mid_x = work_area.left + work_area.width() / 2;
		let mid_y = work_area.top + work_area.height() / 2;

		let left_half = SnapRect {
			left: work_area.left,
			top: work_area.top,
			right: mid_x,
			bottom: work_area.bottom,
		};
		let right_half = SnapRect {
			left: mid_x,
			top: work_area.top,
			right: work_area.right,
			bottom: work_area.bottom,
		};
		let top_half = SnapRect {
			left: work_area.left,
			top: work_area.top,
			right: work_area.right,
			bottom: mid_y,
		};
		let bottom_half = SnapRect {
			left: work_area.left,
			top: mid_y,
			right: work_area.right,
			bottom: work_area.bottom,
		};

		Self {
			left_half,
			right_half,
			top_half,
			bottom_half,
			top_left: SnapRect {
				left: left_half.left,
				top: top_half.top,
				right: left_half.right,
				bottom: top_half.bottom,
			},
			top_right: SnapRect {
				left: right_half.left,
				top: top_half.top,
				right: right_half.right,
				bottom: top_half.bottom,
			},
			bottom_left: SnapRect {
				left: left_half.left,
				top: bottom_half.top,
				right: left_half.right,
				bottom: bottom_half.bottom,
			},
			bottom_right: SnapRect {
				left: right_half.left,
				top: bottom_half.top,
				right: right_half.right,
				bottom: bottom_half.bottom,
			},
		}
	}
}

impl From<RECT> for SnapRect {
	fn from(rect: RECT) -> Self {
		Self {
			left: rect.left,
			top: rect.top,
			right: rect.right,
			bottom: rect.bottom,
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn rect(left: i32, top: i32, right: i32, bottom: i32) -> SnapRect {
		SnapRect {
			left,
			top,
			right,
			bottom,
		}
	}

	#[test]
	fn layout_splits_odd_work_area_with_remainder_on_right_and_bottom() {
		let layout = SnapLayout::new(rect(0, 0, 101, 81));
		assert_eq!(layout.left_half, rect(0, 0, 50, 81));
		assert_eq!(layout.right_half, rect(50, 0, 101, 81));
		assert_eq!(layout.top_half, rect(0, 0, 101, 40));
		assert_eq!(layout.bottom_half, rect(0, 40, 101, 81));
	}

	#[test]
	fn classify_half_states() {
		let work_area = rect(0, 0, 100, 80);
		let layout = SnapLayout::new(work_area);
		assert_eq!(
			classify_rect(layout.left_half, work_area),
			SnapState::LeftHalf
		);
		assert_eq!(
			classify_rect(layout.right_half, work_area),
			SnapState::RightHalf
		);
		assert_eq!(
			classify_rect(layout.top_half, work_area),
			SnapState::TopHalf
		);
		assert_eq!(
			classify_rect(layout.bottom_half, work_area),
			SnapState::BottomHalf
		);
	}

	#[test]
	fn classify_quadrant_states() {
		let work_area = rect(0, 0, 100, 80);
		let layout = SnapLayout::new(work_area);
		assert_eq!(
			classify_rect(layout.top_left, work_area),
			SnapState::TopLeft
		);
		assert_eq!(
			classify_rect(layout.top_right, work_area),
			SnapState::TopRight
		);
		assert_eq!(
			classify_rect(layout.bottom_left, work_area),
			SnapState::BottomLeft
		);
		assert_eq!(
			classify_rect(layout.bottom_right, work_area),
			SnapState::BottomRight
		);
	}

	#[test]
	fn classify_uses_tolerance_for_nearby_rects() {
		let work_area = rect(0, 0, 100, 80);
		let layout = SnapLayout::new(work_area);
		let off_by_a_bit = rect(
			layout.top_left.left + 3,
			layout.top_left.top + 2,
			layout.top_left.right - 4,
			layout.top_left.bottom + 5,
		);
		assert_eq!(classify_rect(off_by_a_bit, work_area), SnapState::TopLeft);
	}

	#[test]
	fn classify_prefers_quadrants_before_halves() {
		let work_area = rect(0, 0, 100, 80);
		let layout = SnapLayout::new(work_area);
		assert_eq!(
			classify_rect(layout.top_left, work_area),
			SnapState::TopLeft
		);
	}

	#[test]
	fn frame_insets_capture_invisible_borders() {
		let outer = rect(-8, 0, 108, 88);
		let visible = rect(0, 0, 100, 80);
		assert_eq!(
			frame_insets(outer, visible),
			FrameInsets {
				left: 8,
				top: 0,
				right: 8,
				bottom: 8,
			}
		);
	}

	#[test]
	fn target_outer_rect_expands_visible_target_by_frame_insets() {
		let target_visible = rect(0, 0, 100, 80);
		assert_eq!(
			outer_rect_for_visible_target(
				target_visible,
				FrameInsets {
					left: 8,
					top: 0,
					right: 8,
					bottom: 8,
				}
			),
			rect(-8, 0, 108, 88)
		);
	}

	#[test]
	fn transitions_from_other_go_to_requested_half() {
		let work_area = rect(0, 0, 100, 80);
		let layout = SnapLayout::new(work_area);
		assert_eq!(
			target_state_for_direction(SnapState::Other, SnapDirection::Left, work_area),
			TargetWindowState::Rect(layout.left_half)
		);
		assert_eq!(
			target_state_for_direction(SnapState::Other, SnapDirection::Right, work_area),
			TargetWindowState::Rect(layout.right_half)
		);
		assert_eq!(
			target_state_for_direction(SnapState::Other, SnapDirection::Up, work_area),
			TargetWindowState::Rect(layout.top_half)
		);
		assert_eq!(
			target_state_for_direction(SnapState::Other, SnapDirection::Down, work_area),
			TargetWindowState::Rect(layout.bottom_half)
		);
	}

	#[test]
	fn horizontal_transitions_expand_and_contract_between_half_and_quadrant() {
		let work_area = rect(0, 0, 100, 80);
		let layout = SnapLayout::new(work_area);
		assert_eq!(
			target_state_for_direction(SnapState::TopHalf, SnapDirection::Left, work_area),
			TargetWindowState::Rect(layout.top_left)
		);
		assert_eq!(
			target_state_for_direction(
				SnapState::BottomHalf,
				SnapDirection::Right,
				work_area
			),
			TargetWindowState::Rect(layout.bottom_right)
		);
		assert_eq!(
			target_state_for_direction(SnapState::TopLeft, SnapDirection::Left, work_area),
			TargetWindowState::Rect(layout.left_half)
		);
		assert_eq!(
			target_state_for_direction(
				SnapState::BottomRight,
				SnapDirection::Right,
				work_area
			),
			TargetWindowState::Rect(layout.right_half)
		);
		assert_eq!(
			target_state_for_direction(
				SnapState::TopRight,
				SnapDirection::Left,
				work_area
			),
			TargetWindowState::Rect(layout.top_left)
		);
		assert_eq!(
			target_state_for_direction(
				SnapState::BottomLeft,
				SnapDirection::Right,
				work_area
			),
			TargetWindowState::Rect(layout.bottom_right)
		);
	}

	#[test]
	fn vertical_transitions_expand_and_contract_between_half_and_quadrant() {
		let work_area = rect(0, 0, 100, 80);
		let layout = SnapLayout::new(work_area);
		assert_eq!(
			target_state_for_direction(SnapState::LeftHalf, SnapDirection::Up, work_area),
			TargetWindowState::Rect(layout.top_left)
		);
		assert_eq!(
			target_state_for_direction(
				SnapState::RightHalf,
				SnapDirection::Down,
				work_area
			),
			TargetWindowState::Rect(layout.bottom_right)
		);
		assert_eq!(
			target_state_for_direction(SnapState::TopLeft, SnapDirection::Up, work_area),
			TargetWindowState::Rect(layout.top_half)
		);
		assert_eq!(
			target_state_for_direction(
				SnapState::BottomRight,
				SnapDirection::Down,
				work_area
			),
			TargetWindowState::Rect(layout.bottom_half)
		);
		assert_eq!(
			target_state_for_direction(
				SnapState::TopRight,
				SnapDirection::Down,
				work_area
			),
			TargetWindowState::Rect(layout.bottom_right)
		);
		assert_eq!(
			target_state_for_direction(
				SnapState::BottomLeft,
				SnapDirection::Up,
				work_area
			),
			TargetWindowState::Rect(layout.top_left)
		);
	}

	#[test]
	fn up_from_top_half_maximizes() {
		let work_area = rect(0, 0, 100, 80);
		assert_eq!(
			target_state_for_direction(SnapState::TopHalf, SnapDirection::Up, work_area),
			TargetWindowState::Maximized
		);
	}

	#[test]
	fn maximized_transitions_back_into_snap_layout() {
		let work_area = rect(0, 0, 100, 80);
		let layout = SnapLayout::new(work_area);
		assert_eq!(
			target_state_for_direction(SnapState::Maximized, SnapDirection::Left, work_area),
			TargetWindowState::Rect(layout.left_half)
		);
		assert_eq!(
			target_state_for_direction(SnapState::Maximized, SnapDirection::Right, work_area),
			TargetWindowState::Rect(layout.right_half)
		);
		assert_eq!(
			target_state_for_direction(SnapState::Maximized, SnapDirection::Up, work_area),
			TargetWindowState::Rect(layout.top_half)
		);
		assert_eq!(
			target_state_for_direction(SnapState::Maximized, SnapDirection::Down, work_area),
			TargetWindowState::Rect(layout.top_half)
		);
	}
}
