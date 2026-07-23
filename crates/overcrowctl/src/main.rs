use overcrow_logging::{DEFAULT_READ_LINES, read_recent_logs};
use overcrow_protocol::Core1Proxy;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Command {
    Status,
    Toggle,
    Interactive,
    Passive,
    Clear,
    Logs,
}

fn parse_command<'a>(mut arguments: impl Iterator<Item = &'a str>) -> Result<Command, ()> {
    let command = match arguments.next() {
        Some("status") => Command::Status,
        Some("toggle") => Command::Toggle,
        Some("interactive") => Command::Interactive,
        Some("passive") => Command::Passive,
        Some("clear") => Command::Clear,
        Some("logs") => Command::Logs,
        _ => return Err(()),
    };
    if arguments.next().is_some() {
        return Err(());
    }
    Ok(command)
}

async fn invoke(command: Command) -> zbus::Result<String> {
    let connection = zbus::Connection::session().await?;
    let proxy = Core1Proxy::new(&connection).await?;

    match command {
        Command::Status => proxy.snapshot().await,
        Command::Toggle => proxy.toggle_overlay().await,
        Command::Interactive => proxy.set_overlay_interactive(true).await,
        Command::Passive => proxy.set_overlay_interactive(false).await,
        Command::Clear => proxy.clear_window().await,
        Command::Logs => unreachable!("local commands must not use D-Bus"),
    }
}

fn print_logs() -> i32 {
    match read_recent_logs(DEFAULT_READ_LINES) {
        Ok(lines) => {
            for line in lines {
                println!("{line}");
            }
            0
        }
        Err(error) => {
            eprintln!("overcrowctl: cannot read diagnostic logs: {error}");
            1
        }
    }
}

async fn run() -> i32 {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let Ok(command) = parse_command(arguments.iter().map(String::as_str)) else {
        eprintln!("usage: overcrowctl <status|toggle|interactive|passive|clear|logs>");
        return 2;
    };

    if command == Command::Logs {
        return print_logs();
    }

    match invoke(command).await {
        Ok(json) => {
            println!("{json}");
            0
        }
        Err(error) => {
            eprintln!("overcrowctl: D-Bus error: {error}");
            1
        }
    }
}

#[tokio::main]
async fn main() {
    std::process::exit(run().await);
}

#[cfg(test)]
mod tests {
    use super::{Command, parse_command};

    #[test]
    fn parses_every_supported_command() {
        assert_eq!(parse_command(["status"].into_iter()), Ok(Command::Status));
        assert_eq!(parse_command(["toggle"].into_iter()), Ok(Command::Toggle));
        assert_eq!(
            parse_command(["interactive"].into_iter()),
            Ok(Command::Interactive)
        );
        assert_eq!(parse_command(["passive"].into_iter()), Ok(Command::Passive));
        assert_eq!(parse_command(["clear"].into_iter()), Ok(Command::Clear));
        assert_eq!(parse_command(["logs"].into_iter()), Ok(Command::Logs));
    }

    #[test]
    fn rejects_missing_unknown_and_extra_arguments() {
        assert!(parse_command([].into_iter()).is_err());
        assert!(parse_command(["launch"].into_iter()).is_err());
        assert!(parse_command(["status", "extra"].into_iter()).is_err());
    }
}
