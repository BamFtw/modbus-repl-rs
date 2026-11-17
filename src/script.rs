//! Script files (`--script PATH`).
//!
//! A script holds the same commands as `--command` / the interactive prompt:
//! one or more commands per line separated by `;`. Everything after `#` is a
//! comment. Scripts typically use the `wait <ms>` command between steps.

use crate::commands;
use std::io::{self, Read};

/// One command of a script together with its (1-based) line number.
#[derive(Debug, PartialEq, Eq)]
pub struct Step {
    pub line: usize,
    pub text: String,
}

/// Split a script into commands, dropping comments and empty commands.
pub fn split(text: &str) -> Vec<Step> {
    let mut steps = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let code = line.split('#').next().unwrap_or_default();
        for cmd in code.split(';').map(str::trim).filter(|c| !c.is_empty()) {
            steps.push(Step { line: n + 1, text: cmd.to_string() });
        }
    }
    steps
}

/// Split a script and check every command before anything is executed, so
/// a typo near the end can't leave the device half configured.
/// Returns all syntax errors as `line N: message`.
pub fn parse(text: &str) -> Result<Vec<Step>, Vec<String>> {
    let steps = split(text);
    let errors: Vec<String> = steps
        .iter()
        .filter_map(|s| commands::parse(&s.text).err().map(|e| format!("line {}: {e} ('{}')", s.line, s.text)))
        .collect();
    if errors.is_empty() {
        Ok(steps)
    } else {
        Err(errors)
    }
}

/// Read a script from `path`, or from standard input when `path` is `-`.
pub fn read(path: &str) -> Result<String, String> {
    if path == "-" {
        let mut text = String::new();
        io::stdin().read_to_string(&mut text).map_err(|e| format!("cannot read script from stdin: {e}"))?;
        return Ok(text);
    }
    std::fs::read_to_string(path).map_err(|e| format!("cannot read script '{path}': {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(line: usize, text: &str) -> Step {
        Step { line, text: text.to_string() }
    }

    #[test]
    fn splitting() {
        let text = "# setup\nwh 0 1 2   # write\n\nwait 100; rh 0 2 ;\n  ; \nri 5 # ; rc 0\r\n";
        assert_eq!(split(text), vec![step(2, "wh 0 1 2"), step(4, "wait 100"), step(4, "rh 0 2"), step(6, "ri 5")]);
    }

    #[test]
    fn validation() {
        assert_eq!(parse("rh 0 2\nwait 5").unwrap().len(), 2);
        let errors = parse("rh 0 2\nwait\nxx 1; ri 0").unwrap_err();
        assert_eq!(errors.len(), 2);
        assert!(errors[0].starts_with("line 2: missing time"));
        assert!(errors[1].starts_with("line 3: invalid action"));
    }
}
