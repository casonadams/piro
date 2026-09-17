#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentMessage {
    User(String),
    Thought(String),
    ToolCall { name: String, input: String },
    Observation { output: String, success: bool },
    Response(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepResult {
    Thought(String),
    CallTool { tool: String, input: String },
    FinalAnswer(String),
}

pub trait AgentBackend: Send + Sync {
    fn next_step(
        &mut self,
        history: &[AgentMessage],
    ) -> impl std::future::Future<Output = Result<StepResult, String>> + Send;
}

#[cfg(test)]
#[derive(Default)]
pub struct MockAgentBackend {
    steps: Vec<StepResult>,
    step_idx: usize,
}

#[cfg(test)]
impl MockAgentBackend {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_steps(steps: Vec<StepResult>) -> Self {
        Self { steps, step_idx: 0 }
    }
}

#[cfg(test)]
impl AgentBackend for MockAgentBackend {
    async fn next_step(&mut self, history: &[AgentMessage]) -> Result<StepResult, String> {
        if let Some(step) = self.steps.get(self.step_idx).cloned() {
            self.step_idx += 1;
            return Ok(step);
        }

        let prompt = history
            .iter()
            .find_map(|m| match m {
                AgentMessage::User(u) => Some(u.as_str()),
                _ => None,
            })
            .unwrap_or("");

        Ok(resolve_demo_step(prompt, history))
    }
}

#[cfg(test)]
fn resolve_demo_step(prompt: &str, history: &[AgentMessage]) -> StepResult {
    let (tool_cmd, thought) = if prompt.contains("count") || prompt.contains("list") {
        (Some("ls -1"), "Listing directory files...")
    } else if prompt.contains("git") || prompt.contains("status") {
        (Some("git status --short"), "Checking repository status...")
    } else {
        (None, "Analyzing request...")
    };

    if !history.iter().any(|m| matches!(m, AgentMessage::Thought(_))) {
        return StepResult::Thought(thought.into());
    }

    if let Some((output, ok)) = history.iter().rev().find_map(|m| match m {
        AgentMessage::Observation { output, success } => Some((output.clone(), *success)),
        _ => None,
    }) {
        let msg = if ok {
            format!(
                "Observation processed successfully ({} lines):\n{}",
                output.lines().count(),
                output
            )
        } else {
            format!("Tool execution failed:\n{}", output)
        };
        return StepResult::FinalAnswer(msg);
    }

    match tool_cmd {
        Some(cmd) => StepResult::CallTool {
            tool: "bash".into(),
            input: cmd.into(),
        },
        None => StepResult::FinalAnswer(format!("Processed: '{}'", prompt)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_mock_scripted_steps() {
        let mut backend = MockAgentBackend::from_steps(vec![
            StepResult::Thought("thinking".into()),
            StepResult::CallTool {
                tool: "bash".into(),
                input: "echo 1".into(),
            },
            StepResult::FinalAnswer("finished".into()),
        ]);

        assert_eq!(
            backend.next_step(&[]).await.unwrap(),
            StepResult::Thought("thinking".into())
        );
        assert_eq!(
            backend.next_step(&[]).await.unwrap(),
            StepResult::CallTool {
                tool: "bash".into(),
                input: "echo 1".into()
            }
        );
        assert_eq!(
            backend.next_step(&[]).await.unwrap(),
            StepResult::FinalAnswer("finished".into())
        );
    }

    #[tokio::test]
    async fn test_mock_rule_based() {
        let mut backend = MockAgentBackend::new();
        let step = backend.next_step(&[]).await.unwrap();
        assert!(matches!(step, StepResult::Thought(_)));
    }
}
