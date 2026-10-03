use std::io::{BufReader, Write};
use std::path::PathBuf;

use loupe_plugins::vst3::{Effect, Library};
use loupe_plugins::wire::{next_line, read_block, write_block, Ask, Reply};

#[cfg(windows)]
mod com {
    #[link(name = "ole32")]
    extern "system" {
        fn CoInitializeEx(reserved: *mut core::ffi::c_void, model: u32) -> i32;
        fn CoUninitialize();
    }

    pub fn start() {
        unsafe {
            CoInitializeEx(std::ptr::null_mut(), 0x2);
        }
    }

    pub fn stop() {
        unsafe {
            CoUninitialize();
        }
    }
}

#[cfg(not(windows))]
mod com {
    pub fn start() {}
    pub fn stop() {}
}

fn main() {
    com::start();
    let mut input = BufReader::new(std::io::stdin());
    let mut out = std::io::stdout();
    let mut open: Option<Effect> = None;
    while let Some(line) = next_line(&mut input) {
        let Some(ask) = Ask::read(&line) else {
            let _ = Reply::Trouble(format!("I did not understand {}", line.trim())).write(&mut out);
            continue;
        };
        if ask == Ask::Process {
            let mut audio = Vec::new();
            if read_block(&mut input, &mut audio).is_err() {
                break;
            }
            if let Some(effect) = open.as_mut() {
                effect.process(&mut audio);
            }
            if write_block(&mut out, &audio).is_err() {
                break;
            }
            continue;
        }
        let reply = match ask {
            Ask::Quit => break,
            Ask::Process => continue,
            Ask::Save => match open.as_ref() {
                Some(effect) => match effect.save() {
                    Ok(state) => Reply::State(state),
                    Err(why) => Reply::Trouble(why),
                },
                None => Reply::Trouble("no plugin is open".into()),
            },
            Ask::Restore(state) => match open.as_mut() {
                Some(effect) => match effect.restore(&state) {
                    Ok(()) => Reply::Fine,
                    Err(why) => Reply::Trouble(why),
                },
                None => Reply::Trouble("no plugin is open".into()),
            },
            Ask::Classes(path) => match Library::open(&PathBuf::from(&path)) {
                Ok(library) => {
                    let names = library.classes().into_iter().map(|class| class.name).collect();
                    Reply::Classes(names)
                }
                Err(why) => Reply::Trouble(why),
            },
            Ask::Load { path, index, rate, block } => match Library::open(&PathBuf::from(&path))
                .and_then(|library| Effect::start(library, index, rate as f64, block))
            {
                Ok(effect) => {
                    open = Some(effect);
                    Reply::Loaded { inputs: 2, outputs: 2 }
                }
                Err(why) => Reply::Trouble(why),
            },
        };
        let _ = reply.write(&mut out);
    }
    drop(open);
    let _ = out.flush();
    com::stop();
}
