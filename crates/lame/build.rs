const SOURCES: [&str; 19] = [
    "bitstream.c",
    "encoder.c",
    "fft.c",
    "gain_analysis.c",
    "id3tag.c",
    "lame.c",
    "newmdct.c",
    "presets.c",
    "psymodel.c",
    "quantize.c",
    "quantize_pvt.c",
    "reservoir.c",
    "set_get.c",
    "tables.c",
    "takehiro.c",
    "util.c",
    "vbrquantize.c",
    "VbrTag.c",
    "version.c",
];

fn main() {
    let vendor = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../vendor/lame");
    println!("cargo:rerun-if-changed={}", vendor.display());
    let mut build = cc::Build::new();
    for source in SOURCES {
        build.file(vendor.join("libmp3lame").join(source));
    }
    build
        .include(&vendor)
        .include(vendor.join("include"))
        .include(vendor.join("libmp3lame"))
        .define("HAVE_CONFIG_H", None)
        .define("NDEBUG", None)
        .opt_level(3)
        .warnings(false)
        .compile("mp3lame");
}
