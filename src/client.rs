// SPDX-License-Identifier: AGPL-3.0-or-later

// https://mpv.io/manual/master/
// https://github.com/snapview/tungstenite-rs
// https://crates.io/crates/shadow-rs
// https://github.dev/mpv-player/mpv
// https://docs.rs/mpvipc/

use futures::StreamExt;

use anyhow::anyhow;
use clap_verbosity_flag::{ErrorLevel, Verbosity};
use mpvipc::{Event, Mpv, MpvCommand, Property};
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::sync::mpsc::UnboundedSender;

async fn ws_thread(relay_url: String) -> anyhow::Result<()> {
	let (mut ws, _) = tokio_tungstenite::connect_async(relay_url).await?;
	loop {
		tokio::select! {
			msg = ws.next() => {

			}
		}
	}
	Ok(())
}

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
	let mut mpv_one = Mpv::connect(&client_sock)?;
	let mut mpv_two = mpv_one.clone(); // to be used by ws_thread to send commands?

	let rt = tokio::runtime::Builder::new_multi_thread()
		.enable_all()
		.worker_threads(2)
		.thread_name("tokio worker")
		.build()?;

	let ws_thread = std::thread::Builder::new()
		.name("websocket thread".to_string())
		.spawn(move || {
			rt.block_on(async move {
				let _ = ws_thread(relay_url.to_string());
			});
		})?;

	let track: String = mpv_one.get_property("media-title")?;
	// println!("track = {track}");
	// std::io::stdout().flush().unwrap();

	mpv_one.observe_property(1, "media-title")?;
	mpv_one.observe_property(2, "pause")?;
	mpv_one.observe_property(3, "playback-time")?;

	let mut tick = 0;

	loop {
		match mpv_one.event_listen()? {
			Event::Shutdown => return Ok(()),
			Event::Unimplemented => {}
			Event::PropertyChange { id, property } => match property {
				Property::Pause(value) => {
					mpv_one.run_command(MpvCommand::ShowText {
						text: "test".to_string(),
						duration_ms: Some(5 * 1000),
						level: None,
					})?;
				}
				Property::Unknown { name, data } => match name.as_str() {
					"media-title" => {}
					_ => {}
				},
				Property::PlaybackTime(t) => {
					tick += 1;
					mpv_one.run_command(MpvCommand::ShowText {
						text: tick.to_string(),
						duration_ms: Some(100),
						level: None,
					})?;
				}
				_ => {}
			},
			Event::Seek => {}
			_ => {}
		}
	}

	Ok(())
}
