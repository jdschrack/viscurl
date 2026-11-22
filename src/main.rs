mod adapters;
mod application;
mod domain;

use std::{
    env,
    fs::File,
    io::{self, IsTerminal, Read},
    path::Path,
    process::ExitCode,
};

const SPEC_LIMIT: usize = 4 * 1024 * 1024;
const USAGE: &str = "Usage: viscurl <openapi.yaml|openapi.json>";

fn load(path: &Path) -> Result<domain::spec::Api, String> {
    if !path
        .metadata()
        .map_err(|error| format!("Cannot open specification: {error}"))?
        .is_file()
    {
        return Err("The specification must be a regular file".into());
    }
    let mut source = String::new();
    let file = File::open(path).map_err(|error| format!("Cannot open specification: {error}"))?;
    if !file
        .metadata()
        .map_err(|error| error.to_string())?
        .is_file()
    {
        return Err("The specification must be a regular file".into());
    }
    file.take((SPEC_LIMIT + 1) as u64)
        .read_to_string(&mut source)
        .map_err(|error| format!("Cannot read specification: {error}"))?;
    if source.len() > SPEC_LIMIT {
        return Err("The specification exceeds the 4 MiB import limit".into());
    }
    domain::spec::parse(&source)
}

fn run() -> Result<(), String> {
    let mut args = env::args_os().skip(1);
    let path = args.next().ok_or(USAGE)?;
    if matches!(path.to_str(), Some("-h" | "--help")) {
        println!("{USAGE}");
        return Ok(());
    }
    if args.next().is_some() {
        return Err(USAGE.into());
    }
    let api = load(Path::new(&path))?;
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err("viscurl requires an interactive terminal".into());
    }
    adapters::terminal::run(api).map_err(|error| error.to_string())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let safe = error
                .chars()
                .map(|ch| if ch.is_control() { ' ' } else { ch })
                .collect::<String>();
            eprintln!("viscurl: {safe}");
            ExitCode::FAILURE
        }
    }
}
