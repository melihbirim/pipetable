use anyhow::Result;
use colored::Colorize;
use futures::StreamExt;
use std::io::Write;

pub const DEFAULT_MODEL: &str = "qwen2.5-coder:1.5b";
const OLLAMA_URL: &str = "http://localhost:11434";

// ─── Provider detection ───────────────────────────────────────────────────────

pub fn active_provider() -> &'static str {
    if std::env::var("ANTHROPIC_API_KEY").is_ok() { "anthropic" }
    else if std::env::var("OPENAI_API_KEY").is_ok() { "openai" }
    else { "ollama" }
}

pub fn default_model() -> &'static str {
    match active_provider() {
        "anthropic" => "claude-haiku-4-5-20251001",
        "openai" => "gpt-4o-mini",
        _ => DEFAULT_MODEL,
    }
}

pub async fn provider_available() -> bool {
    match active_provider() {
        "anthropic" | "openai" => true,
        _ => is_available().await,
    }
}

pub fn provider_label() -> String {
    match active_provider() {
        "anthropic" => "Claude (Anthropic)".into(),
        "openai" => {
            let base = std::env::var("OPENAI_BASE_URL")
                .unwrap_or_else(|_| "OpenAI".into());
            if base == "OpenAI" { base } else { format!("OpenAI-compatible ({base})") }
        }
        _ => format!("Ollama ({})", DEFAULT_MODEL),
    }
}

// ─── Ollama-specific ──────────────────────────────────────────────────────────

pub async fn is_available() -> bool {
    reqwest::get(format!("{OLLAMA_URL}/api/tags")).await.map(|r| r.status().is_success()).unwrap_or(false)
}

pub async fn list_models() -> Result<Vec<String>> {
    let resp: serde_json::Value = reqwest::get(format!("{OLLAMA_URL}/api/tags")).await?.json().await?;
    Ok(resp["models"]
        .as_array()
        .map(|arr| arr.iter().filter_map(|m| m["name"].as_str().map(String::from)).collect())
        .unwrap_or_default())
}

// ─── NL → SQL ─────────────────────────────────────────────────────────────────

pub async fn nl_to_sql(question: &str, schema: &str, model: &str) -> Result<String> {
    match active_provider() {
        "anthropic" => nl_to_sql_anthropic(question, schema, model).await,
        "openai"    => nl_to_sql_openai(question, schema, model).await,
        _           => nl_to_sql_ollama(question, schema, model).await,
    }
}

fn build_prompt(question: &str, schema: &str) -> String {
    format!(
        "You are a DuckDB SQL expert. Given these table schemas:\n\n{schema}\nWrite a DuckDB SQL query to answer: {question}\n\nRules:\n- Return ONLY the SQL query\n- No explanation, no markdown, no backticks\n- Use exact table names from the schema\n- Include LIMIT 100 unless the question asks for all data"
    )
}

async fn nl_to_sql_anthropic(question: &str, schema: &str, model: &str) -> Result<String> {
    let key = std::env::var("ANTHROPIC_API_KEY")?;
    let model = if model == DEFAULT_MODEL { "claude-haiku-4-5-20251001" } else { model };
    eprint!("{}", "Thinking...".dimmed());
    let resp: serde_json::Value = reqwest::Client::new()
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", &key)
        .header("anthropic-version", "2023-06-01")
        .json(&serde_json::json!({
            "model": model,
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": build_prompt(question, schema)}]
        }))
        .send().await?.json().await?;
    eprintln!();
    let sql = strip_markdown_sql(resp["content"][0]["text"].as_str().unwrap_or("").trim());
    println!("{}", highlight_sql(&sql));
    println!();
    Ok(sql)
}

async fn nl_to_sql_openai(question: &str, schema: &str, model: &str) -> Result<String> {
    let key = std::env::var("OPENAI_API_KEY")?;
    let base = std::env::var("OPENAI_BASE_URL").unwrap_or_else(|_| "https://api.openai.com".into());
    let model = if model == DEFAULT_MODEL { "gpt-4o-mini" } else { model };
    eprint!("{}", "Thinking...".dimmed());
    let resp: serde_json::Value = reqwest::Client::new()
        .post(format!("{base}/v1/chat/completions"))
        .header("Authorization", format!("Bearer {key}"))
        .json(&serde_json::json!({
            "model": model,
            "messages": [{"role": "user", "content": build_prompt(question, schema)}]
        }))
        .send().await?.json().await?;
    eprintln!();
    let sql = strip_markdown_sql(resp["choices"][0]["message"]["content"].as_str().unwrap_or("").trim());
    println!("{}", highlight_sql(&sql));
    println!();
    Ok(sql)
}

async fn nl_to_sql_ollama(question: &str, schema: &str, model: &str) -> Result<String> {
    let response = reqwest::Client::new()
        .post(format!("{OLLAMA_URL}/api/generate"))
        .json(&serde_json::json!({ "model": model, "prompt": build_prompt(question, schema), "stream": true }))
        .send().await?;

    let mut stream = response.bytes_stream();
    let mut buffer = String::new();
    let mut full = String::new();

    eprint!("{}", "Thinking".dimmed());
    while let Some(chunk) = stream.next().await {
        buffer.push_str(std::str::from_utf8(&chunk?).unwrap_or(""));
        while let Some(pos) = buffer.find('\n') {
            let line = buffer[..pos].to_string();
            buffer = buffer[pos + 1..].to_string();
            if line.trim().is_empty() { continue; }
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&line) {
                if let Some(token) = json["response"].as_str() {
                    full.push_str(token);
                    eprint!("{}", ".".dimmed());
                    let _ = std::io::stderr().flush();
                }
                if json["done"].as_bool().unwrap_or(false) {
                    eprintln!();
                    let sql = strip_markdown_sql(full.trim());
                    println!("{}", highlight_sql(&sql));
                    println!();
                    return Ok(sql);
                }
            }
        }
    }
    eprintln!();
    Ok(strip_markdown_sql(full.trim()))
}

// ─── Formatting ───────────────────────────────────────────────────────────────

pub fn highlight_sql(sql: &str) -> String {
    const KEYWORDS: &[&str] = &[
        "SELECT", "FROM", "WHERE", "GROUP", "BY", "ORDER", "HAVING",
        "JOIN", "LEFT", "RIGHT", "INNER", "OUTER", "CROSS", "ON", "AS",
        "LIMIT", "OFFSET", "WITH", "UNION", "ALL", "DISTINCT",
        "COUNT", "SUM", "AVG", "MAX", "MIN", "COALESCE", "CAST", "OVER",
        "PARTITION", "BETWEEN", "CASE", "WHEN", "THEN", "ELSE", "END",
        "AND", "OR", "NOT", "IN", "LIKE", "IS", "NULL",
        "DESC", "ASC", "INSERT", "UPDATE", "DELETE", "CREATE", "DROP",
    ];
    let mut out = String::new();
    let mut token = String::new();

    let flush = |tok: &str, out: &mut String| {
        if tok.is_empty() { return; }
        let upper = tok.to_uppercase();
        if KEYWORDS.contains(&upper.as_str()) {
            out.push_str(&tok.to_uppercase().bright_yellow().bold().to_string());
        } else if tok.chars().all(|c| c.is_ascii_digit() || c == '.') {
            out.push_str(&tok.bright_cyan().to_string());
        } else if tok.starts_with('\'') || tok.starts_with('"') {
            out.push_str(&tok.green().to_string());
        } else {
            out.push_str(tok);
        }
    };

    for ch in sql.chars() {
        if ch.is_alphanumeric() || ch == '_' {
            token.push(ch);
        } else {
            flush(&token, &mut out);
            token.clear();
            match ch {
                ',' | ';' => out.push_str(&ch.to_string().dimmed().to_string()),
                '*'       => out.push_str(&"*".bright_yellow().to_string()),
                _         => out.push(ch),
            }
        }
    }
    flush(&token, &mut out);
    out
}

fn strip_markdown_sql(s: &str) -> String {
    let s = s.trim();
    let s = if let Some(inner) = s.strip_prefix("```sql").or_else(|| s.strip_prefix("```")) {
        inner.trim_start_matches('\n')
    } else { s };
    let s = if let Some(inner) = s.strip_suffix("```") { inner.trim_end() } else { s };
    s.trim().to_string()
}
