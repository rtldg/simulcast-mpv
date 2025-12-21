// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2023-2025 rtldg <rtldg@protonmail.com>

#[cfg(feature = "client")]
use base64::prelude::*;

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

	#[cfg(feature = "client")]
	pub fn encrypt_chat(mut message: String, key: &[u8]) -> String {
		const PAD_TO: usize = 480;
		const PAD_SIZE: usize = 20;
		if message.len() < PAD_TO {
			message.reserve(PAD_TO - message.len());
		}
		while message.len() < PAD_TO - PAD_SIZE {
			message.push_str("                    ");
		}
		let key = aws_lc_rs::aead::RandomizedNonceKey::new(&aws_lc_rs::aead::AES_256_GCM, key).unwrap();
		let mut in_out = message.into_bytes();
		let nonce = key
			.seal_in_place_append_tag(aws_lc_rs::aead::Aad::empty(), &mut in_out)
			.unwrap();
		in_out.extend_from_slice(nonce.as_ref());
		BASE64_STANDARD.encode(&in_out)
	}

	#[cfg(feature = "client")]
	pub fn decrypt_chat(encrypted: &str, key: &[u8]) -> anyhow::Result<String> {
		let mut in_out = BASE64_STANDARD.decode(encrypted)?;
		let key = aws_lc_rs::aead::RandomizedNonceKey::new(&aws_lc_rs::aead::AES_256_GCM, key).unwrap();
		let nonce_len = aws_lc_rs::aead::NONCE_LEN;
		anyhow::ensure!(encrypted.len() > nonce_len + 2);
		let nonce = in_out.split_off(encrypted.len() - nonce_len);
		let nonce = aws_lc_rs::aead::Nonce::try_assume_unique_for_key(&nonce).unwrap();
		let plaintext = key.open_in_place(nonce, aws_lc_rs::aead::Aad::empty(), &mut in_out)?;
		Ok(str::from_utf8(&plaintext)?.trim().to_owned())
	}
}
