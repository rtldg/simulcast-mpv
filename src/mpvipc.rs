// SPDX-License-Identifier: WTFPL
// Copyright 2024-2025 rtldg <rtldg@protonmail.com>

use anyhow::anyhow;
use interprocess::local_socket::{prelude::*, GenericFilePath, RecvHalf, SendHalf, Stream};
use serde_json::{json, Value};
use std::{
	collections::VecDeque,
	io::{prelude::*, BufReader},
};

pub struct Mpv {
	reader: BufReader<RecvHalf>,
	writer: SendHalf,

	event_queue: Option<VecDeque<Value>>,
}

impl Mpv {
	/// On Windows: `pipe` should be a string similar to r"\\.\pipe\mysocketnamehere"
	/// On Linux: `pipe` should be a local file path for a unix-socket such as "/tmp/mpv.sock"
	pub fn connect(pipe: &str) -> anyhow::Result<Mpv> {
		let name = if cfg!(windows) && !pipe.starts_with(r"\\.\pipe\") {
			format!(r"\\.pipe\{pipe}").to_fs_name::<GenericFilePath>()?
		} else {
			pipe.to_fs_name::<GenericFilePath>()?
		};

		let stream = Stream::connect(name)?;
		let (r, s) = stream.split();

		Ok(Mpv {
			reader: BufReader::new(r),
			writer: s,

			event_queue: Some(VecDeque::new()),
		})
	}

	pub fn events(&mut self, enabled: bool) {
		if enabled {
			let _ = self.event_queue.get_or_insert_with(VecDeque::new);
		} else {
			self.event_queue = None;
		}
	}

	/// Trims a trailing new-line
	pub fn read_line(&mut self) -> anyhow::Result<String> {
		let mut buffer = String::with_capacity(128);
		let _ = self.reader.read_line(&mut buffer)?;
		buffer.truncate(buffer.trim_end().len());
		//log::debug!("{}", buffer);
		Ok(buffer)
	}

	pub fn read_value(&mut self) -> anyhow::Result<Value> {
		// TODO: Could look into a 'reader' that returns lines to be able to use serde_json::from_reader()...
		Ok(serde_json::from_str(&self.read_line()?)?)
	}

	// TODO: Check for "error"="success"... (like .get_property() does...)
	//       And add a custom Error type for it...
	pub fn send(&mut self, json: &Value) -> anyhow::Result<Value> {
		// TODO: Use "request_id" & properly filter shit maybe...
		//let mut json = json.clone();
		//json["request_id"] = rand::random::<i32>().into();

		serde_json::to_writer(&mut self.writer, json)?;
		//log::debug!("{}", json);
		self.writer.write_all(b"\n")?;
		loop {
			let v = self.read_value()?;
			//log::debug!("got {}", v);
			if v.get("event").is_some() {
				if let Some(queue) = self.event_queue.as_mut() {
					queue.push_back(v);
				}
			} else {
				return Ok(v);
			}
		}
	}

	pub fn raw_command(&mut self, command: &Value) -> anyhow::Result<Value> {
		let json = json!({
			"command": command
		});
		self.send(&json)
	}

	pub fn observe_property(&mut self, id: i32, name: &str) -> anyhow::Result<()> {
		let json = json!({
			"command": ["observe_property", id, name],
		});
		let _ = self.send(&json)?;
		Ok(())
	}

	pub fn listen_for_event(&mut self) -> anyhow::Result<Value> {
		if let Some(queue) = self.event_queue.as_mut() {
			if let Some(v) = queue.pop_front() {
				return Ok(v);
			}
		}

		loop {
			let v = self.read_value()?;
			if v.get("event").is_some() {
				return Ok(v);
			}
		}
	}

	pub fn get_property(&mut self, property: &str) -> anyhow::Result<Value> {
		let json = json!({
			"command": ["get_property", property],
		});
		//log::debug!("about to get_property with {}", json);
		let mut v = self.send(&json)?;
		if v["error"] == "success" {
			Ok(v["data"].take())
		} else {
			Err(anyhow!("get_property failed. value: {v}"))
		}
	}

	pub fn set_property(&mut self, property: &str, value: &Value) -> anyhow::Result<()> {
		let json = json!({
			"command": ["set_property", property, value],
		});
		// TODO:
		let _ = self.send(&json)?;
		Ok(())
	}

	pub fn show_text(&mut self, text: &str, duration_ms: Option<i32>, level: Option<u32>) -> anyhow::Result<()> {
		let mut json = json!({
			"command": ["show-text", text],
		});
		if let Some(duration_ms) = duration_ms {
			json["command"]
				.as_array_mut()
				.unwrap()
				.push(Value::String(duration_ms.to_string()));
		}
		if let Some(level) = level {
			json["command"]
				.as_array_mut()
				.unwrap()
				.push(Value::String(level.to_string()));
		}
		// TODO:
		let _ = self.send(&json)?;
		Ok(())
	}
}
