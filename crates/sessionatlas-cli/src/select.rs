//! Testable interactive selection.
//!
//! Selection reads numbered input from a `BufRead` and writes the prompt to a
//! `Write`, so tests drive it with synthetic input and never launch a
//! terminal or an external process. Task R10 replaces the "open" follow-up;
//! this task only prints the chosen label.

use std::io::{self, BufRead, Write};

/// Result of an interactive selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Selection {
    /// The user cancelled (0, EOF, or an empty list).
    Cancel,
    /// The user chose `choices[index]`.
    Chosen(usize),
}

/// Prompts for a numbered choice among `choices` (1-based, plus `0` to
/// cancel). Invalid input re-prompts; EOF cancels. An empty choice list
/// immediately cancels.
pub fn prompt_select(
    reader: &mut dyn BufRead,
    writer: &mut dyn Write,
    title: &str,
    choices: &[String],
) -> io::Result<Selection> {
    if choices.is_empty() {
        return Ok(Selection::Cancel);
    }
    writeln!(writer, "\n{title}")?;
    for (index, choice) in choices.iter().enumerate() {
        writeln!(writer, "  {}) {choice}", index + 1)?;
    }
    writeln!(writer, "  0) [取消]")?;
    loop {
        write!(writer, "请输入选择 (0-{}): ", choices.len())?;
        writer.flush()?;
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            return Ok(Selection::Cancel);
        }
        match line.trim().parse::<usize>() {
            Ok(0) => return Ok(Selection::Cancel),
            Ok(number) if (1..=choices.len()).contains(&number) => {
                return Ok(Selection::Chosen(number - 1));
            }
            _ => writeln!(writer, "无效选择，请重新输入。")?,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prompt(input: &str, choices: &[String]) -> (Selection, String) {
        let mut reader = io::Cursor::new(input.as_bytes().to_vec());
        let mut output = Vec::new();
        let selection = prompt_select(&mut reader, &mut output, "标题", choices).unwrap();
        (selection, String::from_utf8(output).unwrap())
    }

    #[test]
    fn empty_choices_cancel_immediately() {
        assert_eq!(prompt("", &[]).0, Selection::Cancel);
    }

    #[test]
    fn numeric_choice_selects_correct_index() {
        let choices = vec!["one".to_string(), "two".to_string()];
        assert_eq!(prompt("1\n", &choices).0, Selection::Chosen(0));
        assert_eq!(prompt("2\n", &choices).0, Selection::Chosen(1));
    }

    #[test]
    fn zero_or_eof_cancels() {
        let choices = vec!["one".to_string()];
        assert_eq!(prompt("0\n", &choices).0, Selection::Cancel);
        assert_eq!(prompt("", &choices).0, Selection::Cancel);
    }

    #[test]
    fn invalid_input_reprompts_then_accepts() {
        let choices = vec!["one".to_string(), "two".to_string()];
        let (selection, output) = prompt("abc\n99\n2\n", &choices);
        assert_eq!(selection, Selection::Chosen(1));
        assert!(output.contains("无效选择，请重新输入。"), "{output}");
    }

    #[test]
    fn prompt_lists_choices_and_cancel_option() {
        let (_, output) = prompt("0\n", &["one".to_string()]);
        assert!(output.contains("1) one"), "{output}");
        assert!(output.contains("0) [取消]"), "{output}");
        assert!(output.contains("请输入选择 (0-1):"), "{output}");
    }
}
