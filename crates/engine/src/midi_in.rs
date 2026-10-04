use std::sync::{Arc, Mutex};

use midir::{Ignore, MidiInput, MidiInputConnection};
use rtrb::Producer;

const NOTE_ON: u8 = 0x90;
const NOTE_OFF: u8 = 0x80;
const CLIENT: &str = "Loupe";

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct KeyEvent {
    pub key: u8,
    pub velocity: f32,
}

pub type KeySender = Arc<Mutex<Producer<KeyEvent>>>;

pub fn read(message: &[u8]) -> Option<KeyEvent> {
    let (&status, rest) = message.split_first()?;
    let (&key, rest) = rest.split_first()?;
    let velocity = rest.first().copied().unwrap_or(0);
    if key > 127 {
        return None;
    }
    match status & 0xF0 {
        NOTE_ON if velocity > 0 => Some(KeyEvent { key, velocity: velocity.min(127) as f32 / 127.0 }),
        NOTE_ON | NOTE_OFF => Some(KeyEvent { key, velocity: 0.0 }),
        _ => None,
    }
}

pub struct MidiKeys {
    names: Vec<String>,
    _connections: Vec<MidiInputConnection<()>>,
}

impl MidiKeys {
    pub fn around() -> Vec<String> {
        let Ok(probe) = MidiInput::new(CLIENT) else {
            return Vec::new();
        };
        probe.ports().iter().filter_map(|port| probe.port_name(port).ok()).collect()
    }

    pub fn open(sender: KeySender, wanted: &[String]) -> Result<Self, String> {
        let probe = MidiInput::new(CLIENT).map_err(|why| why.to_string())?;
        let ports = probe.ports();
        let mut names = Vec::new();
        let mut connections = Vec::new();
        for port in &ports {
            if !wanted.is_empty() && !probe.port_name(port).is_ok_and(|name| wanted.iter().any(|pick| *pick == name)) {
                continue;
            }
            let mut input = MidiInput::new(CLIENT).map_err(|why| why.to_string())?;
            input.ignore(Ignore::All);
            let name = input.port_name(port).unwrap_or_else(|_| "MIDI input".to_string());
            let sender = sender.clone();
            let connected = input.connect(
                port,
                "loupe-keys",
                move |_, message, _| {
                    if let (Some(event), Ok(mut queue)) = (read(message), sender.lock()) {
                        let _ = queue.push(event);
                    }
                },
                (),
            );
            if let Ok(connection) = connected {
                names.push(name);
                connections.push(connection);
            }
        }
        Ok(Self { names, _connections: connections })
    }

    pub fn names(&self) -> &[String] {
        &self.names
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn note_messages_read_on_any_channel_and_zero_velocity_is_off() {
        assert_eq!(read(&[0x90, 60, 127]), Some(KeyEvent { key: 60, velocity: 1.0 }));
        assert_eq!(read(&[0x93, 64, 64]).map(|e| e.key), Some(64));
        assert_eq!(read(&[0x90, 60, 0]), Some(KeyEvent { key: 60, velocity: 0.0 }));
        assert_eq!(read(&[0x80, 60, 40]), Some(KeyEvent { key: 60, velocity: 0.0 }));
        assert_eq!(read(&[0xB0, 7, 100]), None);
        assert_eq!(read(&[0x90]), None);
        assert_eq!(read(&[0x90, 200, 10]), None);
    }
}
