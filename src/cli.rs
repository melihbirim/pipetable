use anyhow::Result;
use colored::Colorize;
use rustyline::completion::{Completer, FilenameCompleter, Pair};
use rustyline::error::ReadlineError;
use rustyline::highlight::Highlighter;
use rustyline::hint::Hinter;
use rustyline::history::DefaultHistory;
use rustyline::validate::Validator;
use rustyline::{Context, Editor, Helper};
use std::borrow::Cow;

use crate::engine::State;
use crate::ollama;

const HELP: &str = r#"
Commands:
  .scan <path>       Load a folder or file (Tab completes paths)
  .datasets          List loaded datasets
  .schema <name>     Show columns + sample rows (Tab completes names)
  .drop <name>       Remove a dataset from the session (Tab completes names)
  .use <n1> <n2>…    Focus NL queries on specific datasets (Tab completes)
  .remove <name>     Remove a dataset from focus (Tab completes active)
  .clear             Clear focus — NL queries use all loaded datasets
  .models            List available Ollama models
  .model <name>      Switch model
  .help              Show this help
  .quit / Ctrl+D     Exit

Querying — SQL:
  SELECT region, SUM(revenue) FROM sales GROUP BY 1
  SELECT * FROM orders WHERE amount > 1000 LIMIT 20

Querying — natural language (requires Ollama):
  show me top 5 customers by revenue
  which region had the highest sales last month?

  Without .use, the most relevant tables are auto-selected from your question.
  Use .use to pin specific datasets when you know what you're working with.

Tips:
  · Re-scanning a folder only reloads new or changed files
  · Tab completes dataset names after .use, .remove, FROM, JOIN, .schema
  · Natural language requires Ollama running: ollama serve
"#;

// ─── Tab completion + readline helper ────────────────────────────────────────

struct CliHelper {
    file_completer: FilenameCompleter,
    dataset_names: Vec<String>,
}

impl CliHelper {
    fn new() -> Self {
        Self { file_completer: FilenameCompleter::new(), dataset_names: vec![] }
    }
    fn update_datasets(&mut self, names: Vec<String>) { self.dataset_names = names; }
}

impl Helper for CliHelper {}

impl Completer for CliHelper {
    type Candidate = Pair;

    fn complete(&self, line: &str, pos: usize, ctx: &Context<'_>) -> rustyline::Result<(usize, Vec<Pair>)> {
        // Dot-command completion (no space yet = still typing the command)
        if line.starts_with('.') && !line.contains(' ') {
            const CMDS: &[&str] = &[
                ".scan ", ".datasets", ".schema ", ".drop ", ".use ", ".remove ", ".clear",
                ".models", ".model ", ".help", ".quit",
            ];
            let matches: Vec<Pair> = CMDS.iter()
                .filter(|c| c.trim_end().starts_with(line))
                .map(|c| Pair { display: c.trim_end().to_string(), replacement: c.to_string() })
                .collect();
            if !matches.is_empty() {
                return Ok((0, matches));
            }
        }

        // Path completion after .scan
        if line.starts_with(".scan ") {
            return self.file_completer.complete(line, pos, ctx);
        }

        // Dataset name completion after .schema, .use, FROM, JOIN
        let prefix = &line[..pos];
        let upper = prefix.to_uppercase();
        let name_start = [" FROM ", " JOIN ", ".schema ", ".drop ", ".use "]
            .iter()
            .filter_map(|kw| upper.rfind(kw).map(|i| i + kw.len()))
            .max();
        if let Some(start) = name_start {
            let partial = &prefix[start..];
            // For .use, exclude names already on the same line
            let already: Vec<&str> = if line.starts_with(".use ") {
                line[".use ".len()..pos].split_whitespace().collect()
            } else {
                vec![]
            };
            let matches: Vec<Pair> = self.dataset_names.iter()
                .filter(|n| n.to_lowercase().starts_with(&partial.to_lowercase()))
                .filter(|n| !already.contains(&n.as_str()))
                .map(|n| Pair { display: n.clone(), replacement: format!("{} ", n) })
                .collect();
            if !matches.is_empty() {
                return Ok((start, matches));
            }
        }

        Ok((pos, vec![]))
    }
}

impl Hinter for CliHelper {
    type Hint = String;
    fn hint(&self, _line: &str, _pos: usize, _ctx: &Context<'_>) -> Option<String> { None }
}

impl Highlighter for CliHelper {
    fn highlight_prompt<'b, 's: 'b, 'p: 'b>(&'s self, prompt: &'p str, _default: bool) -> Cow<'b, str> {
        Cow::Borrowed(prompt)
    }
    fn highlight_hint<'h>(&self, hint: &'h str) -> Cow<'h, str> {
        Cow::Borrowed(hint)
    }
    fn highlight<'l>(&self, line: &'l str, _pos: usize) -> Cow<'l, str> {
        Cow::Borrowed(line)
    }
    fn highlight_char(&self, _line: &str, _pos: usize, _forced: bool) -> bool { false }
}

impl Validator for CliHelper {}

// ─── REPL ────────────────────────────────────────────────────────────────────

pub async fn run(path: Option<&str>, model: Option<&str>) -> Result<()> {
    let mut state = State::new()?;
    let mut current_model = model.unwrap_or(ollama::DEFAULT_MODEL).to_string();
    let ollama_ok = ollama::is_available().await;

    eprintln!("{} {}", "Pipetable".bright_yellow().bold(), "https://pipetable.com".dimmed());
    eprintln!("{}", "DuckDB query engine · local files only".dimmed());
    eprintln!();

    if let Some(p) = path {
        state.scan_verbose(p, true);
        eprintln!();
    }

    let example = state.datasets.keys().next().cloned().unwrap_or_else(|| "mytable".to_string());

    if ollama_ok {
        eprintln!("{} {}", "Model:".dimmed(), current_model.dimmed());
        eprintln!();
        eprintln!("{}", "Ask anything:".dimmed());
        eprintln!("  {}", format!("show me top 10 rows from {example}").bright_white());
        eprintln!("  {}", format!("SELECT * FROM {example} LIMIT 10").bright_white());
        eprintln!("{}", "Use .scan <path> to load files. Tab completes paths. .help for all commands.".dimmed());
    } else {
        eprintln!("{}", "⚠  Ollama not running — natural language queries disabled.".yellow().bold());
        eprintln!("   {}", "Start it:  ollama serve".dimmed());
        eprintln!("   {}", format!("Then pull: ollama pull {}", ollama::DEFAULT_MODEL).dimmed());
        eprintln!("   {}", "Not installed? https://ollama.com".dimmed());
        eprintln!();
        eprintln!("{}", "SQL still works:".dimmed());
        eprintln!("  {}", format!("SELECT * FROM {example} LIMIT 10").bright_white());
        eprintln!("{}", "Use .scan <path> to load files. Tab completes paths. .help for all commands.".dimmed());
    }
    eprintln!();

    let mut rl = Editor::<CliHelper, DefaultHistory>::new()?;
    let mut helper = CliHelper::new();
    helper.update_datasets(state.datasets.keys().cloned().collect());
    rl.set_helper(Some(helper));

    let mut use_context: Vec<String> = vec![];

    loop {
        let prompt = if use_context.is_empty() {
            format!("{} ", ">".bright_yellow().bold())
        } else {
            format!("[{}]{} ", use_context.join(" ").dimmed(), ">".bright_yellow().bold())
        };
        match rl.readline(&prompt) {
            Ok(line) => {
                let line = line.trim().to_string();
                if line.is_empty() { continue; }
                let _ = rl.add_history_entry(&line);
                handle_input(&line, &mut state, &mut current_model, ollama_ok, &mut use_context).await;
                if let Some(h) = rl.helper_mut() {
                    h.update_datasets(state.datasets.keys().cloned().collect());
                }
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
        state.scan_verbose(p, false);
        eprintln!();
    }

    if is_sql(question) {
        println!("{}", state.query(question));
        return Ok(());
    }

    let (schema, _) = state.schema_prompt(question, &[]);
    if schema.is_empty() {
        eprintln!("No datasets loaded. Pass a path: pipetable ask \"...\" ~/data/");
        return Ok(());
    }

    if !ollama::is_available().await {
        eprintln!("{}", "⚠  Ollama is not running — cannot process natural language.".yellow().bold());
        eprintln!("   {}", "Start it:  ollama serve".dimmed());
        eprintln!("   {}", format!("Then pull: ollama pull {model}").dimmed());
        eprintln!("   {}", "Not installed? https://ollama.com".dimmed());
        eprintln!("   {}", "For SQL: pipetable ask \"SELECT ...\" ~/data/".dimmed());
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

async fn handle_input(
    line: &str,
    state: &mut State,
    model: &mut String,
    ollama_ok: bool,
    use_context: &mut Vec<String>,
) {
    if let Some(rest) = line.strip_prefix(".scan ") {
        state.scan_verbose(rest.trim(), true);
        println!();
    } else if line == ".datasets" || line == ".list" {
        println!("{}", state.list());
    } else if let Some(rest) = line.strip_prefix(".schema ") {
        println!("{}", state.schema(rest.trim()));
    } else if let Some(rest) = line.strip_prefix(".drop ") {
        println!("{}", state.drop_dataset(rest.trim()));
    } else if let Some(rest) = line.strip_prefix(".use ") {
        let names: Vec<String> = rest.split_whitespace()
            .filter(|n| state.datasets.contains_key(*n))
            .map(String::from)
            .collect();
        let unknown: Vec<&str> = rest.split_whitespace()
            .filter(|n| !state.datasets.contains_key(*n))
            .collect();
        if !unknown.is_empty() {
            println!("{} {}", "Unknown:".yellow(), unknown.join(", "));
        }
        if !names.is_empty() {
            *use_context = names;
            println!("{} {}", "Context:".dimmed(), use_context.iter().map(|n| n.as_str().bright_white().to_string()).collect::<Vec<_>>().join(", "));
        }
    } else if let Some(rest) = line.strip_prefix(".remove ") {
        let name = rest.trim();
        if let Some(pos) = use_context.iter().position(|n| n == name) {
            use_context.remove(pos);
            if use_context.is_empty() {
                println!("{}", "Context cleared — using all loaded datasets.".dimmed());
            } else {
                println!("{} {}", "Context:".dimmed(), use_context.iter().map(|n| n.as_str().bright_white().to_string()).collect::<Vec<_>>().join(", "));
            }
        } else {
            println!("{} {} {}", name.yellow(), "is not in context.".dimmed(), if use_context.is_empty() { "No active context." } else { "Use .use to add it." });
        }
    } else if line == ".clear" {
        use_context.clear();
        println!("{}", "Context cleared — using all loaded datasets.".dimmed());
    } else if line == ".models" {
        match ollama::list_models().await {
            Ok(m) if m.is_empty() => println!("{}", format!("No models. Try: ollama pull {}", ollama::DEFAULT_MODEL).yellow()),
            Ok(m) => { for name in m { println!("  {}", name.bright_white()); } }
            Err(e) => println!("{} {e}", "Error:".red().bold()),
        }
    } else if let Some(rest) = line.strip_prefix(".model ") {
        *model = rest.trim().to_string();
        println!("{} {}", "Model:".dimmed(), model.bright_white());
    } else if line == ".help" {
        println!("{HELP}");
    } else if line == ".quit" || line == ".exit" {
        std::process::exit(0);
    } else if is_sql(line) {
        println!("{}", state.query(line));
        state.save_as_last(line);
        println!("{}", "→ piped as _last".dimmed());
    } else {
        if !ollama_ok {
            println!("{}", "⚠  Ollama is not running — natural language disabled.".yellow());
            println!("   {}", "Run: ollama serve".dimmed());
            println!("   {}", "Or type a SQL query: SELECT ...".dimmed());
            return;
        }

        let (schema, used) = state.schema_prompt(line, use_context);
        if schema.is_empty() {
            println!("{}", "No datasets loaded. Use .scan <path> first.".yellow());
            return;
        }
        let label = if !use_context.is_empty() { "Context:" } else { "Using:" };
        println!("{} {}", label.dimmed(), used.iter().map(|n| n.as_str().bright_white().to_string()).collect::<Vec<_>>().join(", "));

        match ollama::nl_to_sql(line, &schema, model).await {
            Ok(sql) if !sql.is_empty() => {
                println!();
                println!("{}", state.query(&sql));
                state.save_as_last(&sql);
                println!("{}", "→ piped as _last".dimmed());
            }
            Ok(_) => println!("{}", "(no SQL generated)".dimmed()),
            Err(e) => println!("{} {e}", "Ollama error:".red().bold()),
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
