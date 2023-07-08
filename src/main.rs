use clap::{Parser, Subcommand};
use color_eyre::{eyre::eyre, Report, Result};
use derive_more::Display;
use git2::{DiffOptions, Repository, StatusOptions};
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

// Run command and helpers
async fn run(repo_path: std::path::PathBuf) -> Result<()> {
    let repo = Repository::open(repo_path)?;

    let mut status_opts = StatusOptions::new();
    status_opts.include_untracked(true);

    let has_changes = repo
        .statuses(Some(&mut status_opts))?
        .iter()
        .any(|status| status.status() != git2::Status::CURRENT);

    if !has_changes {
        println!("No changes detected.");
        return Ok(());
    }

    let mut diff_opts = DiffOptions::new();
    let mut diff_opts = diff_opts.include_untracked(true);
    let diff = repo.diff_index_to_workdir(None, Some(&mut diff_opts))?;

    let diff_stats = diff.stats()?;
    let mut diff_string =
        if diff_stats.files_changed() + diff_stats.insertions() + diff_stats.deletions() == 0 {
            String::new()
        } else {
            let mut val = String::new();
            diff.print(git2::DiffFormat::Patch, |_, _, line| {
                match line.origin() {
                    '+' | '-' | ' ' => info!("{}", line.origin()),
                    _ => {}
                }
                val += &format!("{}", String::from_utf8_lossy(line.content()));
                true
            })?;
            val
        };
    debug!("Diff string: {}", diff_string);
    if diff_string.is_empty() {
        info!("No changes to commit, exiting.");
        return Ok(());
    }

    if diff_string.len() > 1000 {
        info!("Diff too large, truncating.");
        diff_string.truncate(1000);
    }

    let commit_message = match env::var("OPENAI_API_KEY") {
        Ok(api_key) => generate_commit_message(api_key, &diff_string).await?,
        Err(_) => chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
    };
    info!("Commit message: {}", commit_message);

    let oid = repo.refname_to_id("HEAD")?;
    let parent = repo.find_commit(oid)?;
    let mut index = repo.index()?;

    index.add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)?;
    index.write()?;
    let tree_oid = index.write_tree()?;

    let tree = repo.find_tree(tree_oid)?;
    let signature = repo.signature()?;

    repo.commit(
        Some("HEAD"),
        &signature,
        &signature,
        &commit_message,
        &tree,
        &[&parent],
    )?;

    let mut remote = repo.find_remote("origin")?;
    let mut callbacks = git2::RemoteCallbacks::new();
    callbacks.credentials(|_, username, _| {
        debug!("Getting SSH key: {:?}", username);
        git2::Cred::ssh_key(
            username.unwrap(),
            None,
            std::path::Path::new(&format!("{}/.ssh/id_rsa", env::var("HOME").unwrap())),
            None,
        )
    });
    let mut connection = remote.connect_auth(git2::Direction::Push, Some(callbacks), None)?;
    connection.remote().push(&["refs/heads/master"], None)?;

    info!("Changes committed and pushed.");

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
