#[allow(unused_imports)]
use std::io::{self, Write};
use std::process::Command;
use std::{env};

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
        } else if command.starts_with("echo") {
            println!("{}", &command[5..]);

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
            let mut parts = command.split_whitespace();
            if let Some(program) = parts.next() {
                match Command::new(program).args(parts).status() {
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
