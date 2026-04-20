//! Tiny, zero-dep (well: rpassword for secure key entry) prompt helpers
//! used by the CLI onboarding wizard and the `hope-agent server setup`
//! subcommand.
//!
//! We deliberately avoid `dialoguer` / `inquire` to keep the release
//! binary small and the escape-code surface predictable. A plain
//! `readline` + parsing loop covers every case we hit in practice.

use std::io::{self, Write};

use anyhow::Result;

/// Color helpers — plain ANSI so Windows terminals with VT enabled render
/// them correctly and piped output stays readable.
pub mod color {
    pub const RESET: &str = "\x1b[0m";
    pub const DIM: &str = "\x1b[2m";
    pub const BOLD: &str = "\x1b[1m";
    pub const CYAN: &str = "\x1b[36m";
    pub const GREEN: &str = "\x1b[32m";
    pub const YELLOW: &str = "\x1b[33m";
    pub const RED: &str = "\x1b[31m";
}

pub fn println_header(title: &str) {
    println!();
    println!(
        "{}╔══════════════════════════════════════════════════════╗{}",
        color::CYAN,
        color::RESET
    );
    println!("{}║  {:<52}║{}", color::CYAN, title, color::RESET);
    println!(
        "{}╚══════════════════════════════════════════════════════╝{}",
        color::CYAN,
        color::RESET
    );
    println!();
}

pub fn println_step(index: u32, total: u32, label: &str) {
    println!();
    println!(
        "{}[{}/{}]{} {}{}{}",
        color::BOLD,
        index,
        total,
        color::RESET,
        color::BOLD,
        label,
        color::RESET,
    );
    println!();
}

pub fn print_saved(message: &str) {
    println!("  {}✓{} {}", color::GREEN, color::RESET, message);
}

pub fn print_skipped(message: &str) {
    println!("  {}⊘{} {}", color::DIM, color::RESET, message);
}

pub fn print_error(message: &str) {
    println!("  {}✗{} {}", color::RED, color::RESET, message);
}

/// Read a single line. Trims trailing whitespace.
fn read_line() -> Result<String> {
    let mut buf = String::new();
    io::stdin().read_line(&mut buf)?;
    Ok(buf.trim_end_matches(['\r', '\n']).to_string())
}

/// Free-form text input with an optional default shown in brackets.
pub fn prompt_input(label: &str, default: Option<&str>) -> Result<String> {
    loop {
        if let Some(d) = default {
            print!("  {} [{}]: ", label, d);
        } else {
            print!("  {}: ", label);
        }
        io::stdout().flush()?;
        let line = read_line()?;
        if line.is_empty() {
            if let Some(d) = default {
                return Ok(d.to_string());
            }
            continue;
        }
        return Ok(line);
    }
}

/// Optional text input — empty response returns `None`.
pub fn prompt_optional(label: &str, default: Option<&str>) -> Result<Option<String>> {
    if let Some(d) = default {
        print!("  {} [{}] (blank to skip): ", label, d);
    } else {
        print!("  {} (blank to skip): ", label);
    }
    io::stdout().flush()?;
    let line = read_line()?;
    if line.is_empty() {
        Ok(default.map(|s| s.to_string()))
    } else {
        Ok(Some(line))
    }
}

/// Yes/no prompt. Accepts y/yes/n/no case-insensitively; empty falls back
/// to `default`.
pub fn prompt_confirm(label: &str, default: bool) -> Result<bool> {
    let hint = if default { "Y/n" } else { "y/N" };
    loop {
        print!("  {} [{}]: ", label, hint);
        io::stdout().flush()?;
        let line = read_line()?.to_lowercase();
        if line.is_empty() {
            return Ok(default);
        }
        match line.as_str() {
            "y" | "yes" => return Ok(true),
            "n" | "no" => return Ok(false),
            _ => println!("    please answer y or n"),
        }
    }
}

/// Numbered select from a list of labels. Returns the 0-based index.
/// `default_idx` activates on empty input.
pub fn prompt_select(label: &str, options: &[&str], default_idx: usize) -> Result<usize> {
    println!("  {}", label);
    for (i, opt) in options.iter().enumerate() {
        println!("    {}) {}", i + 1, opt);
    }
    loop {
        print!("  Choose [{}]: ", default_idx + 1);
        io::stdout().flush()?;
        let line = read_line()?;
        if line.is_empty() {
            return Ok(default_idx);
        }
        if let Ok(n) = line.parse::<usize>() {
            if (1..=options.len()).contains(&n) {
                return Ok(n - 1);
            }
        }
        println!("    please enter a number between 1 and {}", options.len());
    }
}

/// Multi-select: checkboxes over a list. `defaults` (length == options.len)
/// pre-populates the on/off state. Users type comma-separated indices to
/// *toggle* then press enter on empty input to confirm.
pub fn prompt_multiselect(label: &str, options: &[&str], defaults: &[bool]) -> Result<Vec<bool>> {
    let mut selection: Vec<bool> = defaults.to_vec();
    if selection.len() != options.len() {
        selection = vec![true; options.len()];
    }
    println!("  {}", label);
    println!(
        "  {}(toggle with comma-separated numbers, or press Enter to confirm){}",
        color::DIM,
        color::RESET
    );
    loop {
        for (i, opt) in options.iter().enumerate() {
            let mark = if selection[i] { "[x]" } else { "[ ]" };
            println!("    {} {}) {}", mark, i + 1, opt);
        }
        print!("  Toggle: ");
        io::stdout().flush()?;
        let line = read_line()?;
        if line.is_empty() {
            return Ok(selection);
        }
        for tok in line.split(|c: char| c == ',' || c.is_whitespace()) {
            if tok.is_empty() {
                continue;
            }
            if let Ok(n) = tok.parse::<usize>() {
                if (1..=options.len()).contains(&n) {
                    selection[n - 1] = !selection[n - 1];
                    continue;
                }
            }
            println!("    ignoring invalid selector: {}", tok);
        }
    }
}

/// Masked input for secrets. Falls back to cleartext when the terminal
/// doesn't support raw mode (e.g. some CI runners) so we never hard-fail.
pub fn prompt_password(label: &str) -> Result<String> {
    print!("  {}: ", label);
    io::stdout().flush()?;
    match rpassword::read_password() {
        Ok(secret) => Ok(secret),
        Err(_) => read_line(),
    }
}
