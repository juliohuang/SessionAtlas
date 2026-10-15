use std::env;
use std::fs::{self, OpenOptions};
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

const FIXTURE_ROOT: &str = env!("SESSIONATLAS_FIXTURE_ROOT");

fn append_startup_record(root: &Path, args: &[String]) -> io::Result<()> {
    let canonical_root = root.canonicalize()?;
    let record_path = canonical_root.join("tool-startup-args.txt");
    if record_path.exists() {
        let metadata = fs::symlink_metadata(&record_path)?;
        if !metadata.file_type().is_file() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "startup record is not a regular file",
            ));
        }
        if record_path.canonicalize()?.parent() != Some(canonical_root.as_path()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "startup record escaped the fixture root",
            ));
        }
    }
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&record_path)?;
    writeln!(file, "launch")?;
    for argument in args {
        writeln!(file, "arg={argument}")?;
    }
    Ok(())
}

fn main() -> io::Result<()> {
    let mut args = env::args().skip(1).collect::<Vec<_>>();
    if args.as_slice() == ["--version"] {
        println!("acceptance-tui 1.0.0");
        return Ok(());
    }

    let fixture_root = PathBuf::from(FIXTURE_ROOT);
    append_startup_record(&fixture_root, &args)?;
    println!("SYNTHETIC ACCEPTANCE ONLY — no AI provider or network connection");
    println!("Local review link: http://127.0.0.1:48765/");
    if args.is_empty() {
        println!("Arguments: (none)");
    } else {
        println!("Arguments:");
        for argument in &args {
            println!("  {argument}");
        }
    }
    println!("Type plain text to echo, or 'quit' to exit.");
    io::stdout().flush()?;

    for line in io::stdin().lock().lines() {
        let line = line?;
        if line.trim() == "quit" {
            break;
        }
        println!("echo: {line}");
        io::stdout().flush()?;
    }
    args.clear();
    Ok(())
}
