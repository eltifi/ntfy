use clap::{Args, Subcommand, ValueEnum};
use crate::config::Config;
use crate::auth::manager::AuthManager;
use crate::auth::types::Role;
use sqlx::SqlitePool;
use std::sync::Arc;

#[derive(Args, Debug)]
pub struct UserArgs {
    #[command(subcommand)]
    pub command: UserCommands,
}

#[derive(Subcommand, Debug)]
pub enum UserCommands {
    /// Add a new user
    Add {
        username: String,
        #[arg(long, default_value = "user")]
        role: RoleArg,
        #[arg(long)]
        password: Option<String>,
    },
    /// Remove a user
    Del {
        username: String,
    },
    /// Change user password
    ChangePass {
        username: String,
        #[arg(long)]
        password: Option<String>,
    },
    /// Change user role
    ChangeRole {
        username: String,
        role: RoleArg,
    },
    /// List all users
    List,
}

#[derive(ValueEnum, Clone, Debug)]
pub enum RoleArg {
    Admin,
    User,
}

impl From<RoleArg> for Role {
    fn from(r: RoleArg) -> Self {
        match r {
            RoleArg::Admin => Role::Admin,
            RoleArg::User => Role::User,
        }
    }
}

pub async fn handle_user_command(args: UserArgs, config: Config) -> anyhow::Result<()> {
    let auth_pool = SqlitePool::connect(&format!("sqlite:{}", config.auth_file.display())).await?;
    let config_arc = Arc::new(config);
    let manager = AuthManager::new(auth_pool, config_arc).await?;

    match args.command {
        UserCommands::Add { username, role, password } => {
            let pass = match password {
                Some(p) => p,
                None => rpassword::prompt_password("Password: ")?,
            };
            manager.add_user(&username, &pass, role.into()).await?;
            println!("User {} added", username);
        },
        UserCommands::Del { username } => {
            manager.delete_user(&username).await?;
            println!("User {} removed", username);
        },
        UserCommands::ChangePass { username, password } => {
             let pass = match password {
                Some(p) => p,
                None => rpassword::prompt_password("New Password: ")?,
            };
            manager.change_password(&username, &pass).await?;
             println!("Password changed for user {}", username);
        },
        UserCommands::ChangeRole { username, role } => {
            manager.change_role(&username, role.into()).await?;
            println!("Role changed for user {}", username);
        },
        UserCommands::List => {
            let users = manager.list_users().await?;
            println!("{:<20} {:<10}", "USERNAME", "ROLE");
            println!("{:<20} {:<10}", "--------", "----");
            for user in users {
                println!("{:<20} {:<10?}", user.name, user.role);
            }
        }
    }

    Ok(())
}
