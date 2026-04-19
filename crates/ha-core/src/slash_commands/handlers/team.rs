use crate::slash_commands::types::{CommandAction, CommandResult};

pub fn handle_team(args: &str) -> Result<CommandResult, String> {
    let message = if args.is_empty() {
        "/team status".to_string()
    } else {
        format!("/team {}", args)
    };
    Ok(CommandResult {
        content: String::new(),
        action: Some(CommandAction::PassThrough { message }),
    })
}
