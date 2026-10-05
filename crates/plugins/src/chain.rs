use std::path::{Path, PathBuf};

use crate::ceiling::Seat;
use crate::sandbox::Sandbox;
use crate::wire::{Ask, Link, Region, Reply};

#[derive(Clone, Debug, PartialEq)]
pub struct Fell {
    pub seat: usize,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Joining {
    pub name: String,
    pub path: PathBuf,
    pub index: usize,
    pub rate: u32,
    pub block: usize,
    pub region: Option<Region>,
    pub state: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Joined {
    pub seat: usize,
    pub latency: usize,
    pub ara: bool,
}

pub struct Alone {
    pub host: Sandbox,
    pub latency: usize,
    pub ara: bool,
}

pub struct Chain {
    host: Sandbox,
    seated: Vec<Option<String>>,
    addressed: Option<usize>,
}

impl Chain {
    pub fn start(host: &Path, room: Seat) -> Result<Self, String> {
        Ok(Self { host: Sandbox::start_in_a_seat(host, room)?, seated: Vec::new(), addressed: None })
    }

    pub fn pid(&self) -> u32 {
        self.host.pid()
    }

    pub fn gone(&self) -> bool {
        self.host.gone()
    }

    pub fn fell_at(&self) -> Option<usize> {
        self.host.fell_at()
    }

    pub fn name_at(&self, seat: usize) -> Option<&str> {
        self.seated.get(seat)?.as_deref()
    }

    pub fn who_fell(&self) -> Option<Fell> {
        let seat = self.host.fell_at()?;
        Some(Fell { seat, name: self.name_at(seat).unwrap_or_default().to_string() })
    }

    pub fn everyone(&self) -> impl Iterator<Item = (usize, &str)> {
        self.seated.iter().enumerate().filter_map(|(seat, name)| name.as_deref().map(|name| (seat, name)))
    }

    pub fn running(&self) -> usize {
        self.seated.iter().flatten().count()
    }

    pub fn holds(&self, seat: usize) -> bool {
        self.seated.get(seat).is_some_and(Option::is_some)
    }

    fn free_seat(&mut self) -> usize {
        match self.seated.iter().position(Option::is_none) {
            Some(seat) => seat,
            None => {
                self.seated.push(None);
                self.seated.len() - 1
            }
        }
    }

    fn address(&mut self, seat: usize) -> Result<(), String> {
        if self.addressed == Some(seat) {
            return Ok(());
        }
        self.addressed = None;
        match self.host.ask(Ask::Seat(seat))? {
            Reply::Fine => {
                self.addressed = Some(seat);
                Ok(())
            }
            Reply::Trouble(why) => Err(why),
            other => Err(out_of_turn(&other)),
        }
    }

    pub fn join(&mut self, order: &Joining) -> Result<Joined, String> {
        let seat = self.free_seat();
        self.sit(seat, order)
    }

    fn sit(&mut self, seat: usize, order: &Joining) -> Result<Joined, String> {
        self.address(seat)?;
        if let Some(region) = order.region.as_ref() {
            match self.host.ask(Ask::Region(region.clone()))? {
                Reply::Fine => {}
                Reply::Trouble(why) => return Err(why),
                other => return Err(out_of_turn(&other)),
            }
        }
        let ask = Ask::Load {
            path: order.path.to_string_lossy().to_string(),
            index: order.index,
            rate: order.rate,
            block: order.block,
        };
        let (latency, ara) = match self.host.ask(ask)? {
            Reply::Loaded { latency, ara, .. } => (latency, ara),
            Reply::Trouble(why) => return Err(why),
            other => return Err(out_of_turn(&other)),
        };
        self.seated.resize(self.seated.len().max(seat + 1), None);
        self.seated[seat] = Some(order.name.clone());
        if !order.state.is_empty() {
            match self.host.ask(Ask::Restore(order.state.clone()))? {
                Reply::Fine => {}
                Reply::Trouble(why) => return Err(why),
                other => return Err(out_of_turn(&other)),
            }
        }
        Ok(Joined { seat, latency, ara })
    }

    pub fn ask_of(&mut self, seat: usize, ask: Ask) -> Result<Reply, String> {
        self.address(seat)?;
        self.host.ask(ask)
    }

    pub fn save(&mut self, seat: usize) -> Result<Vec<u8>, String> {
        match self.ask_of(seat, Ask::Save)? {
            Reply::State(state) => Ok(state),
            Reply::Trouble(why) => Err(why),
            other => Err(out_of_turn(&other)),
        }
    }

    pub fn leave(&mut self, seat: usize) -> Result<Vec<u8>, String> {
        let kept = self.save(seat);
        if let Some(room) = self.seated.get_mut(seat) {
            *room = None;
        }
        let dropped = self.host.ask(Ask::Unload(seat));
        kept.and_then(|state| dropped.map(|_| state))
    }

    pub fn follow_region(&mut self, seat: usize, region: &Region) -> Result<(), String> {
        match self.ask_of(seat, Ask::Region(region.clone()))? {
            Reply::Fine => Ok(()),
            Reply::Trouble(why) => Err(why),
            other => Err(out_of_turn(&other)),
        }
    }

    pub fn run(&mut self, order: &[Link], audio: &mut Vec<[f32; 2]>, side: &[[f32; 2]]) -> Result<(), String> {
        self.host.run_chain(order, audio, side)
    }
}

pub fn out_of_turn(reply: &Reply) -> String {
    format!("the plugin host answered out of turn: {reply:?}")
}

pub fn on_its_own(host: &Path, room: Seat, order: &Joining) -> Result<Alone, String> {
    let mut alone = Chain::start(host, room)?;
    let joined = alone.sit(0, order)?;
    Ok(Alone { host: alone.host, latency: joined.latency, ara: joined.ara })
}

pub fn out_of_the_chain(chain: &mut Chain, seat: usize, host: &Path, room: Seat, order: &Joining) -> Result<Alone, String> {
    let kept = chain.leave(seat)?;
    on_its_own(host, room, &Joining { state: kept, ..order.clone() })
}

pub fn into_the_chain(chain: &mut Chain, mut alone: Alone, order: &Joining) -> Result<Joined, String> {
    let kept = match alone.host.ask(Ask::Save)? {
        Reply::State(state) => state,
        Reply::Trouble(why) => return Err(why),
        other => return Err(out_of_turn(&other)),
    };
    drop(alone);
    chain.join(&Joining { state: kept, ..order.clone() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ceiling::Ceiling;

    fn room() -> Seat {
        Ceiling::of(64).squeeze_in()
    }

    #[test]
    fn a_chain_that_cannot_start_says_so() {
        let fell = Chain::start(Path::new("loupe-host-that-is-not-there"), room());
        assert!(fell.is_err());
    }
}

#[cfg(all(test, unix))]
mod fake_host_tests {
    use super::*;
    use crate::ceiling::Ceiling;
    use std::os::unix::fs::PermissionsExt;

    struct Script(PathBuf);

    impl Drop for Script {
        fn drop(&mut self) {
            if let Some(folder) = self.0.parent() {
                let _ = std::fs::remove_dir_all(folder);
            }
        }
    }

    fn script(name: &str, body: &str) -> Script {
        let folder = std::env::temp_dir().join(format!("loupe-chain-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let file = folder.join("loupe-host");
        std::fs::write(&file, format!("#!/bin/sh\n{body}")).unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
        Script(file)
    }

    fn order(path: &str) -> Joining {
        Joining { name: path.to_string(), path: PathBuf::from(path), index: 0, rate: 48_000, block: 64, region: None, state: Vec::new() }
    }

    const ANSWERS_EVERYTHING: &str = r#"
while read -r ask rest; do
  case "$ask" in
    seat) printf 'fine\n' ;;
    unload) printf 'fine\n' ;;
    load) printf 'loaded\t2\t2\t0\t0\n' ;;
    restore) printf 'fine\n' ;;
    save) printf 'state\t0a0b\n' ;;
    chain) head -c 517 > /dev/null ; printf 'B\000\000\000\000' ;;
    quit) exit 0 ;;
    *) printf 'fine\n' ;;
  esac
done
"#;

    #[test]
    fn three_plugins_take_three_seats_in_one_process() {
        let host = script("seats", ANSWERS_EVERYTHING);
        let mut chain = Chain::start(&host.0, Ceiling::of(8).squeeze_in()).expect("the chain host starts");
        let first = chain.join(&order("one.vst3")).expect("one joins");
        let second = chain.join(&order("two.vst3")).expect("two joins");
        let third = chain.join(&order("three.vst3")).expect("three joins");
        assert_eq!((first.seat, second.seat, third.seat), (0, 1, 2));
        assert_eq!(chain.running(), 3);
        assert!(chain.holds(1));
        let kept = chain.leave(1).expect("the middle one leaves with its settings");
        assert_eq!(kept, vec![10, 11]);
        assert!(!chain.holds(1));
        assert_eq!(chain.running(), 2);
        assert_eq!(chain.join(&order("four.vst3")).expect("four joins").seat, 1, "the freed seat is used again");
    }

    #[test]
    fn a_chain_host_that_dies_mid_block_names_the_plugin() {
        let body = ANSWERS_EVERYTHING.replace("chain) head -c 517 > /dev/null ; printf 'B\\000\\000\\000\\000' ;;", "chain) head -c 517 > /dev/null ; printf 'fell\\t1\\n' ; exit 3 ;;");
        let host = script("fell", &body);
        let mut chain = Chain::start(&host.0, Ceiling::of(8).squeeze_in()).expect("the chain host starts");
        let first = chain.join(&order("Gentle Reverb")).expect("the first joins");
        let second = chain.join(&order("Crashy Saturator")).expect("the second joins");
        let mut audio = vec![[0.5; 2]; 64];
        let order = [Link::wet(first.seat), Link::wet(second.seat)];
        let why = chain.run(&order, &mut audio, &[]).expect_err("the host died");
        assert_eq!(why, "the plugin crashed");
        assert_eq!(chain.who_fell(), Some(Fell { seat: 1, name: "Crashy Saturator".into() }), "the chain named the plugin that fell");
        assert!(chain.gone(), "the whole chain is down, not just the one plugin");
        assert_eq!(chain.everyone().count(), 2, "both plugins are still named, so the user can be told what went with it");
    }
}
