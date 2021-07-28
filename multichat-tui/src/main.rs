mod command;
mod screen;

use screen::{Event, Level, Screen};
use std::io::{self, Error};
use std::process;

#[tokio::main]
async fn main() {
    let mut screen = match Screen::new() {
        Ok(screen) => screen,
        Err(err) => {
            eprintln!("Error: {}", err);
            process::exit(1);
        }
    };

    if let Err(err) = run(&mut screen).await.and_then(|_| screen.close()) {
        eprintln!("Error: {}", err);
        process::exit(1);
    }
}

async fn run(screen: &mut Screen) -> Result<(), Error> {
    screen.log(
        Level::Info,
        format!(
            "Multichat TUI v{}, using protocol v{}",
            env!("CARGO_PKG_VERSION"),
            multichat_client::proto::VERSION
        ),
    );

    loop {
        screen.render()?;

        match screen.process().await? {
            Some(Event::Input(input)) => screen.log(Level::Info, input),
            Some(Event::Quit) => return Ok(()),
            None => {}
        }
    }
}
