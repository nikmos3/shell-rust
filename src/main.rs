#[allow(unused_imports)]
use std::io::{self, Write};
use std::process::{Command, Stdio};
use std::fs::File;
use std::{env};

fn parse_arguments(input: &str) -> Vec<String> {
    let mut arguments = Vec::new();
    let mut current = String::new();
    let mut in_single_quotes = false;
    let mut in_double_quotes = false;
    let mut token_started = false;

    let mut characters = input.chars().peekable();
    while let Some(character) = characters.next() {
        if !in_single_quotes && !in_double_quotes && character == '>' {
            if current == "1" && token_started {
                arguments.push("1>".to_string());
                current.clear();
            } else {
                if token_started {
                    arguments.push(std::mem::take(&mut current));
                }
                arguments.push(">".to_string());
            }
            token_started = false;
            continue;
        }

        match character {
            '\\' if !in_single_quotes && !in_double_quotes => {
                if let Some(escaped) = characters.next() {
                    current.push(escaped);
                    token_started = true;
                } else {
                    current.push(character);
                    token_started = true;
                }
            }
            '\\' if in_double_quotes => {
                if matches!(characters.peek(), Some('"') | Some('\\')) {
                    if let Some(escaped) = characters.next() {
                        current.push(escaped);
                    }
                } else {
                    current.push(character);
                }
                token_started = true;
            }
            '\'' if !in_double_quotes => {
                in_single_quotes = !in_single_quotes;
                token_started = true;
            }
            '"' if !in_single_quotes => {
                in_double_quotes = !in_double_quotes;
                token_started = true;
            }
            character if character.is_whitespace() && !in_single_quotes && !in_double_quotes => {
                if token_started {
                    arguments.push(std::mem::take(&mut current));
                    token_started = false;
                }
            }
            _ => {
                current.push(character);
                token_started = true;
            }
        }
    }

    if token_started {
        arguments.push(current);
    }

    arguments
}

fn write_output(output: &mut Option<File>, text: &str) -> io::Result<()> {
    match output {
        Some(file) => writeln!(file, "{text}"),
        None => writeln!(io::stdout().lock(), "{text}"),
    }
}

fn main() {
    // TODO: Uncomment the code below to pass the first stage
    loop {
        print!("$ ");
        io::stdout().flush().unwrap();

        let mut command = String::new();
        io::stdin().read_line(&mut command).unwrap();

        command = command.trim_end_matches(['\n', '\r']).to_string();
        let mut parts = parse_arguments(&command);
        if parts.first().is_some_and(|part| part == "exit") {
            break;
        }

        let mut output = None;
        if let Some(operator) = parts
            .iter()
            .position(|part| part == ">" || part == "1>")
        {
            if operator + 1 >= parts.len() {
                eprintln!("shell: redirection requires a file");
                continue;
            }
            match File::create(&parts[operator + 1]) {
                Ok(file) => output = Some(file),
                Err(error) => {
                    eprintln!("{}: {}", parts[operator + 1], error);
                    continue;
                }
            }
            parts.drain(operator..=operator + 1);
        }

        let Some(program) = parts.first() else {
            continue;
        };
        let result = match program.as_str() {
            "echo" => write_output(&mut output, &parts[1..].join(" ")),
            "pwd" => match env::current_dir() {
                Ok(path) => write_output(&mut output, &path.display().to_string()),
                Err(error) => {
                    eprintln!("pwd: {error}");
                    continue;
                }
            },
            "type" => {
                let Some(name) = parts.get(1) else {
                    continue;
                };
                let text = if ["type", "exit", "echo", "pwd", "cd"].contains(&name.as_str()) {
                    format!("{name} is a shell builtin")
                } else {
                    match which::which(name) {
                        Ok(path) => format!("{name} is {}", path.display()),
                        Err(_) => format!("{name}: not found"),
                    }
                };
                write_output(&mut output, &text)
            }
            "cd" => {
                let Some(path) = parts.get(1) else {
                    continue;
                };
                let destination = if path == "~" {
                    match env::var("HOME") {
                        Ok(home) => home,
                        Err(_) => {
                            eprintln!("cd: HOME not set");
                            continue;
                        }
                    }
                } else {
                    path.clone()
                };
                if let Err(error) = env::set_current_dir(&destination) {
                    eprintln!("cd: {destination}: {error}");
                }
                continue;
            }
            _ => {
                let mut child = Command::new(program);
                child.args(&parts[1..]);
                if let Some(file) = output.take() {
                    child.stdout(Stdio::from(file));
                }
                match child.status() {
                    Ok(_) => continue,
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {
                        eprintln!("{}: command not found", program);
                    }
                    Err(error) => eprintln!("{}: {}", program, error),
                }
                continue;
            }
        };
        if let Err(error) = result {
            eprintln!("{program}: {error}");
        }
    }
}
