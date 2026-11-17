use std::fs;

use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::agent::tool::{Tool, ToolDefinition, ToolFuture, ToolResult, Workspace, str_arg};

const READ_LIMIT: u64 = 2000;
const MAX_LINE_CHARS: usize = 2000;

type FileOp = fn(&Workspace, &Value) -> Result<String, String>;

pub struct FileTool {
    definition: ToolDefinition,
    workspace: Workspace,
    op: FileOp,
    read_only: bool,
}

impl Tool for FileTool {
    fn definition(&self) -> &ToolDefinition {
        &self.definition
    }

    fn read_only(&self) -> bool {
        self.read_only
    }

    fn run(&self, input: Value, _lines: mpsc::UnboundedSender<String>) -> ToolFuture {
        let result = match (self.op)(&self.workspace, &input) {
            Ok(output) => ToolResult::ok(output),
            Err(e) => ToolResult::err(e),
        };
        Box::pin(std::future::ready(result))
    }
}

fn definition(name: &str, description: &str, schema: Value) -> ToolDefinition {
    ToolDefinition {
        name: name.into(),
        description: description.into(),
        schema,
    }
}

const PATH_ARG: &str = "File path, absolute or relative to the working directory";

pub fn read_tool(workspace: Workspace) -> FileTool {
    FileTool {
        definition: definition(
            "read",
            "Read a text file. Lines come numbered; use offset and limit to page through large files.",
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": PATH_ARG },
                    "offset": { "type": "integer", "description": "First line to read, from 1" },
                    "limit": { "type": "integer", "description": "Number of lines (default 2000)" },
                },
                "required": ["path"],
            }),
        ),
        workspace,
        op: read_file,
        read_only: true,
    }
}

pub fn write_tool(workspace: Workspace) -> FileTool {
    FileTool {
        definition: definition(
            "write",
            "Create or overwrite a file with the given content, creating parent directories.",
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": PATH_ARG },
                    "content": { "type": "string", "description": "The full file content" },
                },
                "required": ["path", "content"],
            }),
        ),
        workspace,
        op: write_file,
        read_only: false,
    }
}

pub fn edit_tool(workspace: Workspace) -> FileTool {
    FileTool {
        definition: definition(
            "edit",
            "Replace an exact string in a file. old_string must match once, including whitespace, \
             unless replace_all is set. Read the file first.",
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": PATH_ARG },
                    "old_string": { "type": "string", "description": "The exact text to replace" },
                    "new_string": { "type": "string", "description": "The replacement text" },
                    "replace_all": { "type": "boolean", "description": "Replace every match" },
                },
                "required": ["path", "old_string", "new_string"],
            }),
        ),
        workspace,
        op: edit_file,
        read_only: false,
    }
}

fn read_file(workspace: &Workspace, input: &Value) -> Result<String, String> {
    let path = workspace.resolve(str_arg(input, "path")?);
    let bytes = fs::read(&path).map_err(|e| format!("Cannot read {}: {e}", path.display()))?;
    if bytes[..bytes.len().min(8192)].contains(&0) {
        return Err(format!("{} is a binary file", path.display()));
    }
    let text = String::from_utf8_lossy(&bytes);
    let skip = input.get("offset").and_then(Value::as_u64).unwrap_or(1).max(1) - 1;
    let limit = input.get("limit").and_then(Value::as_u64).unwrap_or(READ_LIMIT);
    let (skip, limit) = (
        usize::try_from(skip).unwrap_or(usize::MAX),
        usize::try_from(limit).unwrap_or(usize::MAX),
    );
    let mut out = String::new();
    for (i, line) in text.lines().enumerate().skip(skip).take(limit) {
        let line: String = line.chars().take(MAX_LINE_CHARS).collect();
        out.push_str(&format!("{:>6}\t{line}\n", i + 1));
    }
    let rest = text.lines().count().saturating_sub(skip.saturating_add(limit));
    if rest > 0 {
        out.push_str(&format!(
            "... {rest} more lines; continue with offset {}\n",
            skip + limit + 1
        ));
    }
    if out.is_empty() {
        out = "(no lines in range)".into();
    }
    Ok(out)
}

fn write_file(workspace: &Workspace, input: &Value) -> Result<String, String> {
    let path = workspace.resolve(str_arg(input, "path")?);
    let content = str_arg(input, "content")?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("Cannot create {}: {e}", parent.display()))?;
    }
    fs::write(&path, content).map_err(|e| format!("Cannot write {}: {e}", path.display()))?;
    Ok(format!("Wrote {} bytes to {}", content.len(), path.display()))
}

fn edit_file(workspace: &Workspace, input: &Value) -> Result<String, String> {
    let path = workspace.resolve(str_arg(input, "path")?);
    let (old, new) = (str_arg(input, "old_string")?, str_arg(input, "new_string")?);
    let replace_all = input.get("replace_all").and_then(Value::as_bool).unwrap_or(false);
    if old.is_empty() || old == new {
        return Err("old_string must be non-empty and differ from new_string".into());
    }
    let text = fs::read_to_string(&path).map_err(|e| format!("Cannot read {}: {e}", path.display()))?;
    let count = text.matches(old).count();
    if count == 0 {
        return Err(format!("old_string was not found in {}", path.display()));
    }
    if count > 1 && !replace_all {
        return Err(format!(
            "old_string appears {count} times in {}; add surrounding context or set replace_all",
            path.display()
        ));
    }
    let updated = if replace_all {
        text.replace(old, new)
    } else {
        text.replacen(old, new, 1)
    };
    fs::write(&path, updated).map_err(|e| format!("Cannot write {}: {e}", path.display()))?;
    Ok(format!("Edited {} ({count} replaced)", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace(name: &str) -> Workspace {
        let dir = std::env::temp_dir().join(format!("piro-files-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Workspace::new(dir)
    }

    #[test]
    fn write_then_read_pages_numbered_lines() {
        let ws = workspace("read");
        write_file(&ws, &json!({ "path": "a/b.txt", "content": "one\ntwo\nthree\n" })).unwrap();
        let page = read_file(&ws, &json!({ "path": "a/b.txt", "offset": 2, "limit": 1 })).unwrap();
        assert_eq!(page, "     2\ttwo\n... 1 more lines; continue with offset 3\n");
        assert!(read_file(&ws, &json!({ "path": "missing" })).is_err());
    }

    #[test]
    fn edit_requires_a_unique_match_unless_replacing_all() {
        let ws = workspace("edit");
        write_file(&ws, &json!({ "path": "f", "content": "x y x" })).unwrap();
        let ambiguous = edit_file(&ws, &json!({ "path": "f", "old_string": "x", "new_string": "z" }));
        assert!(ambiguous.unwrap_err().contains("appears 2 times"));
        assert!(edit_file(&ws, &json!({ "path": "f", "old_string": "q", "new_string": "z" })).is_err());
        edit_file(&ws, &json!({ "path": "f", "old_string": "y", "new_string": "w" })).unwrap();
        edit_file(
            &ws,
            &json!({ "path": "f", "old_string": "x", "new_string": "z", "replace_all": true }),
        )
        .unwrap();
        assert_eq!(fs::read_to_string(ws.resolve("f")).unwrap(), "z w z");
    }
}
