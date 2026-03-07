use windows_sys::Win32::{
	Foundation::{HWND, RECT},
	Graphics::Gdi::{
		GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
	},
	UI::WindowsAndMessaging::{
		GetWindowRect, IsIconic, IsZoomed, MoveWindow, ShowWindow, SW_RESTORE,
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

	restore_window_if_needed(hwnd);

	let work_area = match monitor_work_area(hwnd) {
		Some(rect) => rect,
		None => return false,
	};
	let current = match window_rect(hwnd) {
		Some(rect) => rect,
		None => return false,
	};
	let target = target_rect_for_direction(
		classify_rect(current, work_area),
		direction,
		work_area,
	);

	if current == target {
		return true;
	}

	unsafe {
		MoveWindow(
			hwnd,
			target.left,
			target.top,
			target.width(),
			target.height(),
			1,
		) != 0
	}
}

fn restore_window_if_needed(hwnd: HWND) {
	unsafe {
		if IsIconic(hwnd) != 0 || IsZoomed(hwnd) != 0 {
			let _ = ShowWindow(hwnd, SW_RESTORE);
		}
	}
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

fn target_rect_for_direction(
	state: SnapState,
	direction: SnapDirection,
	work_area: SnapRect,
) -> SnapRect {
	let layout = SnapLayout::new(work_area);
	match direction {
		SnapDirection::Left => match state {
			SnapState::TopHalf => layout.top_left,
			SnapState::BottomHalf => layout.bottom_left,
			SnapState::TopLeft | SnapState::BottomLeft => layout.left_half,
			_ => layout.left_half,
		},
		SnapDirection::Right => match state {
			SnapState::TopHalf => layout.top_right,
			SnapState::BottomHalf => layout.bottom_right,
			SnapState::TopRight | SnapState::BottomRight => layout.right_half,
			_ => layout.right_half,
		},
		SnapDirection::Up => match state {
			SnapState::LeftHalf => layout.top_left,
			SnapState::RightHalf => layout.top_right,
			SnapState::TopLeft | SnapState::TopRight => layout.top_half,
			_ => layout.top_half,
		},
		SnapDirection::Down => match state {
			SnapState::LeftHalf => layout.bottom_left,
			SnapState::RightHalf => layout.bottom_right,
			SnapState::BottomLeft | SnapState::BottomRight => layout.bottom_half,
			_ => layout.bottom_half,
		},
	}
}

fn rect_matches(actual: SnapRect, expected: SnapRect, tolerance: i32) -> bool {
	(actual.left - expected.left).abs() <= tolerance
		&& (actual.top - expected.top).abs() <= tolerance
		&& (actual.right - expected.right).abs() <= tolerance
		&& (actual.bottom - expected.bottom).abs() <= tolerance
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
	fn transitions_from_other_go_to_requested_half() {
		let work_area = rect(0, 0, 100, 80);
		let layout = SnapLayout::new(work_area);
		assert_eq!(
			target_rect_for_direction(SnapState::Other, SnapDirection::Left, work_area),
			layout.left_half
		);
		assert_eq!(
			target_rect_for_direction(SnapState::Other, SnapDirection::Right, work_area),
			layout.right_half
		);
		assert_eq!(
			target_rect_for_direction(SnapState::Other, SnapDirection::Up, work_area),
			layout.top_half
		);
		assert_eq!(
			target_rect_for_direction(SnapState::Other, SnapDirection::Down, work_area),
			layout.bottom_half
		);
	}

	#[test]
	fn horizontal_transitions_expand_and_contract_between_half_and_quadrant() {
		let work_area = rect(0, 0, 100, 80);
		let layout = SnapLayout::new(work_area);
		assert_eq!(
			target_rect_for_direction(SnapState::TopHalf, SnapDirection::Left, work_area),
			layout.top_left
		);
		assert_eq!(
			target_rect_for_direction(
				SnapState::BottomHalf,
				SnapDirection::Right,
				work_area
			),
			layout.bottom_right
		);
		assert_eq!(
			target_rect_for_direction(SnapState::TopLeft, SnapDirection::Left, work_area),
			layout.left_half
		);
		assert_eq!(
			target_rect_for_direction(
				SnapState::BottomRight,
				SnapDirection::Right,
				work_area
			),
			layout.right_half
		);
	}

	#[test]
	fn vertical_transitions_expand_and_contract_between_half_and_quadrant() {
		let work_area = rect(0, 0, 100, 80);
		let layout = SnapLayout::new(work_area);
		assert_eq!(
			target_rect_for_direction(SnapState::LeftHalf, SnapDirection::Up, work_area),
			layout.top_left
		);
		assert_eq!(
			target_rect_for_direction(
				SnapState::RightHalf,
				SnapDirection::Down,
				work_area
			),
			layout.bottom_right
		);
		assert_eq!(
			target_rect_for_direction(SnapState::TopLeft, SnapDirection::Up, work_area),
			layout.top_half
		);
		assert_eq!(
			target_rect_for_direction(
				SnapState::BottomRight,
				SnapDirection::Down,
				work_area
			),
			layout.bottom_half
		);
	}
}
