#[allow(unused_imports)]
use rustyline::completion::{Completer, Pair};
use rustyline::highlight::Highlighter;
use rustyline::hint::Hinter;
use rustyline::validate::Validator;
use rustyline::{CompletionType, Config, Context, Editor, Helper};
use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::PermissionsExt;
use std::process::{Command, Stdio};

struct ShellHelper;

impl Completer for ShellHelper {
    type Candidate = Pair;

    fn complete(
        &self,
        line: &str,
        pos: usize,
        _context: &Context<'_>,
    ) -> rustyline::Result<(usize, Vec<Self::Candidate>)> {
        let line = &line[..pos];
        let start = line.rfind(char::is_whitespace).map_or(0, |index| index + 1);
        let partial = &line[start..];
        let mut candidates = Vec::new();

        if line[..start].trim().is_empty() {
            candidates.extend(
                ["echo", "exit"]
                    .into_iter()
                    .filter(|candidate| candidate.starts_with(partial))
                    .map(str::to_owned),
            );

            if !partial.is_empty() {
                if let Some(path) = env::var_os("PATH") {
                    for directory in env::split_paths(&path) {
                        let Ok(entries) = fs::read_dir(directory) else {
                            continue;
                        };
                        for entry in entries.flatten() {
                            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                                continue;
                            };
                            if name.starts_with(partial)
                                && entry.metadata().is_ok_and(|metadata| {
                                    metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
                                })
                            {
                                candidates.push(name);
                            }
                        }
                    }
                }
            }
        } else if !partial.is_empty() {
            let (directory, prefix, path_prefix) =
                partial
                    .rsplit_once('/')
                    .map_or((".", partial, ""), |(directory, prefix)| {
                        (
                            if directory.is_empty() { "/" } else { directory },
                            prefix,
                            &partial[..partial.len() - prefix.len()],
                        )
                    });
            if let Ok(entries) = fs::read_dir(directory) {
                for entry in entries.flatten() {
                    let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                        continue;
                    };
                    if name.starts_with(prefix)
                        && entry.metadata().is_ok_and(|metadata| metadata.is_file())
                    {
                        candidates.push(format!("{path_prefix}{name}"));
                    }
                }
            }
        }

        candidates.sort();
        candidates.dedup();
        let is_unique = candidates.len() == 1;
        let candidates = candidates
            .into_iter()
            .map(|candidate| Pair {
                display: candidate.clone(),
                replacement: if is_unique {
                    format!("{candidate} ")
                } else {
                    candidate
                },
            })
            .collect();
        Ok((start, candidates))
    }
}

impl Hinter for ShellHelper {
    type Hint = String;
}

impl Highlighter for ShellHelper {}
impl Validator for ShellHelper {}
impl Helper for ShellHelper {}

fn parse_arguments(input: &str) -> Vec<String> {
    let mut arguments = Vec::new();
    let mut current = String::new();
    let mut in_single_quotes = false;
    let mut in_double_quotes = false;
    let mut token_started = false;

    let mut characters = input.chars().peekable();
    while let Some(character) = characters.next() {
        if !in_single_quotes && !in_double_quotes && character == '>' {
            let append = characters.peek() == Some(&'>');
            if append {
                characters.next();
            }
            if (current == "1" || current == "2") && token_started {
                arguments.push(format!("{current}{}", if append { ">>" } else { ">" }));
                current.clear();
            } else {
                if token_started {
                    arguments.push(std::mem::take(&mut current));
                }
                arguments.push(if append { ">>" } else { ">" }.to_string());
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
    let config = Config::builder()
        .completion_type(CompletionType::List)
        .completion_show_all_if_ambiguous(false)
        .build();
    let mut editor =
        match Editor::<ShellHelper, rustyline::history::DefaultHistory>::with_config(config) {
            Ok(editor) => editor,
            Err(error) => {
                eprintln!("shell: failed to initialize input editor: {error}");
                return;
            }
        };
    editor.set_helper(Some(ShellHelper));

    loop {
        let command = match editor.readline("$ ") {
            Ok(command) => command,
            Err(rustyline::error::ReadlineError::Interrupted)
            | Err(rustyline::error::ReadlineError::Eof) => break,
            Err(error) => {
                eprintln!("shell: failed to read command: {error}");
                break;
            }
        };
        let _ = editor.add_history_entry(command.as_str());
        let mut parts = parse_arguments(&command);
        if parts.first().is_some_and(|part| part == "exit") {
            break;
        }

        let mut output = None;
        let mut error_output = None;
        while let Some(operator) = parts
            .iter()
            .position(|part| [">", "1>", "2>", ">>", "1>>", "2>>"].contains(&part.as_str()))
        {
            if operator + 1 >= parts.len() {
                eprintln!("shell: redirection requires a file");
                break;
            }
            let file = if parts[operator].ends_with(">>") {
                OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&parts[operator + 1])
            } else {
                File::create(&parts[operator + 1])
            };
            match file {
                Ok(file) if parts[operator] == "2>" => error_output = Some(file),
                Ok(file) if parts[operator] == "2>>" => error_output = Some(file),
                Ok(file) => output = Some(file),
                Err(error) => {
                    eprintln!("{}: {}", parts[operator + 1], error);
                    break;
                }
            }
            parts.drain(operator..=operator + 1);
        }

        if parts
            .iter()
            .any(|part| [">", "1>", "2>", ">>", "1>>", "2>>"].contains(&part.as_str()))
        {
            continue;
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
                    if error.kind() == io::ErrorKind::NotFound {
                        eprintln!("cd: {destination}: No such file or directory");
                    } else {
                        eprintln!("cd: {destination}: {error}");
                    }
                }
                continue;
            }
            _ => {
                let mut child = Command::new(program);
                child.args(&parts[1..]);
                if let Some(file) = output.take() {
                    child.stdout(Stdio::from(file));
                }
                if let Some(file) = error_output.take() {
                    child.stderr(Stdio::from(file));
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
