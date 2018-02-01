extern crate clap;
extern crate csv;
extern crate regex;
extern crate toml;
#[macro_use]
extern crate serde_derive;
extern crate base64;
extern crate pbr;
extern crate serde_json;
#[macro_use(defer)]
extern crate scopeguard;

mod git;

use base64::encode;
use clap::{App, Arg};
use pbr::ProgressBar;
use regex::Regex;
use std::collections::VecDeque;
use std::env::current_exe;
use std::fs::File;
use std::io::prelude::*;
use std::io::{stdout, Write};
use std::iter::FromIterator;
use std::path::Path;
use std::process::exit;
use std::str;
use std::sync::atomic::{AtomicUsize, Ordering, ATOMIC_USIZE_INIT};
use std::sync::{Arc, Mutex, RwLock};
use std::thread;
use std::vec::Vec;

use git::{ChangeType, Commit, GitClient};

const NUM_THREADS: usize = 6;
static mut VERBOSE: bool = false;
static THREAD_DONE_COUNT: AtomicUsize = ATOMIC_USIZE_INIT;

macro_rules! verbose_print(
    ($($arg:tt)*) => { {
		unsafe {
			if VERBOSE {
				let r = writeln!(&mut ::std::io::stdout(), $($arg)*);
				r.expect("failed printing to stdout");
			}
		}
    } }
);

#[derive(Clone, Serialize, PartialEq)]
pub enum MatchType {
    Pattern,
    File,
}

#[derive(Debug, Default, Deserialize)]
struct Config {
    patterns: Option<Vec<Pattern>>,
    filters: Option<Vec<Pattern>>,
    files: Option<Vec<FilePattern>>,
}

#[derive(Debug, Default, Serialize, Deserialize, Clone)]
struct Pattern {
    description: String,
    pattern: String,
    enabled: Option<bool>,
    case_sensitive: Option<bool>,

    #[serde(skip_deserializing, skip_serializing)]
    regex: Option<Regex>,
}

#[derive(Debug, Default, Serialize, Deserialize, Clone)]
struct FilePattern {
    description: String,
    extension: String,
    binary: Option<bool>,
}

#[derive(Clone, Serialize)]
struct PatternMatch {
    description: String,
    text: String,
    repo_path: String,
    file: String,
    full_path: String,
    match_type: MatchType,
    change_type: ChangeType,
    commit_hash: String,
    commit_date: String,
}

fn main() {
    let args = App::new("Source Secrets")
        .version("1.0")
        .author("Lander Brandt <lander@conficker.io>")
        .about("Searches a git repository for secrets")
        .arg(
            Arg::with_name("repos")
                .value_name("GIT_REPO_PATH")
                .help("Sets the path of the git repository")
                .multiple(true)
                .required(true),
        ).arg(
            Arg::with_name("definitions")
                .short("d")
                .value_name("definitions.toml")
                .help("File containing pattern definitions")
                .takes_value(true),
        ).arg(
            Arg::with_name("output_file")
                .short("o")
                .value_name("OUTPUT_FILE")
                .help("File to output data to write results to (use - for stdout)")
                .takes_value(true)
                .required(true),
        ).arg(
            Arg::with_name("since")
                .short("s")
                .value_name("DATE")
                .help("Look at commits since this date (e.g. \"Jan 1, 2018\" or \"2 weeks ago\")")
                .takes_value(true),
        ).arg(
            Arg::with_name("until")
                .short("u")
                .value_name("DATE")
                .help("Look at commits before this date (e.g. \"Jan 1, 2018\" or \"2 weeks ago\")")
                .takes_value(true),
        ).arg(
            Arg::with_name("verbose")
                .short("v")
                .value_name("VERBOSE")
                .help("Set verbose output (shows results as they come in)")
                .takes_value(false),
        ).get_matches();
    unsafe {
        VERBOSE = args.is_present("verbose");
    }

    let repos = args.values_of_lossy("repos").unwrap();

    let output_file = match args.value_of("output_file").unwrap() {
        "-" => Box::new(stdout()) as Box<Write>,
        filename => {
            Box::new(File::create(filename).expect("Unable to create output file")) as Box<Write>
        }
    };

    let definitions_path = match args.value_of("definitions") {
        Some(p) => p.to_owned(),
        None => {
            let mut p = current_exe()
                .unwrap()
                .parent()
                .unwrap()
                .join("definitions.toml");
            p.to_str().unwrap().to_owned()
        }
    };

    let mut definitions_file = File::open(definitions_path).expect("definitions.toml not found");
    let mut config_contents = String::new();
    definitions_file
        .read_to_string(&mut config_contents)
        .expect("error while reading definitions file");

    let pattern_config = toml::from_str(&config_contents);

    if let Err(err) = pattern_config {
        eprintln!("Error parsing config: {:?}", err);
        exit(1);
    }

    let pattern_config: Config = pattern_config.unwrap();
    let mut patterns = pattern_config.patterns.unwrap();
    let mut filters: Option<Vec<Pattern>> = pattern_config.filters;
    let files = pattern_config.files.unwrap();

    // loop over all of the patterns to compile their regexes
    for pattern in &mut patterns {
        if !pattern.enabled.unwrap_or(true) {
            continue;
        }

        if !pattern.case_sensitive.unwrap_or(false) {
            pattern.pattern = "(?i)".to_owned() + &pattern.pattern;
        }

        pattern.regex = match Regex::new(&pattern.pattern) {
            Ok(r) => Some(r),
            Err(e) => {
                eprintln!(
                    "Could not compile pattern {}: {}",
                    pattern.description.clone(),
                    e
                );
                None
            }
        };
    }

    // loop over all of the patterns to compile their regexes
    if filters.is_some() {
        let mut filters = filters.as_mut().unwrap();
        for filter in &mut filters.into_iter() {
            if !filter.enabled.unwrap_or(true) {
                continue;
            }

            if !filter.case_sensitive.unwrap_or(false) {
                filter.pattern = "(?i)".to_owned() + &filter.pattern;
            }

            filter.regex = match Regex::new(&filter.pattern) {
                Ok(r) => Some(r),
                Err(e) => {
                    eprintln!(
                        "Could not compile filter {}: {}",
                        filter.description.clone(),
                        e
                    );
                    None
                }
            };
        }
    }

    let mut all_commits = Vec::new();
    let mut clients = Vec::new();

    // ensure all of the repos exist
    for repo in &repos {
        let repo_path = Path::new(&repo);
        // not being pedantic and checking if .git path is a folder here
        // if a .git file exists in a folder I want to see how this thing blows up
        // TODO: add test for .git file, not folder, existing in repo path
        if !repo_path.exists() || !repo_path.join(".git").exists() {
            eprintln!("Repo path {} does not exist", repo);
            continue;
        }
        println!("Getting data for repo {}", repo);

        let client = Arc::new(GitClient::new(repo.to_string()));

        let mut commits = client
            .clone()
            .get_commits(args.value_of("since"), args.value_of("until"));
        all_commits.reserve(commits.len());

        for mut commit in commits {
            commit.client = Some(client.clone());
            all_commits.push(commit);
        }

        clients.push(client);
    }

    let mut threads = Vec::new();
    // set up the progress bar for all threads + commits
    let pb = Arc::new(Mutex::new(ProgressBar::new(
        (all_commits.len() + NUM_THREADS as usize) as u64,
    )));
    let found_matches = Arc::new(RwLock::new(VecDeque::new() as VecDeque<PatternMatch>));

    if all_commits.len() == 0 {
        println!("No commits found to search");
        return;
    }

    let commits_per_thread = all_commits.len() / NUM_THREADS;
    let last_thread_commit_count = commits_per_thread + (all_commits.len() % NUM_THREADS);
    for i in 0..NUM_THREADS {
        let mut num_commits = commits_per_thread;
        if i == NUM_THREADS - 1 {
            num_commits = last_thread_commit_count;
        }

        let commits: VecDeque<Commit> = VecDeque::from_iter(all_commits.drain(0..num_commits));
        let patterns = patterns.clone();
        let found_matches = found_matches.clone();
        let pb = pb.clone();
        let files = files.clone();

        threads.push(thread::spawn(move || {
            pattern_matcher_thread(
                commits,
                patterns,
                files,
                pb,
                move |matched: PatternMatch| {
                    match matched.match_type {
                        MatchType::Pattern => {
                            verbose_print!("\r\n{}: {}", matched.file, matched.text)
                        }
                        MatchType::File => verbose_print!("\r\n{}", matched.file),
                    }

                    found_matches.write().unwrap().push_back(matched);
                },
            )
        }));
    }
    // this should be empty here -- let's explicitly get rid of this resource
    drop(all_commits);

    let mut csv_writer = csv::Writer::from_writer(output_file);
    let found_matches = found_matches.clone();
    while found_matches.read().unwrap().len() > 0
        || THREAD_DONE_COUNT.load(Ordering::Relaxed) != NUM_THREADS
    {
        let mut matches = found_matches.write().unwrap();
        'outer: loop {
            match matches.pop_front() {
                Some(pattern_match) => {
                    if pattern_match.match_type == MatchType::Pattern && filters.as_ref().is_some()
                    {
                        let filters = filters.as_ref().unwrap();
                        for &ref filter in filters {
                            if filter.regex.as_ref().unwrap().is_match(&pattern_match.text) {
                                continue 'outer;
                            }
                        }
                    }
                    csv_writer
                        .serialize(pattern_match)
                        .expect("failed to serialize pattern");
                }
                None => {
                    csv_writer.flush().unwrap();
                    break;
                }
            }
        }
    }

    for thread in threads {
        if let Err(err) = thread.join() {
            eprintln!("Error joining thread: {:?}", err);
        }
    }
}

fn pattern_matcher_thread<F, T>(
    mut commits: VecDeque<Commit>,
    patterns: Vec<Pattern>,
    files: Vec<FilePattern>,
    pb: Arc<Mutex<ProgressBar<T>>>,
    on_found: F,
) where
    F: Fn(PatternMatch),
    T: Write,
{
    defer!({
        // this one is faked because otherwise the user gets a false impression that all work is done
        let mut pb = pb.lock().unwrap();
        pb.inc();
        drop(pb);

        THREAD_DONE_COUNT.fetch_add(1, Ordering::SeqCst);
    });

    let mut in_file = false;
    let mut file_info: Option<FilePattern> = None;
    let mut file_name: Option<String> = None;
    let mut file_index: Option<String> = None;

    loop {
        let commit = commits.pop_front();

        match commit {
            Some(commit) => {
                let client = commit.client.as_ref().unwrap();
                let mut pb = pb.lock().unwrap();
                pb.inc();
                drop(pb);

                let content = client.get_commit_content(&commit);
                in_file = false;
                file_info = None;
                file_name = None;
                file_index = None;

                'outer: for line in str::lines(&content) {
                    let line = line.trim();

                    // ignore @@ lines since that just tells you the line range
                    // and that we're on a new file boundary
                    if line.starts_with("diff --git") {
                        if in_file && file_index.is_some() {
                            // we're in a file that we have a pattern for -- we need to get its
                            // contents now
                            let mut file_data =
                                client.get_file_at_commit(file_index.as_ref().unwrap(), None);
                            if file_data.len() == 0 {
                                file_data = client.get_file_at_commit(
                                    &commit.hash,
                                    Some(file_name.as_ref().unwrap()),
                                );
                            }

                            let file_data_string: String =
                                match file_info.as_ref().unwrap().binary.unwrap_or(false) {
                                    // if it's a binary file we need to encode as base64
                                    true => encode(&file_data.as_slice()),
                                    false => String::from_utf8_lossy(&file_data).into_owned(),
                                };

                            let fname = file_name.as_ref().unwrap().clone();
                            let matched = PatternMatch {
                                description: file_info.as_ref().unwrap().description.clone(),
                                text: file_data_string,
                                match_type: MatchType::File,
                                repo_path: client.repo_path.clone(),
                                full_path: Path::new(&client.repo_path)
                                    .join(&fname)
                                    .into_os_string()
                                    .into_string()
                                    .unwrap(),
                                file: fname,
                                change_type: ChangeType::Unknown,
                                commit_hash: commit.hash.clone(),
                                commit_date: commit.date.clone(),
                            };
                            on_found(matched);

                            in_file = false;
                        }

                        for file in &files {
                            // it does, so now let's parse it out
                            // NOTE: this could easily be broken by paths with spaces...
                            // we're going to assume that the repos do not contain any folder
                            // ending with " b/"

                            // 11 is the length of "git --diff a/"
                            file_name = Some(line.chars().skip(13).collect());
                            file_name =
                                Some(file_name.unwrap().split(" b/").next().unwrap().to_string());

                            // just check if the line contains the extension first
                            // there will definitely be false positives since we aren't making
                            // another allocation for the period
                            if line.contains(&file.extension) {
                                if file_name.as_ref().unwrap().ends_with(&file.extension) {
                                    in_file = true;
                                    file_info = Some(file.clone());
                                    continue 'outer;
                                }
                            }
                        }

                        file_index = None;
                        continue;
                    }

                    if in_file {
                        if line.starts_with("index ") {
                            file_index = Some(line.split("..").skip(1).take(1).collect());
                        }

                        continue;
                    }

                    // arbitrary length
                    if line.len() > 5000 {
                        verbose_print!("Skipping line -- too long");
                        continue;
                    }

                    check_patterns(
                        &patterns,
                        &line,
                        &on_found,
                        &client.repo_path,
                        file_name.as_ref().unwrap(),
                        &commit,
                    );
                }
            }
            _ => break,
        }
    }
}

fn check_patterns<F>(
    patterns: &Vec<Pattern>,
    line: &str,
    on_found: &F,
    repo_path: &String,
    file_name: &String,
    commit: &Commit,
) where
    F: Fn(PatternMatch),
{
    for &ref pattern in patterns {
        if !pattern.enabled.unwrap_or(true) {
            continue;
        }

        if pattern.regex.as_ref().unwrap().is_match(&line) {
            let mat = pattern.regex.as_ref().unwrap().find(&line).unwrap();
            let matched_string: String = line
                .chars()
                .skip(mat.start() - 1)
                .take(mat.end() - 1)
                .collect();

            let change_type = match line.chars().next().unwrap() {
                '+' => ChangeType::Addition,
                '-' => ChangeType::Removal,
                '@' => {
                    continue;
                }
                unknown_type => {
                    eprintln!("Unexpected value for change type: {:?}", unknown_type);
                    continue;
                }
            };

            let matched_text: String = matched_string.chars().skip(1).collect();
            let matched = PatternMatch {
                description: pattern.description.clone(),
                text: matched_text.trim().to_owned(),
                match_type: MatchType::Pattern,
                repo_path: repo_path.clone(),
                file: file_name.to_owned(),
                full_path: Path::new(&repo_path)
                    .join(file_name)
                    .into_os_string()
                    .into_string()
                    .unwrap(),
                change_type: change_type,
                commit_hash: commit.hash.clone(),
                commit_date: commit.date.clone(),
            };

            on_found(matched);
        }
    }
}
