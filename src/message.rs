// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2023-2025 rtldg <rtldg@protonmail.com>

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub enum WsMessage {
	// Used to query the server's version & repository.
	// v2.1.0+
	// Client<->Server.
	Info(String),

	// I didn't make Info() forward-compatible enough for my liking.
	// So here's this instead where we just add more fields...
	// v2.3.0+
	// Client<->Server.
	Info2 { version: semver::Version },

	//
	// Only client->server.
	Join(String),
	// Number of current users in the party.
	// Implies pause (if count != 1 || previous >= 1).
	// Only server->client.
	Party(u32),

	//
	Resume,
	// Implies pause.
	AbsoluteSeek(f64),
	//
	Ping(String),
	//
	Pong(String),

	//
	// This message was done different in v2.3.0.
	// I edited it for v3.0.0 to be encrypted.
	// Client<->server.
	Chat(String),
	//
	// v3.0.0.
	// Server->client.
	RoomRandomChatSalt(String),
}

impl WsMessage {
	/// The `Message::Text` type stores a `Bytes` internally which clones cheaply so let's just prepare that early so we don't have to allocate as much 😇
	pub fn to_websocket_msg(&self) -> tokio_tungstenite::tungstenite::protocol::Message {
		tokio_tungstenite::tungstenite::protocol::Message::Text(serde_json::to_string(self).unwrap().into())
	}

	pub fn send_helper(&self) -> tokio_tungstenite::tungstenite::protocol::Message {
		match self {
			WsMessage::Ping(_) | WsMessage::Pong(_) => (),
			WsMessage::Chat(_) => {
				log::debug!("send msg = Chat(<omitted>)");
			}
			_ => log::debug!("send msg = {self:?}"),
		}
		self.to_websocket_msg()
	}
}
