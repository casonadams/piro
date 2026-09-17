#![allow(dead_code)]

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

#[derive(Default)]
pub struct MockAgentBackend {
    scripted_steps: Vec<StepResult>,
    step_idx: usize,
    rule_based: bool,
}

impl MockAgentBackend {
    pub fn new() -> Self {
        Self {
            scripted_steps: Vec::new(),
            step_idx: 0,
            rule_based: true,
        }
    }

    pub fn from_steps(steps: Vec<StepResult>) -> Self {
        Self {
            scripted_steps: steps,
            step_idx: 0,
            rule_based: false,
        }
    }
}

impl AgentBackend for MockAgentBackend {
    async fn next_step(&mut self, history: &[AgentMessage]) -> Result<StepResult, String> {
        if !self.rule_based {
            if self.step_idx < self.scripted_steps.len() {
                let step = self.scripted_steps[self.step_idx].clone();
                self.step_idx += 1;
                return Ok(step);
            }
            return Ok(StepResult::FinalAnswer("Done.".to_string()));
        }

        // Rule-based demonstration backend
        let user_prompt = history
            .iter()
            .find_map(|m| match m {
                AgentMessage::User(u) => Some(u.as_str()),
                _ => None,
            })
            .unwrap_or("");

        let has_thought = history.iter().any(|m| matches!(m, AgentMessage::Thought(_)));
        let last_observation = history.iter().rev().find_map(|m| match m {
            AgentMessage::Observation { output, success } => Some((output.clone(), *success)),
            _ => None,
        });

        if !has_thought {
            if user_prompt.contains("count") || user_prompt.contains("list") {
                return Ok(StepResult::Thought("Listing directory files...".into()));
            } else if user_prompt.contains("git") || user_prompt.contains("status") {
                return Ok(StepResult::Thought("Checking repository status...".into()));
            } else {
                return Ok(StepResult::Thought("Analyzing request...".into()));
            }
        }

        if let Some((output, success)) = last_observation {
            if success {
                let count = output.lines().count();
                return Ok(StepResult::FinalAnswer(format!(
                    "Observation processed successfully ({} lines):\n{}",
                    count, output
                )));
            } else {
                return Ok(StepResult::FinalAnswer(format!("Tool execution failed:\n{}", output)));
            }
        }

        // Had thought, now call tool
        if user_prompt.contains("count") || user_prompt.contains("list") {
            Ok(StepResult::CallTool {
                tool: "bash".into(),
                input: "ls -1".into(),
            })
        } else if user_prompt.contains("git") || user_prompt.contains("status") {
            Ok(StepResult::CallTool {
                tool: "bash".into(),
                input: "git status --short".into(),
            })
        } else {
            Ok(StepResult::FinalAnswer(format!("I have reviewed: '{}'", user_prompt)))
        }
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

        let step1 = backend.next_step(&[]).await.unwrap();
        assert_eq!(step1, StepResult::Thought("thinking".into()));

        let step2 = backend.next_step(&[]).await.unwrap();
        assert_eq!(
            step2,
            StepResult::CallTool {
                tool: "bash".into(),
                input: "echo 1".into()
            }
        );

        let step3 = backend.next_step(&[]).await.unwrap();
        assert_eq!(step3, StepResult::FinalAnswer("finished".into()));

        let step4 = backend.next_step(&[]).await.unwrap();
        assert_eq!(step4, StepResult::FinalAnswer("Done.".into()));
    }

    #[tokio::test]
    async fn test_mock_rule_based() {
        let mut backend = MockAgentBackend::new();
        let history = vec![AgentMessage::User("count files".into())];

        let step1 = backend.next_step(&history).await.unwrap();
        assert!(matches!(step1, StepResult::Thought(_)));

        let mut history2 = history.clone();
        history2.push(AgentMessage::Thought("Listing directory files...".into()));

        let step2 = backend.next_step(&history2).await.unwrap();
        assert_eq!(
            step2,
            StepResult::CallTool {
                tool: "bash".into(),
                input: "ls -1".into()
            }
        );

        let mut history3 = history2.clone();
        history3.push(AgentMessage::Observation {
            output: "file1\nfile2".into(),
            success: true,
        });

        let step3 = backend.next_step(&history3).await.unwrap();
        assert!(matches!(step3, StepResult::FinalAnswer(_)));
    }
}
