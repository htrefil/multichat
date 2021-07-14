mod logging;
mod screen;

use crossterm::event::EventStream;
use crossterm::terminal;
use crossterm::tty::IsTty;
use futures::stream::StreamExt;
use logging::Entry;
use screen::{Event, Screen};
use std::io::{self, Error, ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::process;
use structopt::StructOpt;
use tokio::sync::mpsc::UnboundedReceiver;

#[derive(StructOpt)]
#[structopt(name = "multichat-tui", about = "Multichat TUI client")]
struct Args {
    #[structopt(long, short, help = "Server to connect to")]
    server: Option<String>,
    #[structopt(long, short, help = "Group to join")]
    group: Option<String>,
    #[structopt(long, short, help = "Username")]
    username: Option<String>,
    #[structopt(long, short, help = "TLS identity path")]
    identity_path: Option<PathBuf>,
}

#[tokio::main]
async fn main() {
    let receiver = logging::init().unwrap();
    let args = Args::from_args();

    if let Err(err) = run(
        receiver,
        args.server.as_deref(),
        args.group.as_deref(),
        args.username.as_deref(),
        args.identity_path.as_deref(),
    )
    .await
    {
        log::error!("Error: {}", err);
        process::exit(1);
    }
}

async fn run(
    mut receiver: UnboundedReceiver<Entry>,
    server: Option<&str>,
    group: Option<&str>,
    username: Option<&str>,
    identity_path: Option<&Path>,
) -> Result<(), Error> {
    let mut screen = Screen::new()?;
    let mut stream = EventStream::new();
    let mut exit = false;

    loop {
        if exit {
            return Ok(());
        }

        let event = tokio::select! {
            // New log entry.
            entry = receiver.recv() => {
                let entry = entry.unwrap();
                screen.write(entry.level, None, entry.message);

                None
            }
            // New terminal event (resize, text input, ...).
            event = stream.next() => {
                let event = match event {
                    Some(event) => event?,
                    None => continue,
                };

                screen.event(event)
            }
        };

        if let Some(Event::Exit) = event {
            return Ok(());
        }

        screen.render()?;
    }
}
