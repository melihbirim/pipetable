use anyhow::Result;
use rustyline::{DefaultEditor, error::ReadlineError};

use crate::engine::State;
use crate::ollama;

const HELP: &str = r#"
Commands:
  .scan <path>     Load a folder or file
  .datasets        List loaded datasets
  .schema <name>   Show schema for a dataset
  .models          List available Ollama models
  .model <name>    Switch model
  .help            Show this help
  .quit / Ctrl+D   Exit

Anything else is treated as a natural language question (requires Ollama)
or a SQL query (starts with SELECT, WITH, etc.)
"#;

pub async fn run(path: Option<&str>, model: Option<&str>) -> Result<()> {
    let mut state = State::new()?;
    let mut current_model = model.unwrap_or(ollama::DEFAULT_MODEL).to_string();

    // Check Ollama availability once at startup
    let ollama_ok = ollama::is_available().await;

    eprintln!("Pipetable  https://pipetable.com");
    eprintln!("DuckDB query engine · local files only");
    eprintln!();

    // Auto-scan if path given
    if let Some(p) = path {
        let out = state.scan(p);
        println!("{out}");
    }

    if ollama_ok {
        eprintln!("Model: {current_model} (Ollama)");
    } else {
        eprintln!("Ollama not found at localhost:11434 — natural language disabled.");
        eprintln!("  Install: https://ollama.com  then: ollama pull {}", ollama::DEFAULT_MODEL);
        eprintln!("  SQL queries still work.");
    }
    eprintln!("Type .help for commands. Ctrl+D to exit.");
    eprintln!();

    let mut rl = DefaultEditor::new()?;

    loop {
        let readline = rl.readline("> ");
        match readline {
            Ok(line) => {
                let line = line.trim().to_string();
                if line.is_empty() { continue; }
                let _ = rl.add_history_entry(&line);
                handle_input(&line, &mut state, &mut current_model, ollama_ok).await;
            }
            Err(ReadlineError::Interrupted) | Err(ReadlineError::Eof) => break,
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

pub async fn ask(question: &str, path: Option<&str>, model: Option<&str>) -> Result<()> {
    let mut state = State::new()?;
    let model = model.unwrap_or(ollama::DEFAULT_MODEL);

    if let Some(p) = path {
        state.scan(p);
    }

    if is_sql(question) {
        println!("{}", state.query(question));
        return Ok(());
    }

    let schema = state.schema_prompt();
    if schema.is_empty() {
        eprintln!("No datasets loaded. Pass a path: pipetable ask \"...\" ~/data/");
        return Ok(());
    }

    if !ollama::is_available().await {
        eprintln!("Ollama not found at localhost:11434.");
        eprintln!("  Install: https://ollama.com  then: ollama pull {model}");
        eprintln!("  Or use SQL directly: pipetable ask \"SELECT ...\" ~/data/");
        return Ok(());
    }

    let sql = ollama::nl_to_sql(question, &schema, model).await?;
    if !sql.is_empty() {
        println!();
        println!("{}", state.query(&sql));
    }
    Ok(())
}

// ─── Input dispatch ───────────────────────────────────────────────────────────

async fn handle_input(line: &str, state: &mut State, model: &mut String, ollama_ok: bool) {
    if let Some(rest) = line.strip_prefix(".scan ") {
        println!("{}", state.scan(rest.trim()));
    } else if line == ".datasets" || line == ".list" {
        println!("{}", state.list());
    } else if let Some(rest) = line.strip_prefix(".schema ") {
        println!("{}", state.schema(rest.trim()));
    } else if line == ".models" {
        match ollama::list_models().await {
            Ok(m) if m.is_empty() => println!("No models found. Try: ollama pull {}", ollama::DEFAULT_MODEL),
            Ok(m) => { for name in m { println!("  {name}"); } }
            Err(e) => println!("Error: {e}"),
        }
    } else if let Some(rest) = line.strip_prefix(".model ") {
        *model = rest.trim().to_string();
        println!("Model set to: {model}");
    } else if line == ".help" {
        println!("{HELP}");
    } else if line == ".quit" || line == ".exit" {
        std::process::exit(0);
    } else if is_sql(line) {
        println!("{}", state.query(line));
    } else {
        if !ollama_ok {
            println!("Ollama is not running. SQL only mode. Type .help for commands.");
            return;
        }
        let schema = state.schema_prompt();
        if schema.is_empty() {
            println!("No datasets loaded. Use .scan <path> first.");
            return;
        }
        match ollama::nl_to_sql(line, &schema, model).await {
            Ok(sql) if !sql.is_empty() => {
                println!();
                println!("{}", state.query(&sql));
            }
            Ok(_) => println!("(no SQL generated)"),
            Err(e) => println!("Ollama error: {e}"),
        }
    }
}

fn is_sql(s: &str) -> bool {
    let u = s.trim_start().to_uppercase();
    u.starts_with("SELECT ")
        || u.starts_with("WITH ")
        || u.starts_with("INSERT ")
        || u.starts_with("UPDATE ")
        || u.starts_with("DELETE ")
        || u.starts_with("CREATE ")
        || u.starts_with("DESCRIBE ")
        || u == "SELECT" || u == "DESCRIBE"
}
