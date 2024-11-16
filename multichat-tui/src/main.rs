mod command;
mod screen;
mod term_safe;
mod tui;

use screen::Screen;
use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    let mut screen = match Screen::new() {
        Ok(screen) => screen,
        Err(err) => {
            eprintln!("Error: {}", err);
            return ExitCode::FAILURE;
        }
    };

    match tui::run(&mut screen).await.and_then(|_| screen.close()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("Error: {}", err);
            ExitCode::FAILURE
        }
    }
}
