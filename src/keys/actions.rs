use std::os::windows::process::CommandExt;
use std::process::Command;
use std::sync::{mpsc, OnceLock};
use std::thread;

use crate::logging::log_error;

use super::desktop::{
	active_window_for_move, switch_to_desktop, target_desktop_for_shortcut,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Action {
	SwitchDesktop { desktop: u32, move_window: bool },
	LaunchWezterm { local: bool },
	Quit,
}

static ACTION_TX: OnceLock<mpsc::Sender<Action>> = OnceLock::new();

pub(crate) fn start_action_worker() {
	if ACTION_TX.get().is_some() {
		return;
	}
	let (tx, rx) = mpsc::channel::<Action>();
	if ACTION_TX.set(tx).is_err() {
		return;
	}
	thread::spawn(move || {
		while let Ok(action) = rx.recv() {
			let result = std::panic::catch_unwind(|| match action {
				Action::SwitchDesktop {
					desktop,
					move_window,
				} => {
					let moved_window = if move_window {
						active_window_for_move()
					} else {
						None
					};
					switch_to_desktop(
						target_desktop_for_shortcut(desktop),
						0,
						moved_window,
					);
				}
				Action::LaunchWezterm { local } => {
					if local {
						launch_wezterm("local");
					} else {
						launch_wezterm("arch");
					}
				}
				Action::Quit => {
					std::process::exit(0);
				}
			});
			if result.is_err() {
				log_error("keys/actions", "action worker panicked while handling action");
			}
		}
	});
}

pub(crate) fn dispatch_action(action: Action) -> bool {
	if let Some(tx) = ACTION_TX.get() {
		if tx.send(action).is_ok() {
			return true;
		}
		log_error("keys/actions", "failed to dispatch action: channel closed");
		return false;
	}
	log_error(
		"keys/actions",
		"failed to dispatch action: worker not initialized",
	);
	false
}

fn launch_wezterm(domain: &str) {
	const CREATE_NO_WINDOW: u32 = 0x0800_0000;
	const DETACHED_PROCESS: u32 = 0x0000_0008;
	if let Err(err) = Command::new("wezterm-gui.exe")
		.arg("start")
		.arg("--domain")
		.arg(domain)
		.creation_flags(CREATE_NO_WINDOW | DETACHED_PROCESS)
		.spawn()
	{
		log_error(
			"keys/actions",
			&format!(
				"failed to launch wezterm for domain '{}': {}",
				domain, err
			),
		);
	}
}
