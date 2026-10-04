fn main() {
    let path = std::env::args().nth(1).expect("give a plugin path");
    let library = match loupe_plugins::vst3::Library::open(std::path::Path::new(&path)) {
        Ok(library) => library,
        Err(why) => {
            println!("could not open: {why}");
            return;
        }
    };
    for (at, class) in library.classes().into_iter().enumerate() {
        let made = unsafe { library.make::<vst3::Steinberg::Vst::IEditController>(&class.id) };
        let said = match made {
            Ok(_) => "made as a controller".to_string(),
            Err(why) => why,
        };
        println!("{at}: {} [{}] -> {said}", class.name, class.category);
    }
}
