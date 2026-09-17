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
    pub fn from_steps(steps: Vec<StepResult>) -> Self {
        Self { steps, step_idx: 0 }
    }
}

#[cfg(test)]
impl AgentBackend for MockAgentBackend {
    async fn next_step(&mut self, _history: &[AgentMessage]) -> Result<StepResult, String> {
        if let Some(step) = self.steps.get(self.step_idx).cloned() {
            self.step_idx += 1;
            return Ok(step);
        }
        Ok(StepResult::FinalAnswer("Done.".into()))
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
        assert_eq!(
            backend.next_step(&[]).await.unwrap(),
            StepResult::FinalAnswer("Done.".into())
        );
    }
}
