use clap::{Parser, Subcommand};
use color_eyre::{eyre::eyre, Report, Result};
use derive_more::Display;
use openai_api_rs::v1::api::Client;
use openai_api_rs::v1::chat_completion::{self, ChatCompletionRequest};
use std::fs::{canonicalize, File};
use std::io::{Read, Write};
use std::process::Command;
use std::{env, process::Stdio};
use tracing::{debug, info};
use tracing_subscriber::EnvFilter;

static COMMAND_NAME: &str = "autocommit";

fn setup() -> Result<(), Report> {
    if std::env::var("RUST_LIB_BACKTRACE").is_err() {
        std::env::set_var("RUST_LIB_BACKTRACE", "1")
    }
    color_eyre::install()?;

    if std::env::var("RUST_LOG").is_err() {
        std::env::set_var("RUST_LOG", "info")
    }
    tracing_subscriber::fmt::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    Ok(())
}

/// Search for a pattern in a file and display the lines that contain it.
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
#[command(propagate_version = true)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    Run {
        path: std::path::PathBuf,
    },
    Create {
        /// Path to the git repo.
        #[clap(long, short = 'p')]
        path: std::path::PathBuf,

        /// Minutes between autocommits
        #[clap(long, short = 'f')]
        frequency: u32,
    },
    /// List currently configured autocommits.
    List,
    Delete {
        /// Path of autocommit repo to delete.
        path: std::path::PathBuf,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    setup()?;
    let cli = Cli::parse();

    match &cli.command {
        Commands::Run { path } => {
            let path = canonicalize(path)?;
            info!("Running {}", path.display());
            run(path.to_path_buf()).await?;
        }
        Commands::Create { path, frequency } => {
            let path = canonicalize(path)?;
            info!(
                "Creating autocommit on {} with frequency {}",
                path.display(),
                frequency
            );
            // Check if autocommit exists on path.
            let mut autocommits = list()?;
            for autocommit in autocommits.iter() {
                if autocommit.command == path.to_str().unwrap() {
                    return Err(eyre!("Autocommit already exists on path"));
                }
            }

            autocommits.push(CronLine::new(
                [
                    format!("*/{}", frequency).to_string(),
                    "*".to_string(),
                    "*".to_string(),
                    "*".to_string(),
                    "*".to_string(),
                ],
                COMMAND_NAME.to_string(),
                vec![
                    "run".to_string(), // Run our binary.
                    path.to_str().unwrap().to_string(),
                    ">>".to_string(),
                    format!("{}/.autocommit_log", path.to_str().unwrap().to_string()),
                    "2>&1".to_string(),
                ],
            ));
            write_autocommits(&autocommits)?;
        }
        Commands::List => {
            info!("Listing");
            let autocommits = list()?;
            info!("Found {} autocommits", autocommits.len());
            for autocommit in autocommits {
                info!("{}", autocommit);
            }
        }
        Commands::Delete { path } => {
            let path = canonicalize(path)?;
            info!("Deleting {}", path.display());

            // Check if autocommit exists on path.
            let mut autocommits = list()?;
            let mut deleted = false;
            autocommits.retain(|e| {
                // TODO: make this conditional better, and less error prone.
                if e.args[1] != path.to_str().unwrap() {
                    true
                } else {
                    deleted = true;
                    false
                }
            });
            if !deleted {
                return Err(eyre!("Autocommit not found on path {}", path.display()));
            }
            debug!("Autocommits {:?}", autocommits);
            write_autocommits(&autocommits)?;
        }
    }
    Ok(())
}

#[derive(Debug, Default, Display)]
#[display(fmt = "{:?} {:?} {:?}", frequency, command, args)]
struct CronLine {
    frequency: [String; 5],
    command: String,
    args: Vec<String>,
}

impl CronLine {
    fn new(frequency: [String; 5], command: String, args: Vec<String>) -> Self {
        Self {
            frequency,
            command,
            args,
        }
    }

    fn parse(line: &str) -> Result<CronLine> {
        let parts = line.split_whitespace();
        let mut cron_line = CronLine::default();
        for (i, part) in parts.enumerate() {
            match i {
                0..=4 => cron_line.frequency[i] = part.to_string(),
                5 => cron_line.command = part.to_string(),
                _ => cron_line.args.push(part.to_string()),
            }
        }

        if cron_line.command.is_empty() || cron_line.args.is_empty() {
            return Err(eyre!("Invalid cron line, missing parts "));
        }

        for part in cron_line.frequency.iter() {
            if part.is_empty() {
                return Err(eyre!("Invalid cron line frequency, missing parts "));
            }
        }

        Ok(cron_line)
    }

    fn to_string(&self) -> String {
        format!(
            "{} {} {}",
            self.frequency.join(" "),
            self.command,
            self.args.join(" ")
        )
    }
}

// TODO: this prevents the user from running other cron jobs rn :(
fn write_autocommits(autocommits: &Vec<CronLine>) -> Result<()> {
    let mut file = File::create("/tmp/crontab.txt")?;
    let data = format!("OPENAI_API_KEY={}\n\n", env::var("OPENAI_API_KEY")?)
        + &autocommits
            .iter()
            .map(|a| a.to_string())
            .collect::<Vec<String>>()
            .join("\n")
        + "\n";
    file.write_all(data.as_bytes())?;

    // Create cron.
    Command::new("crontab").arg("/tmp/crontab.txt").spawn()?;
    Ok(())
}

fn list() -> Result<Vec<CronLine>> {
    let command = Command::new("crontab")
        .arg("-l")
        .stdout(Stdio::piped())
        .spawn()?;
    let mut command_output = String::new();
    command
        .stdout
        .unwrap()
        .read_to_string(&mut command_output)?;
    let lines = command_output.lines();
    let mut autocommits = Vec::new();
    for line in lines {
        if line.contains(COMMAND_NAME) {
            autocommits.push(CronLine::parse(line)?);
        }
    }
    Ok(autocommits)
}

fn run_command_in_dir(dir: &std::path::PathBuf, command: &str, args: &[&str]) -> Result<String> {
    let mut command = Command::new(command)
        .args(args)
        .current_dir(dir)
        .stdout(Stdio::piped())
        .spawn()?;
    let mut command_output = String::new();
    command
        .stdout
        .unwrap()
        .read_to_string(&mut command_output)?;
    Ok(command_output)
}

// Run command and helpers
async fn run(repo_path: std::path::PathBuf) -> Result<()> {
    // Check if the provided path is a git repo.
    if !repo_path.join(".git").is_dir() {
        return Err(eyre!("Path is not a git repo"));
    }

    // Run `git status` and check if there are any changes.
    let git_status_out = run_command_in_dir(&repo_path, "git", &["status"])?;
    if git_status_out.contains("nothing to commit, working tree clean") {
        info!("no changes: {}", git_status_out);
        return Ok(());
    }

    // Run `git diff` to get the output changes.
    let git_diff_out = run_command_in_dir(&repo_path, "git", &["diff"])?;
    info!("git diff output: {}", git_diff_out);

    Ok(())
}

async fn generate_commit_message(api_key: String, diff_string: &str) -> Result<String> {
    // hehehe
    let prompt = format!("You are CommitBot, an assistant tasked with writing helpful commit messages based on code changes.
      You will be given a set of patches of code changes, and you must write a short commit message describing the changes. Do not be verbose. 
      Your response must include only high level logical changes if the diff is large, otherwise you may include specific changes.
      Try to fit your response in one line.
      \n\n{}", diff_string);

    let client = Client::new(api_key);
    let req = ChatCompletionRequest {
        model: chat_completion::GPT3_5_TURBO.to_string(),
        messages: vec![chat_completion::ChatCompletionMessage {
            role: chat_completion::MessageRole::user,
            content: Some(prompt),
            name: None,
            function_call: None,
        }],
        functions: None,
        function_call: None,
    };

    let resp = client.chat_completion(req).await?;
    let commit_message = resp.choices[0]
        .message
        .content
        .clone()
        .unwrap_or("Could not generate commit message".to_string())
        .to_string();

    Ok(commit_message)
}
