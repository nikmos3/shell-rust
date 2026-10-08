#[allow(unused_imports)]
use std::io::{self, Write};
use std::process::Command;
use std::{env};

fn parse_arguments(input: &str) -> Vec<String> {
    let mut arguments = Vec::new();
    let mut current = String::new();
    let mut in_single_quotes = false;
    let mut in_double_quotes = false;
    let mut token_started = false;

    for character in input.chars() {
        match character {
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

fn main() {
    // TODO: Uncomment the code below to pass the first stage
    loop {
        print!("$ ");
        io::stdout().flush().unwrap();

        let mut command = String::new();
        io::stdin().read_line(&mut command).unwrap();

        command = command.trim().to_string();
        if command == "exit"{
            break;
        } else if command == "echo" {
            println!();

        } else if let Some(text) = command.strip_prefix("echo ") {
            println!("{}", parse_arguments(text).join(" "));

        } else if command.starts_with("type") {
            let name = &command[5..];
            if name == "type" || name == "exit" || name == "echo" || name == "pwd" || name == "cd" {
                println!("{} is a shell builtin", name);
            }else {
                match which::which(name) {
                    Ok(path) => println!("{} is {}",name,path.display()),
                    Err(_) => println!("{}: not found", name),
                }
                
               
            }

        } else if let Some(path) = command.strip_prefix("cd ") {
            let path = path.trim();
            if path == "~" {
                match env::var("HOME") {
                    Ok(home) => {
                        if env::set_current_dir(&home).is_err() {
                            println!("cd: {}: No such file or directory", home);
                        }
                    }
                    Err(_) => println!("cd: HOME not set"),
                }

            } else if env::set_current_dir(path).is_err() {
                println!("cd: {}: No such file or directory", path);
            }
        } else if command == "pwd" {
            println!("{}", env::current_dir().unwrap().display())
        } else {
            let parts = parse_arguments(&command);
            if let Some(program) = parts.first() {
                match Command::new(program).args(&parts[1..]).status() {
                    Ok(_) => {}
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {
                        println!("{}: command not found", program);
                    }
                    Err(error) => eprintln!("{}: {}", program, error),
                }
            }
        }
    }
}
