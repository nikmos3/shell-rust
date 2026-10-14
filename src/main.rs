#[allow(unused_imports)]
use rustyline::completion::{Completer, Pair};
use rustyline::config::BellStyle;
use rustyline::highlight::Highlighter;
use rustyline::hint::Hinter;
use rustyline::validate::Validator;
use rustyline::{CompletionType, Config, Context, Editor, Helper};
use std::collections::HashMap;
use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::PermissionsExt;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

// ─── Tab-Completion ─────────────────────────────────────────────────────────

struct ShellHelper {
    completion_scripts: Arc<Mutex<HashMap<String, String>>>,
}

#[derive(PartialEq, Eq)]
struct CompletionCandidate {
    name: String,
    is_directory: bool,
}

impl CompletionCandidate {
    fn new(name: String, is_directory: bool) -> Self {
        Self { name, is_directory }
    }

    fn into_pair(self, is_unique: bool) -> Pair {
        let display = if self.is_directory {
            format!("{}/", self.name)
        } else {
            self.name.clone()
        };
        let replacement = if is_unique {
            format!("{}{}", self.name, if self.is_directory { "/" } else { " " })
        } else {
            self.name
        };

        Pair {
            display,
            replacement,
        }
    }
}

impl Completer for ShellHelper {
    type Candidate = Pair;

    fn complete(
        &self,
        line: &str,
        pos: usize,
        _context: &Context<'_>,
    ) -> rustyline::Result<(usize, Vec<Self::Candidate>)> {
        let completion_line = line;
        let line = &line[..pos];
        let start = line.rfind(char::is_whitespace).map_or(0, |index| index + 1);
        let partial = &line[start..];
        let words_before_partial = line[..start].split_whitespace();
        let command = words_before_partial.clone().next();
        let previous_word = words_before_partial.last().unwrap_or("");
        let registered_script = command
            .filter(|_| line.chars().any(char::is_whitespace))
            .and_then(|command| match self.completion_scripts.lock() {
                Ok(scripts) => scripts.get(command).cloned(),
                Err(error) => {
                    eprintln!("complete: failed to access registrations: {error}");
                    None
                }
            });
        let registered_candidates = registered_script.map(|script| {
            let Some(command) = command else {
                return Vec::new();
            };
            match Command::new(&script)
                .args([command, partial, previous_word])
                .env("COMP_LINE", completion_line)
                .env("COMP_POINT", pos.to_string())
                .output()
            {
                Ok(output) => String::from_utf8_lossy(&output.stdout)
                    .lines()
                    .filter(|name| !name.is_empty())
                    .map(|name| CompletionCandidate::new(name.to_owned(), false))
                    .collect(),
                Err(error) => {
                    eprintln!("complete: failed to run {script}: {error}");
                    Vec::new()
                }
            }
        });
        let mut candidates = if let Some(candidates) = registered_candidates {
            candidates
        } else if line[..start].trim().is_empty() {
            command_candidates(partial)
        } else {
            path_candidates(partial)
        };

        candidates.sort_by(|left, right| left.name.cmp(&right.name));
        candidates.dedup();
        if candidates.len() > 1 {
            let mut common_prefix = candidates[0].name.clone();
            for candidate in candidates.iter().skip(1) {
                common_prefix = common_prefix
                    .chars()
                    .zip(candidate.name.chars())
                    .take_while(|(left, right)| left == right)
                    .map(|(character, _)| character)
                    .collect();
            }
            if common_prefix.starts_with(partial) && common_prefix.len() > partial.len() {
                return Ok((
                    start,
                    vec![Pair {
                        display: common_prefix.clone(),
                        replacement: common_prefix,
                    }],
                ));
            }
        }
        let is_unique = candidates.len() == 1;
        let candidates = candidates
            .into_iter()
            .map(|candidate| candidate.into_pair(is_unique))
            .collect();
        Ok((start, candidates))
    }
}

fn command_candidates(partial: &str) -> Vec<CompletionCandidate> {
    let mut candidates = ["echo", "exit"]
        .into_iter()
        .filter(|candidate| candidate.starts_with(partial))
        .map(|candidate| CompletionCandidate::new(candidate.to_owned(), false))
        .collect::<Vec<_>>();

    if partial.is_empty() {
        return candidates;
    }

    let Some(path) = env::var_os("PATH") else {
        return candidates;
    };

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
                candidates.push(CompletionCandidate::new(name, false));
            }
        }
    }

    candidates
}

fn path_candidates(partial: &str) -> Vec<CompletionCandidate> {
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
    let Ok(entries) = fs::read_dir(directory) else {
        return Vec::new();
    };

    entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_str()?.to_owned();
            if !name.starts_with(prefix) {
                return None;
            }

            let metadata = entry.metadata().ok()?;
            (metadata.is_file() || metadata.is_dir()).then(|| {
                CompletionCandidate::new(format!("{path_prefix}{name}"), metadata.is_dir())
            })
        })
        .collect()
}

impl Hinter for ShellHelper {
    type Hint = String;
}

impl Highlighter for ShellHelper {}
impl Validator for ShellHelper {}
impl Helper for ShellHelper {}

// ─── Argument Parsing ───────────────────────────────────────────────────────

fn parse_arguments(input: &str) -> Vec<String> {
    let mut arguments = Vec::new();
    let mut current = String::new();
    let mut in_single_quotes = false;
    let mut in_double_quotes = false;
    let mut token_started = false;

    let mut characters = input.chars().peekable();
    while let Some(character) = characters.next() {
        if !in_single_quotes && !in_double_quotes && character == '>' {
            handle_redirection_token(
                &mut characters,
                &mut arguments,
                &mut current,
                &mut token_started,
            );
            continue;
        }

        match character {
            '\\' if !in_single_quotes && !in_double_quotes => {
                handle_backslash_unquoted(&mut characters, &mut current);
                token_started = true;
            }
            '\\' if in_double_quotes => {
                handle_backslash_in_double_quotes(&mut characters, &mut current);
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
            c if c.is_whitespace() && !in_single_quotes && !in_double_quotes => {
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

/// Handles `>` and `>>` redirection tokens during argument parsing.
fn handle_redirection_token(
    characters: &mut std::iter::Peekable<std::str::Chars>,
    arguments: &mut Vec<String>,
    current: &mut String,
    token_started: &mut bool,
) {
    let append = characters.peek() == Some(&'>');
    if append {
        characters.next();
    }
    let operator = if append { ">>" } else { ">" };

    if (current == "1" || current == "2") && *token_started {
        arguments.push(format!("{current}{operator}"));
        current.clear();
    } else {
        if *token_started {
            arguments.push(std::mem::take(current));
        }
        arguments.push(operator.to_string());
    }
    *token_started = false;
}

/// Handles `\X` escape sequences outside of any quotes.
fn handle_backslash_unquoted(
    characters: &mut std::iter::Peekable<std::str::Chars>,
    current: &mut String,
) {
    if let Some(escaped) = characters.next() {
        current.push(escaped);
    } else {
        current.push('\\');
    }
}

/// Handles `\X` escape sequences inside double quotes (only `\"` and `\\`).
fn handle_backslash_in_double_quotes(
    characters: &mut std::iter::Peekable<std::str::Chars>,
    current: &mut String,
) {
    if matches!(characters.peek(), Some('"') | Some('\\')) {
        if let Some(escaped) = characters.next() {
            current.push(escaped);
        }
    } else {
        current.push('\\');
    }
}

// ─── I/O Redirection ────────────────────────────────────────────────────────

const REDIRECTION_OPERATORS: &[&str] = &[">", "1>", "2>", ">>", "1>>", "2>>"];

struct Redirection {
    stdout: Option<File>,
    stderr: Option<File>,
}

/// Extracts redirection operators and opens the target files,
/// removing the consumed tokens from `parts`. Returns `None` if a
/// redirection is malformed (missing file, I/O error).
fn extract_redirections(parts: &mut Vec<String>) -> Option<Redirection> {
    let mut stdout = None;
    let mut stderr = None;

    while let Some(position) = parts
        .iter()
        .position(|part| REDIRECTION_OPERATORS.contains(&part.as_str()))
    {
        if position + 1 >= parts.len() {
            eprintln!("shell: redirection requires a file");
            return None;
        }

        let operator = &parts[position];
        let is_append = operator.ends_with(">>");
        let targets_stderr = operator.starts_with('2');

        let file = if is_append {
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(&parts[position + 1])
        } else {
            File::create(&parts[position + 1])
        };

        match file {
            Ok(file) if targets_stderr => stderr = Some(file),
            Ok(file) => stdout = Some(file),
            Err(error) => {
                eprintln!("{}: {}", parts[position + 1], error);
                return None;
            }
        }

        parts.drain(position..=position + 1);
    }

    Some(Redirection { stdout, stderr })
}

fn write_output(output: &mut Option<File>, text: &str) -> io::Result<()> {
    match output {
        Some(file) => writeln!(file, "{text}"),
        None => writeln!(io::stdout().lock(), "{text}"),
    }
}

// ─── Builtin Commands ───────────────────────────────────────────────────────

const BUILTINS: &[&str] = &["type", "exit", "echo", "pwd", "cd", "complete", "jobs"];

/// Runs a builtin command. Returns `Ok(true)` when the shell should
/// `continue` to the next iteration (i.e. skip the `result` error check),
/// `Ok(false)` when the caller should inspect the `io::Result`, and
/// `Err(result)` to hand back an `io::Result` for error reporting.
enum BuiltinOutcome {
    /// The command was fully handled; go to the next loop iteration.
    Continue,
    /// The command produced an `io::Result` that should be checked.
    IoResult(io::Result<()>),
}

fn run_builtin(
    program: &str,
    parts: &[String],
    output: &mut Option<File>,
    completions: &Arc<Mutex<HashMap<String, String>>>,
) -> Option<BuiltinOutcome> {
    match program {
        "echo" => Some(BuiltinOutcome::IoResult(write_output(
            output,
            &parts[1..].join(" "),
        ))),
        "pwd" => Some(run_pwd(output)),
        "type" => Some(run_type(parts, output)),
        "cd" => Some(run_cd(parts)),
        "complete" => Some(run_complete(parts, output, completions)),
        "jobs" => Some(BuiltinOutcome::Continue),
        _ => None,
    }
}

fn run_complete(
    parts: &[String],
    output: &mut Option<File>,
    completions: &Arc<Mutex<HashMap<String, String>>>,
) -> BuiltinOutcome {
    match parts.get(1).map(String::as_str) {
        Some("-C") => {
            if let (Some(path), Some(name)) = (parts.get(2), parts.get(3)) {
                match completions.lock() {
                    Ok(mut completions) => {
                        completions.insert(name.clone(), path.clone());
                    }
                    Err(error) => {
                        eprintln!("complete: failed to register completion: {error}");
                    }
                }
            }
        }
        Some("-r") => {
            if let Some(name) = parts.get(2) {
                match completions.lock() {
                    Ok(mut completions) => {
                        completions.remove(name);
                    }
                    Err(error) => {
                        eprintln!("complete: failed to remove completion: {error}");
                    }
                }
            }
        }
        Some("-p") => {
            if let Some(name) = parts.get(2) {
                let path = match completions.lock() {
                    Ok(completions) => completions.get(name).cloned(),
                    Err(error) => {
                        eprintln!("complete: failed to access registrations: {error}");
                        return BuiltinOutcome::Continue;
                    }
                };
                if let Some(path) = path {
                    return BuiltinOutcome::IoResult(write_output(
                        output,
                        &format!("complete -C '{path}' {name}"),
                    ));
                }
                eprintln!("complete: {name}: no completion specification");
            }
        }
        _ => {}
    }
    BuiltinOutcome::Continue
}

fn run_pwd(output: &mut Option<File>) -> BuiltinOutcome {
    match env::current_dir() {
        Ok(path) => BuiltinOutcome::IoResult(write_output(output, &path.display().to_string())),
        Err(error) => {
            eprintln!("pwd: {error}");
            BuiltinOutcome::Continue
        }
    }
}

fn run_type(parts: &[String], output: &mut Option<File>) -> BuiltinOutcome {
    let Some(name) = parts.get(1) else {
        return BuiltinOutcome::Continue;
    };
    let text = if BUILTINS.contains(&name.as_str()) {
        format!("{name} is a shell builtin")
    } else {
        match which::which(name) {
            Ok(path) => format!("{name} is {}", path.display()),
            Err(_) => format!("{name}: not found"),
        }
    };
    BuiltinOutcome::IoResult(write_output(output, &text))
}

fn run_cd(parts: &[String]) -> BuiltinOutcome {
    let Some(path) = parts.get(1) else {
        return BuiltinOutcome::Continue;
    };
    let destination = if path == "~" {
        match env::var("HOME") {
            Ok(home) => home,
            Err(_) => {
                eprintln!("cd: HOME not set");
                return BuiltinOutcome::Continue;
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
    BuiltinOutcome::Continue
}

// ─── External Commands ──────────────────────────────────────────────────────

fn run_external(program: &str, parts: &[String], redirection: Redirection) {
    let mut child = Command::new(program);
    child.args(&parts[1..]);
    if let Some(file) = redirection.stdout {
        child.stdout(Stdio::from(file));
    }
    if let Some(file) = redirection.stderr {
        child.stderr(Stdio::from(file));
    }
    match child.status() {
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            eprintln!("{program}: command not found");
        }
        Err(error) => eprintln!("{program}: {error}"),
    }
}

// ─── Main Loop ──────────────────────────────────────────────────────────────

fn main() {
    let completions = Arc::new(Mutex::new(HashMap::new()));
    let config = Config::builder()
        .completion_type(CompletionType::List)
        .completion_show_all_if_ambiguous(false)
        .bell_style(BellStyle::Audible)
        .build();
    let mut editor =
        match Editor::<ShellHelper, rustyline::history::DefaultHistory>::with_config(config) {
            Ok(editor) => editor,
            Err(error) => {
                eprintln!("shell: failed to initialize input editor: {error}");
                return;
            }
        };
    editor.set_helper(Some(ShellHelper {
        completion_scripts: Arc::clone(&completions),
    }));

    loop {
        let input = match editor.readline("$ ") {
            Ok(input) => input,
            Err(rustyline::error::ReadlineError::Interrupted)
            | Err(rustyline::error::ReadlineError::Eof) => break,
            Err(error) => {
                eprintln!("shell: failed to read command: {error}");
                break;
            }
        };
        let _ = editor.add_history_entry(input.as_str());

        let mut parts = parse_arguments(&input);

        if parts.first().is_some_and(|part| part == "exit") {
            break;
        }

        // ── Resolve redirections ────────────────────────────────────────
        let Some(mut redirection) = extract_redirections(&mut parts) else {
            continue;
        };

        // Guard: lingering redirection operators mean a parse error occurred.
        if parts
            .iter()
            .any(|part| REDIRECTION_OPERATORS.contains(&part.as_str()))
        {
            continue;
        }

        // ── Dispatch command ────────────────────────────────────────────
        let Some(program) = parts.first().cloned() else {
            continue;
        };

        if let Some(outcome) = run_builtin(&program, &parts, &mut redirection.stdout, &completions)
        {
            match outcome {
                BuiltinOutcome::Continue => continue,
                BuiltinOutcome::IoResult(result) => {
                    if let Err(error) = result {
                        eprintln!("{program}: {error}");
                    }
                }
            }
        } else {
            run_external(&program, &parts, redirection);
        }
    }
}
