# sourcesecrets

A tool for finding patterns of text in Git history

## Rationale

Repository maintainers sometimes do not fully consider the impact of the "secret" they are redacting or may not realize they have committed the removal of a secret from source code without fully redacting it or mitigating the issue. `sourcesecrets` allows you to provide a file extension or regular expression to find in every single commit ever made to a repository to find secrets left in source code history.

## Usage

Example 1:

```
sourcesecrets -o secrets.csv repos/*/
```

Example 2:

```
sourcesecrets -o secrets.csv -d definitions.private.toml repo_path
```

## Improvements to be made

1. The `git` utility is invoked for *every* commit to get contents and other details. Using some `libgit2` bindings or another library may provide benefits over the overhead of invoking a new process for every commit.
