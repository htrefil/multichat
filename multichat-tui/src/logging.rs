use log::{Level, Log, Metadata, Record, SetLoggerError};
use tokio::sync::mpsc::error::SendError;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

pub struct Entry {
    pub level: Level,
    pub message: String,
}

pub fn init() -> Result<UnboundedReceiver<Entry>, SetLoggerError> {
    let (sender, receiver) = mpsc::unbounded_channel();

    log::set_boxed_logger(Box::new(Logger {
        sender: sender.into(),
    }))?;

    Ok(receiver)
}

struct Logger {
    // Unbounded because we could deadlock otherwise.
    sender: UnboundedSender<Entry>,
}

impl Log for Logger {
    fn enabled(&self, _metadata: &Metadata) -> bool {
        // TODO: Add filtering. We most certainly don't want to spam the user with trace and debug logs.
        true
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }

        // If the other half is closed, just print it into stdout.
        let message = format!("{}", record.args());
        if let Err(SendError(entry)) = self.sender.send(Entry {
            level: record.level(),
            message,
        }) {
            println!("{}", entry.message);
        }
    }

    fn flush(&self) {}
}
