/* SPDX-FileCopyrightText: © 2023 Nadim Kobeissi <nadim@symbolic.software>
 * SPDX-License-Identifier: MIT */

mod actions;
mod desktop;
mod drag;
mod hook;
mod log;
mod mouse_hook;

use std::sync::Mutex;

static KEYDOWN_STATE: Mutex<[bool; 256]> = Mutex::new([false; 256]);

pub async fn init() {
	log::session("keys::init");
	actions::start_action_worker();
	hook::bind_shortcuts();
	mouse_hook::bind_mouse_hook();
	hook::keyboard_event_loop();
}

fn key_is_down(vkey: u32) -> bool {
	if vkey >= 256 {
		return false;
	}
	if let Ok(state) = KEYDOWN_STATE.lock() {
		return state[vkey as usize];
	}
	false
}
