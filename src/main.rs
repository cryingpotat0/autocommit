use clap::{Parser, Subcommand};
use color_eyre::{eyre::eyre, Report, Result};
use git2::{DiffOptions, Repository, StatusOptions};
use std::env;
use tracing::{debug, info};
use tracing_subscriber::EnvFilter;

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
        frequency: u8,
    },
    /// List currently configured autocommits.
    List,
    Delete {
        /// Path of autocommit repo to delete.
        path: std::path::PathBuf,
    },
}

fn main() -> Result<()> {
    setup()?;
    let cli = Cli::parse();

    match &cli.command {
        Commands::Run { path } => {
            debug!("Running {}", path.display());
            run(path.to_path_buf())?;
        }
        Commands::Create { path, frequency } => {
            debug!("Creating {} with frequency {}", path.display(), frequency);
        }
        Commands::List => {
            debug!("Listing");
        }
        Commands::Delete { path } => {
            debug!("Deleting {}", path.display());
        }
    }
    Ok(())
}

fn run(repo_path: std::path::PathBuf) -> Result<()> {
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
    let diff = repo.diff_index_to_workdir(None, Some(&mut diff_opts))?;

    let diff_stats = diff.stats()?;
    let mut diff_string = if diff_stats.files_changed() == 0 {
        String::new()
    } else {
        let mut val = String::new();
        diff.print(git2::DiffFormat::Patch, |_, _, line| {
            match line.origin() {
                '+' | '-' | ' ' => print!("{}", line.origin()),
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
        Ok(api_key) => generate_commit_message(api_key, &diff_string)?,
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
    callbacks.credentials(|_, username, allowed| {
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

fn generate_commit_message(api_key: String, diff_string: &str) -> Result<String> {
    // hehehe
    let prompt = format!("You are CommitBot, an assistant tasked with writing helpful commit messages based on code changes.
      You will be given a set of patches of code changes, and you must write a short commit message describing the changes. Do not be verbose. 
      Your response must include only high level logical changes if the diff is large, otherwise you may include specific changes.
      Try to fit your response in one line.
      \n\n{}", diff_string);
    let response = ureq::post("https://api.openai.com/v1/chat/completions")
        .set("Authorization", format!("Bearer {}", api_key).as_str())
        .set("Content-Type", "application/json")
        .send_json(ureq::json!({
            "model": "gpt-3.5-turbo",
            "messages": [{
              "role": "user",
              "content": prompt,
            }],
            "max_tokens": 1000,
            "n": 1,
            "temperature": 0.1,
        }));
    let response_json: serde_json::Value = if let Ok(res) = response {
        res.into_json()?
    } else {
        debug!("Response error: {:?}", response);
        return Err(eyre!("Failed to generate commit message"));
    };

    debug!("Response: {:?}", response_json);

    let commit_message = response_json["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("Generated commit message not available")
        .trim()
        .to_string();

    Ok(commit_message)
}
