# Autocommit

Autocommit is a tool to automatically create commits of a repo every X minutes. If you have an OpenAI key (env var as `OPENAI_API_KEY`) the commit diff (upto 1500 characters) is passed to gpt-3.5-turbo to summarize the commit to create a meaningful commit message. Otherwise, the current datetime is used as the commit message.

Autocommit has 4 commands:

```
Usage: autocommit <COMMAND>

Commands:
  run --path {PATH_TO_GIT_REPO}
  create --frequency {FREQUENCY_IN_MINUTES} --path {PATH_TO_GIT_REPO}
  list    # List currently configured autocommits
  delete --path {PATH_TO_GIT_REPO}
```

The general usage pattern will just be to use `create` to set up autocommit on a particular Git repo, `list` to see what is currently setup and `delete` to stop autocommitting. `run` can be used to test `autocommit` behavior in a one-off fashion, and it's also what the cronjob is configured to call. `autocommit` will log to `${REPO_PATH}/.autocommit_log` when it runs at the configured frequency. **Make sure to add .autocommit_log to your .gitignore before setting up a repo, otherwise you will run into an infinite loop where changes to `autocommit` will trigger more commits**.


There are a few rough edges that need to be fixed, although the general structure of the code works (all the commits in this repo have been generated through `autocommit`):
- Make the binary path aware (it's hardcoded to my laptop right now)
- Improve the API key piping story
- Improve robustness to different configurations of git repos. The wrapper library for `libgit` was used, but it might be better to simply dispatch subprocess commands to automatically pick up the right SSH keys, add untracked files etc.
- Improve the logging story so people don't have to remember to add `.autocommit_log` to their `.gitignore`
- Allow other cron jobs to be added
