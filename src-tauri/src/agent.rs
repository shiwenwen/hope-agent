use anyhow::Result;
use rig::providers::anthropic;
use rig::completion::Prompt;

pub struct AssistantAgent {
    client: anthropic::Client,
}

impl AssistantAgent {
    pub fn new(api_key: &str) -> Self {
        let client = anthropic::Client::new(api_key);
        Self { client }
    }

    pub async fn chat(&self, message: &str) -> Result<String> {
        let agent = self.client
            .agent("claude-sonnet-4-6")
            .preamble("You are OpenComputer, a personal AI assistant with deep system integration. \
                      You help users interact with their computer naturally and efficiently.")
            .build();

        let response = agent.prompt(message).await?;
        Ok(response)
    }
}
