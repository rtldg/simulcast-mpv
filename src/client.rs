// SPDX-License-Identifier: AGPL-3.0-or-later

// https://mpv.io/manual/master/
// https://github.com/snapview/tungstenite-rs
// https://crates.io/crates/shadow-rs
// https://github.dev/mpv-player/mpv
// https://docs.rs/mpvipc/

use std::io::Write;

use anyhow::anyhow;
use clap_verbosity_flag::{ErrorLevel, Verbosity};
use mpvipc::{Event, Mpv, Property};

pub fn client(
	verbosity: Verbosity<ErrorLevel>,
	relay_url: http::Uri,
	relay_room: String,
	client_sock: String,
) -> anyhow::Result<()> {
	if !relay_url.host().is_some() {
		return Err(anyhow!("relay url is missing a host. url: {}", relay_url));
	}

	// I originally was testing with hardcoded socket names and had a typo that put me back an hour...
	let mut mpv = Mpv::connect(&client_sock)?;

	let track: String = mpv.get_property("media-title")?;
	// println!("track = {track}");
	// std::io::stdout().flush().unwrap();

	mpv.observe_property(1, "media-title")?;
	mpv.observe_property(2, "pause")?;

	loop {
		match mpv.event_listen()? {
			Event::Shutdown => return Ok(()),
			Event::Unimplemented => {}
			Event::PropertyChange { id, property } => match property {
				Property::Pause(value) => {}
				Property::Unknown { name, data } => match name.as_str() {
					"media-title" => {}
					_ => {}
				},
				_ => {}
			},
			Event::Tick => {}
			Event::Seek => {}
			_ => {}
		}
	}

	Ok(())
}
