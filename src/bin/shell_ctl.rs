//! Install, uninstall or query the per-user shell registration.
//! Usage: shell_ctl.exe status | install <path-to-shell.exe> | uninstall

use fluent_win::shell_registration::{ShellRegistration, command_line};

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(command) = args.next() else {
        eprintln!("usage: shell_ctl.exe status | install <path> | uninstall");
        std::process::exit(2);
    };
    let registration = ShellRegistration::default();
    let result = match command.as_str() {
        "status" => registration.current().map(|value| {
            match value {
                Some(value) => println!("Shell = {value}"),
                None => println!("Shell is not set (Windows uses explorer.exe)"),
            }
        }),
        "install" => {
            let Some(path) = args.next() else {
                eprintln!("install needs the path to the shell executable");
                std::process::exit(2);
            };
            if !std::path::Path::new(&path).exists() {
                eprintln!("no such executable: {path}");
                std::process::exit(2);
            }
            let value = command_line(&path);
            registration.install(&value).map(|()| {
                println!("Shell = {value}");
                println!("Log off and back on for Winlogon to start it.");
            })
        }
        "uninstall" => registration.uninstall().map(|()| {
            match registration.current() {
                Ok(Some(value)) => println!("Shell restored to {value}"),
                Ok(None) => println!("Shell value removed (Windows uses explorer.exe)"),
                Err(e) => eprintln!("warning: could not read Shell after uninstall: {e}"),
            }
        }),
        _ => {
            eprintln!("unknown command: {command}");
            std::process::exit(2);
        }
    };
    if let Err(e) = result {
        eprintln!("shell_ctl: {e}");
        std::process::exit(1);
    }
}
