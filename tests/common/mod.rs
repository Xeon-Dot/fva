//! Shared test helpers for integration tests.

use fva::indexer::chunker::CodeChunk;

/// Build a CodeChunk for testing
pub fn make_chunk(id: &str, symbol: &str, kind: &str, content: &str, path: &str) -> CodeChunk {
    CodeChunk {
        id: id.to_string(),
        file_path: path.to_string(),
        relative_path: path.to_string(),
        language: "rust".to_string(),
        symbol_name: symbol.to_string(),
        symbol_kind: kind.to_string(),
        start_line: 1,
        end_line: content.lines().count().max(1),
        content: content.to_string(),
        content_hash: "".to_string(),
        line_count: content.lines().count().max(1),
    }
}

pub fn make_chunks() -> Vec<CodeChunk> {
    vec![
        make_chunk(
            "auth:login",
            "login_user",
            "fn",
            "fn login_user(username: &str, password: &str) -> bool {\n    // authenticate user\n    true\n}",
            "src/auth.rs",
        ),
        make_chunk(
            "auth:logout",
            "logout_user",
            "fn",
            "fn logout_user(session: &str) {\n    // clear session\n}",
            "src/auth.rs",
        ),
        make_chunk(
            "auth:validate",
            "validate_token",
            "fn",
            "fn validate_token(token: &str) -> Result<User> {\n    // verify JWT token\n    Ok(User)\n}",
            "src/auth.rs",
        ),
        make_chunk(
            "db:query",
            "query_users",
            "fn",
            "fn query_users(conn: &Connection) -> Result<Vec<User>> {\n    conn.query(\"SELECT * FROM users\")\n}",
            "src/db.rs",
        ),
        make_chunk(
            "db:insert",
            "insert_user",
            "fn",
            "fn insert_user(conn: &Connection, user: &User) -> Result<()> {\n    conn.execute(\"INSERT INTO users VALUES\", user)\n}",
            "src/db.rs",
        ),
        make_chunk(
            "ui:render",
            "render_html",
            "fn",
            "fn render_html(template: &str, data: &Data) -> String {\n    template.render(&data)\n}",
            "src/ui.rs",
        ),
        make_chunk(
            "ui:format",
            "format_date",
            "fn",
            "fn format_date(dt: &DateTime) -> String {\n    dt.format(\"%Y-%m-%d\")\n}",
            "src/ui.rs",
        ),
        make_chunk(
            "sort:bubble",
            "bubble_sort",
            "fn",
            "fn bubble_sort(arr: &mut [i32]) {\n    for i in 0..arr.len() {\n        for j in 0..arr.len()-i-1 {\n            if arr[j] > arr[j+1] {\n                arr.swap(j, j+1);\n            }\n        }\n    }\n}",
            "src/sort.rs",
        ),
        make_chunk(
            "sort:quick",
            "quick_sort",
            "fn",
            "fn quick_sort(arr: &mut [i32]) {\n    if arr.len() <= 1 { return; }\n    let pivot = partition(arr);\n    quick_sort(&mut arr[..pivot]);\n    quick_sort(&mut arr[pivot+1..]);\n}",
            "src/sort.rs",
        ),
        make_chunk(
            "config:parse",
            "parse_config",
            "fn",
            "fn parse_config(path: &Path) -> Result<Config> {\n    let content = std::fs::read_to_string(path)?;\n    toml::from_str(&content)\n}",
            "src/config.rs",
        ),
        make_chunk(
            "http:handle",
            "handle_request",
            "fn",
            "fn handle_request(req: &Request) -> Response {\n    match req.method() {\n        Method::GET => handle_get(req),\n        Method::POST => handle_post(req),\n        _ => Response::not_found(),\n    }\n}",
            "src/http.rs",
        ),
        make_chunk(
            "http:parse",
            "parse_body",
            "fn",
            "fn parse_body(req: &Request) -> Result<Body> {\n    let bytes = req.body_bytes();\n    serde_json::from_slice(&bytes)\n}",
            "src/http.rs",
        ),
        make_chunk(
            "math:add",
            "add_numbers",
            "fn",
            "fn add_numbers(a: i32, b: i32) -> i32 { a + b }",
            "src/math.rs",
        ),
    ]
}
