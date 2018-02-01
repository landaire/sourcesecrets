use std::collections::VecDeque;
use std::process::{Command, Output};
use std::sync::Arc;

#[derive(Clone, Serialize)]
pub enum ChangeType {
    Addition,
    Removal,
    Unknown,
}

#[derive(Clone, Serialize)]
pub struct Commit {
    pub hash: String,
    pub date: String,
    #[serde(skip_serializing)]
    pub client: Option<Arc<GitClient>>,
}

pub struct GitClient {
    pub repo_path: String,
}

impl GitClient {
    pub fn new(repo_path: String) -> GitClient {
        GitClient {
            repo_path: repo_path,
        }
    }

    pub fn get_commits(&self, since_date: Option<&str>, until_date: Option<&str>) -> Vec<Commit> {
        let mut args: Vec<String> = vec![
            "log".to_string(),
            "--format=%H %aI".to_string(),
            "--branches=*".to_string(),
        ];

        if let Some(date) = since_date {
            // could totally do command injection here
            args.push(format!("--since=\"{}\"", date));
        }

        if let Some(date) = until_date {
            args.push(format!("--until=\"{}\"", date));
        }

        let result = self.exec(args.as_slice());

        return str::lines(&String::from_utf8(result.stdout).unwrap())
            .map(|l| {
                let mut parts = l.split_whitespace();
                Commit {
                    hash: parts.next().unwrap().to_string(),
                    date: parts.next().unwrap().to_string(),
                    client: None,
                }
            })
            .collect::<Vec<Commit>>();
    }

    pub fn get_commit_content(&self, commit: &Commit) -> String {
        let args = vec![
            "diff".to_string(),
            "-U0".to_string(),
            format!("{}^!", commit.hash),
        ];
        String::from_utf8_lossy(&self.exec(&args).stdout).into_owned()
    }

    pub fn get_file_at_commit(&self, commit: &str, filename: Option<&String>) -> Vec<u8> {
        let commit = match filename {
            Some(path) => format!("{}:{}", commit.to_string(), path),
            None => format!("{}", commit.to_string()),
        };
        let args = vec!["show".to_string(), commit];
        let output = self.exec(&args);
        // if output.stderr.len() != 0 {
        //     eprintln!("{}", String::from_utf8_lossy(&output.stderr));
        // }

        output.stdout
    }

    pub fn get_file_names_for_commit(&self, commit: &Commit) -> VecDeque<String> {
        let args = vec![
            "diff".to_string(),
            "--name-only".to_string(),
            commit.hash.clone(),
        ];
        let output = self.exec(&args);
        str::lines(&String::from_utf8_lossy(&output.stdout).into_owned())
            .map(|line| line.to_string())
            .collect()
    }

    fn exec(&self, args: &[String]) -> Output {
        Command::new("git")
            .args(args)
            .current_dir(&self.repo_path)
            .output()
            .expect("failed to execute git")
    }
}
