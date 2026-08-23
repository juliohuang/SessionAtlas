//! Terminal-safe rendering for the read-only commands.
//!
//! Every value that originates in the index (project names, paths, tool tags,
//! branches, session fields, timestamps) passes through [`sanitize`] before it
//! reaches stdout, so hostile data can never inject ANSI escape sequences,
//! control characters, or extra lines into terminal output.

use sessionatlas_core::model::{Project, Session};

/// Removes C0/C1 control characters and drops ANSI/OSC escape sequences,
/// leaving executable-free plain text. Newlines inside a value are removed so
/// a hostile cell cannot break row layout or the table header.
pub fn sanitize(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(character) = chars.next() {
        match character {
            '\u{001B}' => skip_escape_sequence(&mut chars),
            character if is_terminal_control(character) => {}
            character => out.push(character),
        }
    }
    out
}

/// C0 (0x00..=0x1F, 0x7F) and C1 (0x80..=0x9F) control characters. The C1 range
/// covers single-byte CSI/OSC introducers some terminals interpret directly.
fn is_terminal_control(character: char) -> bool {
    matches!(character, '\u{0000}'..='\u{001F}' | '\u{007F}'..='\u{009F}')
}

/// Consumes the remainder of an ANSI/OSC escape sequence after the leading ESC.
fn skip_escape_sequence(chars: &mut std::str::Chars<'_>) {
    let Some(introducer) = chars.next() else {
        return;
    };
    match introducer {
        // CSI (e.g. `ESC [ 1;31 m`): consume until the final byte 0x40..=0x7E.
        '[' => {
            for next in chars.by_ref() {
                if matches!(next, '\u{0040}'..='\u{007E}') {
                    break;
                }
            }
        }
        // OSC (e.g. `ESC ] 0 ; title BEL` or `ESC ] ... ESC \`): consume until
        // BEL or ST, whichever comes first, or the end of input.
        ']' => loop {
            match chars.next() {
                None => break,
                Some('\u{0007}') => break,
                Some('\u{001B}') => {
                    if chars.next() == Some('\\') {
                        break;
                    }
                }
                Some(_) => {}
            }
        },
        // A bare ESC or an unknown introducer: drop ESC and keep parsing the
        // following character on the next loop iteration.
        _ => {}
    }
}

/// Relative time label mirroring the C# `ListCommand.FormatRelativeTime`.
///
/// Accepts anything convertible to `SystemTime` (the core model timestamps
/// convert via `From<DateTime<Utc>>`), so this module never names chrono types.
pub fn relative_time<TS>(timestamp: TS) -> String
where
    TS: Into<std::time::SystemTime>,
{
    let last = timestamp.into();
    let now = std::time::SystemTime::now();
    let elapsed = match now.duration_since(last) {
        Ok(elapsed) => elapsed,
        Err(_) => return "刚刚".to_string(),
    };
    let minutes = elapsed.as_secs() / 60;
    if minutes < 1 {
        "刚刚".to_string()
    } else if minutes < 60 {
        format!("{minutes}m")
    } else {
        let hours = minutes / 60;
        if hours < 24 {
            format!("{hours}h")
        } else {
            let days = hours / 24;
            if days < 7 {
                format!("{days}d")
            } else {
                format!("{}w", days / 7)
            }
        }
    }
}

/// Formats an RFC 3339 timestamp (e.g. from `DateTime::to_rfc3339`) as
/// `YYYY-MM-DD HH:MM` UTC wall time, mirroring the C# search table.
pub fn format_absolute_time(rfc3339: &str) -> String {
    if rfc3339.len() >= 16 {
        format!(
            "{}-{}-{} {}:{}",
            &rfc3339[0..4],
            &rfc3339[5..7],
            &rfc3339[8..10],
            &rfc3339[11..13],
            &rfc3339[14..16]
        )
    } else {
        rfc3339.to_string()
    }
}

/// Formats an RFC 3339 timestamp as `MM-dd HH:MM`, mirroring the C# recent
/// table.
pub fn format_mm_dd_hm(rfc3339: &str) -> String {
    if rfc3339.len() >= 16 {
        format!(
            "{}-{} {}:{}",
            &rfc3339[5..7],
            &rfc3339[8..10],
            &rfc3339[11..13],
            &rfc3339[14..16]
        )
    } else {
        rfc3339.to_string()
    }
}

/// Mirrors the C# `Truncate`: keeps the tail of the value and prefixes `...`.
/// Operates on characters so multibyte text is never split mid-sequence.
pub fn truncate(value: &str, max_length: usize) -> String {
    if max_length == 0 {
        return String::new();
    }
    let chars: Vec<char> = value.chars().collect();
    if chars.len() <= max_length {
        return value.to_string();
    }
    let keep = max_length.saturating_sub(3);
    let mut out = String::with_capacity(max_length);
    out.push_str("...");
    out.extend(chars.iter().skip(chars.len().saturating_sub(keep)));
    out
}

/// Aligns a simple single-line text table with two-space column gaps. The last
/// column is left ragged, matching terminal table conventions.
pub fn render_table(header: &[String], rows: &[Vec<String>]) -> String {
    let mut widths: Vec<usize> = header.iter().map(|cell| cell.chars().count()).collect();
    for row in rows {
        for (index, cell) in row.iter().enumerate() {
            if let Some(width) = widths.get_mut(index) {
                *width = (*width).max(cell.chars().count());
            }
        }
    }
    let mut lines = Vec::with_capacity(rows.len() + 2);
    lines.push(format_row(header, &widths));
    lines.push(format_separator(&widths));
    lines.extend(rows.iter().map(|row| format_row(row, &widths)));
    lines.join("\n")
}

/// Renders the `list` table, mirroring the C# column set and truncation.
pub fn render_list(projects: &[Project]) -> String {
    let header = vec![
        "#".to_string(),
        "项目".to_string(),
        "目录".to_string(),
        "工具".to_string(),
        "路径".to_string(),
        "分支".to_string(),
        "最后访问".to_string(),
    ];
    let rows: Vec<Vec<String>> = projects
        .iter()
        .enumerate()
        .map(|(index, project)| {
            vec![
                (index + 1).to_string(),
                truncate(&sanitize(&project.display_name().unwrap_or_default()), 25),
                project_directory_status(project).to_string(),
                truncate(&sanitize(&project.tool_tags()), 18),
                truncate(&sanitize(&project.path), 38),
                truncate(&sanitize(project.git_branch.as_deref().unwrap_or("-")), 15),
                relative_time(project.last_accessed_at),
            ]
        })
        .collect();
    render_table(&header, &rows)
}

/// Renders the `search` table, mirroring the C# column set and the absolute
/// `YYYY-MM-DD HH:MM` timestamp.
pub fn render_search(projects: &[Project]) -> String {
    let header = vec![
        "#".to_string(),
        "项目".to_string(),
        "目录".to_string(),
        "工具".to_string(),
        "路径".to_string(),
        "最后访问".to_string(),
    ];
    let rows: Vec<Vec<String>> = projects
        .iter()
        .enumerate()
        .map(|(index, project)| {
            vec![
                (index + 1).to_string(),
                truncate(&sanitize(&project.display_name().unwrap_or_default()), 25),
                project_directory_status(project).to_string(),
                truncate(&sanitize(&project.tool_tags()), 18),
                truncate(&sanitize(&project.path), 40),
                format_absolute_time(&project.last_accessed_at.to_rfc3339()),
            ]
        })
        .collect();
    render_table(&header, &rows)
}

fn project_directory_status(project: &Project) -> &'static str {
    if project.path_missing {
        "⚠ 缺失"
    } else {
        "存在"
    }
}

/// Renders the `recent` table, mirroring the C# `时间/工具/项目路径` columns.
pub fn render_recent(sessions: &[Session]) -> String {
    let header = vec![
        "时间".to_string(),
        "工具".to_string(),
        "项目路径".to_string(),
    ];
    let rows: Vec<Vec<String>> = sessions
        .iter()
        .map(|session| {
            vec![
                format_mm_dd_hm(&session.started_at.to_rfc3339()),
                sanitize(&session.tool_name),
                sanitize(&session.project_path),
            ]
        })
        .collect();
    render_table(&header, &rows)
}

fn format_row(cells: &[String], widths: &[usize]) -> String {
    let mut line = String::new();
    for (index, width) in widths.iter().enumerate() {
        let cell = cells.get(index).map(String::as_str).unwrap_or("");
        line.push_str(&format!("{cell:<width$}"));
        if index + 1 < widths.len() {
            line.push_str("  ");
        }
    }
    line.trim_end().to_string()
}

fn format_separator(widths: &[usize]) -> String {
    let mut line = String::new();
    for (index, width) in widths.iter().enumerate() {
        line.push_str(&"-".repeat(*width));
        if index + 1 < widths.len() {
            line.push_str("  ");
        }
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_keeps_plain_text() {
        assert_eq!(
            sanitize("plain path with spaces-and_underscores"),
            "plain path with spaces-and_underscores"
        );
    }

    #[test]
    fn sanitize_strips_ansi_csi_sequences() {
        assert_eq!(sanitize("a\u{001B}[31mred"), "ared");
        assert_eq!(sanitize("a\u{001B}[1;2;3m\u{001B}[Kb"), "ab");
    }

    #[test]
    fn sanitize_drops_osc_title_sequences() {
        assert_eq!(sanitize("x\u{001B}]0;title\u{0007}y"), "xy");
        assert_eq!(sanitize("x\u{001B}]0;title\u{001B}\\y"), "xy");
    }

    #[test]
    fn sanitize_removes_control_characters() {
        assert_eq!(sanitize("a\u{0000}b\u{0007}\u{001F}c"), "abc");
        assert_eq!(sanitize("line1\nline2\t"), "line1line2");
        assert_eq!(sanitize("del\u{007F}ete"), "delete");
    }

    #[test]
    fn sanitize_keeps_non_ascii_text() {
        assert_eq!(sanitize("项目: 中文 / café"), "项目: 中文 / café");
    }

    #[test]
    fn relative_time_classifies_elapsed_windows() {
        let now = std::time::SystemTime::now();
        assert_eq!(relative_time(now), "刚刚");
        assert_eq!(
            relative_time(now - std::time::Duration::from_secs(5)),
            "刚刚"
        );
        assert_eq!(
            relative_time(now - std::time::Duration::from_secs(120)),
            "2m"
        );
        assert_eq!(
            relative_time(now - std::time::Duration::from_secs(7_200)),
            "2h"
        );
        assert_eq!(
            relative_time(now - std::time::Duration::from_secs(172_800)),
            "2d"
        );
        assert_eq!(
            relative_time(now - std::time::Duration::from_secs(1_209_600)),
            "2w"
        );
    }

    #[test]
    fn relative_time_handles_future_timestamps_as_just_now() {
        let future = std::time::SystemTime::now() + std::time::Duration::from_secs(60);
        assert_eq!(relative_time(future), "刚刚");
    }

    #[test]
    fn absolute_time_formats_rfc3339() {
        assert_eq!(
            format_absolute_time("2026-07-30T12:00:00+00:00"),
            "2026-07-30 12:00"
        );
        assert_eq!(
            format_absolute_time("2026-07-30T12:00:00.123456+00:00"),
            "2026-07-30 12:00"
        );
        assert_eq!(format_absolute_time("garbage"), "garbage");
    }

    #[test]
    fn mm_dd_hm_formats_rfc3339() {
        assert_eq!(format_mm_dd_hm("2026-07-30T12:00:00+00:00"), "07-30 12:00");
        assert_eq!(format_mm_dd_hm("garbage"), "garbage");
    }

    #[test]
    fn truncate_keeps_tail_with_ellipsis() {
        assert_eq!(truncate("short", 10), "short");
        assert_eq!(truncate("1234567890", 5), "...90");
        assert_eq!(truncate("1234567890", 0), "");
        assert_eq!(truncate("", 5), "");
    }

    #[test]
    fn table_pads_columns() {
        let header = vec!["a".to_string(), "bb".to_string()];
        let rows = vec![vec!["ccc".to_string(), "d".to_string()]];
        assert_eq!(render_table(&header, &rows), "a    bb\n---  --\nccc  d");
    }

    #[test]
    fn list_and_search_render_missing_project_marker() {
        let mut project = Project {
            path: if cfg!(windows) {
                r"C:\gone\missing-project".to_string()
            } else {
                "/gone/missing-project".to_string()
            },
            path_missing: true,
            ..Project::default()
        };
        project.last_accessed_at = std::time::SystemTime::now().into();

        let list = render_list(&[project.clone()]);
        let search = render_search(&[project]);
        assert!(list.contains("目录"));
        assert!(list.contains("⚠ 缺失"));
        assert!(search.contains("目录"));
        assert!(search.contains("⚠ 缺失"));
    }
}
