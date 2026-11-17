use crate::agent::message::ToolCall;

pub const DEFAULT_MAX_AGENT_STEPS: usize = 1000;

pub fn resolved_max_steps() -> Option<usize> {
    match std::env::var("PIRO_MAX_STEPS") {
        Ok(val) => match val.trim().to_lowercase().as_str() {
            "0" | "none" | "unlimited" | "infinite" => None,
            s => s.parse::<usize>().ok().or(Some(DEFAULT_MAX_AGENT_STEPS)),
        },
        Err(_) => Some(DEFAULT_MAX_AGENT_STEPS),
    }
}

#[derive(Default)]
pub struct RepetitionGuard {
    last_call: Option<(String, String)>,
    count: usize,
}

impl RepetitionGuard {
    fn is_repetitive(&mut self, call: &ToolCall) -> bool {
        let key = (call.name.clone(), call.input.to_string());
        if self.last_call.as_ref() == Some(&key) {
            self.count += 1;
            self.count >= 3
        } else {
            self.last_call = Some(key);
            self.count = 1;
            false
        }
    }

    pub fn first_repetitive<'a>(&mut self, calls: &'a [ToolCall]) -> Option<&'a ToolCall> {
        calls.iter().find(|c| self.is_repetitive(c))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn call(command: &str) -> ToolCall {
        ToolCall {
            id: "id".into(),
            name: "bash".into(),
            input: json!({ "command": command }),
        }
    }

    #[test]
    fn repetition_guard_trips_on_third_identical_call() {
        let mut guard = RepetitionGuard::default();
        assert!(!guard.is_repetitive(&call("ls")));
        assert!(!guard.is_repetitive(&call("ls")));
        assert!(guard.is_repetitive(&call("ls")));
        assert!(!guard.is_repetitive(&call("pwd")));
    }
}
