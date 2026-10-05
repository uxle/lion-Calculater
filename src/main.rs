// Lion Calculator — native binary entry.
// Build: cargo build --release   (requires rustc ≥ 1.85, slint 1.17)
//
// Optional start-up mode argument:
//   lion-calculator [basic|scientific|financial|programmer]
//   (short forms: sci / fin / pro also work)

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mode = match std::env::args().nth(1).as_deref().map(str::to_ascii_lowercase) {
        Some(arg) => match arg.as_str() {
            "basic"                          => 0,
            "sci"     | "scientific"         => 1,
            "fin"     | "financial"          => 2,
            "pro"     | "programmer"         => 3,
            "-h" | "--help" | "help"         => {
                println!(
                    "Lion Calculator — modern native calculator for LionOS\n\n\
                     Usage: lion-calculator [basic|scientific|financial|programmer]\n\n\
                     Without an argument the calculator starts in Basic mode."
                );
                return Ok(());
            }
            other => {
                eprintln!("Unknown mode '{other}' — expected basic|scientific|financial|programmer");
                std::process::exit(2);
            }
        },
        None => 0,
    };
    lion_calculator::app::run_with_mode(mode).map_err(Into::into)
}
