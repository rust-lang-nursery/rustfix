#[macro_use]
extern crate serde_derive;
extern crate serde_json;
#[macro_use]
extern crate quick_error;
#[macro_use]
extern crate clap;
extern crate colored;

extern crate rustfix;

use std::fs::File;
use std::io::{Read, Write};
use std::error::Error;
use std::process::Command;
use std::collections::{HashMap, HashSet};

use colored::Colorize;
use clap::{Arg, App};

use rustfix::{Suggestion, LineRange, LinePosition};
use rustfix::diagnostics::Diagnostic;

const USER_OPTIONS: &'static str = "What do you want to do? \
    [r]eplace | [s]kip | save and [q]uit | [a]bort (without saving)";

fn main() {
    let program = try_main();
    match program {
        Ok(_) => std::process::exit(0),
        Err(ProgramError::UserAbort) => {
            writeln!(std::io::stdout(), "{}", ProgramError::UserAbort).unwrap();
            std::process::exit(0);
        }
        Err(error) => {
            writeln!(std::io::stderr(), "An error occured: {}", error).unwrap();
            writeln!(std::io::stderr(), "{:?}", error).unwrap();
            if let Some(cause) = error.cause() {
                writeln!(std::io::stderr(), "Cause: {:?}", cause).unwrap();
            }
            std::process::exit(1);
        }
    }
}

macro_rules! flush {
    () => (try!(std::io::stdout().flush());)
}

fn try_main() -> Result<(), ProgramError> {
    let matches = App::new("rustfix")
        .about("Automatically apply suggestions made by rustc")
        .version(crate_version!())
        .arg(Arg::with_name("clippy")
            .long("clippy")
            .help("Use `cargo clippy` for suggestions"))
        .arg(Arg::with_name("yolo")
            .long("yolo")
            .help("Automatically apply all unambiguous suggestions"))
        .arg(Arg::with_name("apply-only-use")
             .long("apply-only-use")
             .help("Apply only use fix suggestions"))
        .get_matches();

    let mut extra_args = Vec::new();
    
    if !matches.is_present("clippy") {
        extra_args.push("-Aclippy");
    }

    let mode = if matches.is_present("yolo") {
        AutofixMode::Yolo
    } else {
        AutofixMode::None
    };

    let apply_only_use = matches.is_present("apply-only-use");

    // Get JSON output from rustc...
    let json = get_json(&extra_args)?;

    let mut suggestions: Vec<Suggestion> = json.lines()
        .filter(not_empty)
        // Convert JSON string (and eat parsing errors)
        .flat_map(|line| serde_json::from_str::<CargoMessage>(line))
        // One diagnostic line might have multiple suggestions
        .flat_map(|cargo_msg| rustfix::collect_suggestions(&cargo_msg.message, None))
        .filter(|v| {
            if apply_only_use {
                v.is_use_suggestion()
            } else {
                true
            }
        })
        .collect();

    comb_use_suggestions(&mut suggestions);

    try!(handle_suggestions(&suggestions, mode));

    Ok(())
}

#[derive(Deserialize)]
struct CargoMessage {
    message: Diagnostic,
}

fn get_json(extra_args: &[&str]) -> Result<String, ProgramError> {
    let output = try!(Command::new("cargo")
        .args(&["clippy", "--message-format", "json"])
        .arg("--")
        .args(extra_args)
        .output());

    Ok(String::from_utf8(output.stdout)?)
}

#[derive(PartialEq, Eq, Debug)]
enum AutofixMode {
    /// Do not apply any fixes automatically
    None,
    // /// Only apply suggestions of a whitelist of lints
    // Whitelist,
    // /// Check the confidence flag supplied by rustc
    // Confidence,
    /// Automatically apply all unambiguous suggestions
    Yolo,
}

fn handle_suggestions(suggestions: &[Suggestion], mode: AutofixMode) -> Result<(), ProgramError> {
    let mut accepted_suggestions: Vec<&Suggestion> = vec![];

    if suggestions.is_empty() {
        println!("I don't have any suggestions for you right now. Check back later!");
        return Ok(());
    }

    'suggestions: for suggestion in suggestions {
        println!("\n\n{info}: {message}\n\
            {arrow} {file}:{range}\n\
            {suggestion}\n\n\
            {lead}{text}{tail}\n\n\
            {with}\n\n\
            {replacement}\n",
            info = "Info".green().bold(),
            message = split_at_lint_name(&suggestion.message),
            arrow = "  -->".blue().bold(),
            suggestion = "Suggestion - Replace:".yellow().bold(),
            file = suggestion.file_name,
            range = suggestion.line_range,
            lead = indent(4, &suggestion.text.0),
            text = suggestion.text.1.red(),
            tail = suggestion.text.2,
            with = "with:".yellow().bold(),
            replacement = indent(4, &suggestion.replacement));
        
        if mode == AutofixMode::Yolo {
            accepted_suggestions.push(suggestion);
            println!("automatically applying suggestion (--yolo)");
            continue 'suggestions;
        }

        'userinput: loop {
            print!("{arrow} {user_options}\n\
                {prompt} ",
                arrow = "==>".green().bold(),
                prompt = "  >".green().bold(),
                user_options = USER_OPTIONS.green());

            flush!();
            let mut input = String::new();
            try!(std::io::stdin().read_line(&mut input));

            match input.trim() {
                "s" => {
                    println!("Skipped.");
                    continue 'suggestions;
                }
                "r" => {
                    accepted_suggestions.push(suggestion);
                    println!("Suggestion accepted. I'll remember that and apply it later.");
                    continue 'suggestions;
                }
                "q" => {
                    println!("Thanks for playing!");
                    break 'suggestions;
                }
                "a" => {
                    return Err(ProgramError::UserAbort);
                }
                _ => {
                    println!("{error}: I didn't quite get that. {user_options}",
                                error = "Error".red().bold(),
                                user_options = USER_OPTIONS);
                    continue 'userinput;
                }
            }
        }
    }

    if !accepted_suggestions.is_empty() {
        println!("Good work. Let me just apply these {} changes!",
                 accepted_suggestions.len());

        for suggestion in accepted_suggestions.iter().rev() {
            try!(apply_suggestion(suggestion));

            print!(".");
            flush!();
        }

        println!("\nDone.");
    }

    Ok(())
}

quick_error! {
    /// All possible errors in programm lifecycle
    #[derive(Debug)]
    pub enum ProgramError {
        UserAbort {
            display("Let's get outta here!")
        }
        /// Missing File
        NoFile {
            display("No input file given")
        }
        SubcommandError(subcommand: String, output: String) {
            display("Error executing subcommand `{}`", subcommand)
            description(output)
        }
        /// Error while dealing with file or stdin/stdout
        Io(err: std::io::Error) {
            from()
            cause(err)
            display("I/O error")
            description(err.description())
        }
        Utf8Error(err: std::string::FromUtf8Error) {
            from()
            display("Error reading input as UTF-8")
        }
        /// Error with deserialization
        Serde(err: serde_json::Error) {
            from()
            cause(err)
            display("Serde JSON error")
            description(err.description())
        }
    }
}

// Helpers
// -------

fn read_file_to_string(file_name: &str) -> Result<String, std::io::Error> {
    let mut file = try!(File::open(file_name));
    let mut buffer = String::new();
    try!(file.read_to_string(&mut buffer));
    Ok(buffer)
}

fn not_empty(s: &&str) -> bool {
    !s.trim().is_empty()
}

fn split_at_lint_name(s: &str) -> String {
    s.split(", #[")
        .collect::<Vec<_>>()
        .join("\n      #[") // Length of whitespace == length of "Info: "
}

fn indent(size: u32, s: &str) -> String {
    let whitespace: String = std::iter::repeat(' ').take(size as usize).collect();

    s.lines()
        .map(|l| format!("{}{}", whitespace, l))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Apply suggestion to a file
///
/// Please beware of ugly hacks below! Originally, I wanted to replace byte ranges, but sadly the
/// ranges rustc's JSON output gives me do not correspond to the parts of the file they are meant
/// to correspond to. So, for now, let's just replace lines!
///
/// This function is as stupid as possible. Make sure you call for the replacemnts in one file in
/// reverse order to not mess up the lines for replacements further down the road.
fn apply_suggestion(suggestion: &Suggestion) -> Result<(), ProgramError> {
    use std::cmp::max;

    let file_content = try!(read_file_to_string(&suggestion.file_name));
    let mut new_content = String::new();

    // Add the lines before the section we want to replace
    new_content.push_str(&file_content.lines()
        .take(max(suggestion.line_range.start.line - 1, 0) as usize)
        .collect::<Vec<_>>()
        .join("\n"));
    new_content.push_str("\n");

    // Parts of line before replacement
    new_content.push_str(&file_content.lines()
        .nth(suggestion.line_range.start.line - 1)
        .unwrap_or("")
        .chars()
        .take(suggestion.line_range.start.column - 1)
        .collect::<String>());

    // Insert new content! Finally!
    new_content.push_str(&suggestion.replacement);

    // Parts of line after replacement
    new_content.push_str(&file_content.lines()
        .nth(suggestion.line_range.end.line - 1)
        .unwrap_or("")
        .chars()
        .skip(suggestion.line_range.end.column - 1)
        .collect::<String>());

    // Add the lines after the section we want to replace
    new_content.push_str("\n");
    new_content.push_str(&file_content.lines()
        .skip(suggestion.line_range.end.line as usize)
        .collect::<Vec<_>>()
        .join("\n"));
    new_content.push_str("\n");

    let mut file = try!(File::create(&suggestion.file_name));
    let new_content = new_content.as_bytes();

    try!(file.set_len(new_content.len() as u64));
    try!(file.write_all(&new_content));

    Ok(())
}


/// We need some special care for use suggestions,
/// to eliminate duplicates, for example for HashMap,
/// rustc suggests 4 variants:
/// use std::collections::HashMap;
/// use std::collections::hash_map::HashMap;
/// use std::collections::HashMap;
/// use std::collections::hash_map::HashMap;
///
/// Also it would be good to place them at the top of file
fn comb_use_suggestions(suggestions: &mut Vec<Suggestion>) {
    let mut to_remove = HashSet::new();
    {
        type UseSuggestionId<'a> = (&'a str, LineRange, &'a str);
        let mut use_classes = HashMap::<UseSuggestionId, Vec<usize>>::new();
        for (i, s) in suggestions.iter().enumerate() {
            if s.is_use_suggestion() {
                let short_type_name = {
                    let use_expr = &s.replacement;
                    assert!(use_expr.starts_with("use "));
                    assert!(use_expr.ends_with(";\n"));
                    use_expr.rfind("::").map(|idx|
                                             &use_expr[idx + 2..use_expr.len() - 2])
                        .unwrap_or(&use_expr[4..use_expr.len() - 2])
                };
                let id = (s.file_name.as_str(), s.line_range, short_type_name);
                use_classes.entry(id).or_insert(vec![i]).push(i);
            }
        }

        for (_, class) in &use_classes {
            let min_pos: usize = class[class
                                       .iter()
                                       .map(|v| suggestions[*v].replacement.len())
                                       .enumerate()
                                       .min_by(|l, r| l.1.cmp(&r.1))
                                       .unwrap().0];
            for idx in class.iter() {
                if *idx != min_pos {
                    to_remove.insert(*idx);
                }
            }
        }
    }

    let ret: Vec<_> = suggestions.drain(..)
        .enumerate()
        .filter(|&(i, _)| !to_remove.contains(&i))
        .map(|v| {
            let new_v = Suggestion {
                line_range:  LineRange {
                    start: LinePosition { line: 1, column: 1},
                    end: LinePosition { line: 1, column: 1}
                }, ..v.1
            };
            new_v
        })
        .collect();
    *suggestions = ret;
}
